use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::AppPaths;
use crate::error::{RebeccaError, Result};
use crate::scan::{MeasuredScan, ScanBackendKind, ScanReport};

use super::{
    SCAN_CACHE_DIR, SCAN_CACHE_PRUNE_BATCH_LIMIT, ScanCacheLookup, ScanCacheMiss,
    ScanCachePathSnapshot, ScanCachePolicy, ScanCachePruneReport, ScanCacheRecord, ScanCacheStore,
    ScanCacheUsnValidation, ScanCacheUsnValidator, ScanCacheWriteDurability,
};

#[cfg(test)]
static STRICT_SYNC_CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

impl ScanCacheStore {
    pub fn from_app_paths(paths: &AppPaths) -> Self {
        Self::new(paths.cache_dir.join(SCAN_CACHE_DIR))
    }

    pub fn new(root_dir: impl Into<PathBuf>) -> Self {
        Self {
            root_dir: root_dir.into(),
        }
    }

    pub fn root_dir(&self) -> &Path {
        &self.root_dir
    }

    pub fn cache_file_for(&self, root: &Path) -> PathBuf {
        self.root_dir.join(format!("{:016x}.json", path_hash(root)))
    }

    pub fn load(&self, root: &Path) -> ScanCacheLookup {
        self.load_with_policy(root, ScanCachePolicy::default())
    }

    pub fn prune(&self) -> ScanCachePruneReport {
        self.prune_with_policy(ScanCachePolicy::default())
    }

    pub fn prune_with_policy(&self, policy: ScanCachePolicy) -> ScanCachePruneReport {
        let mut report = ScanCachePruneReport::default();

        let entries = match std::fs::read_dir(&self.root_dir) {
            Ok(entries) => entries,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return report;
            }
            Err(err) => {
                tracing::debug!(
                    path = %self.root_dir.display(),
                    error = %err,
                    "scan cache prune skipped"
                );
                return report;
            }
        };

        for entry in entries.take(SCAN_CACHE_PRUNE_BATCH_LIMIT) {
            let entry = match entry {
                Ok(entry) => entry,
                Err(err) => {
                    tracing::debug!(
                        path = %self.root_dir.display(),
                        error = %err,
                        "scan cache prune skipped"
                    );
                    continue;
                }
            };

            let cache_file = entry.path();
            let is_json_cache = cache_file
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("json"));
            if !is_json_cache {
                continue;
            }

            report.inspected = report.inspected.saturating_add(1);

            let raw = match std::fs::read_to_string(&cache_file) {
                Ok(raw) => raw,
                Err(err) => {
                    prune_cache_file(&cache_file);
                    report.pruned = report.pruned.saturating_add(1);
                    tracing::debug!(
                        path = %cache_file.display(),
                        error = %err,
                        "scan cache prune removed unreadable record"
                    );
                    continue;
                }
            };

            let record: ScanCacheRecord = match serde_json::from_str(&raw) {
                Ok(record) => record,
                Err(err) => {
                    prune_cache_file(&cache_file);
                    report.pruned = report.pruned.saturating_add(1);
                    tracing::debug!(
                        path = %cache_file.display(),
                        error = %err,
                        "scan cache prune removed corrupted record"
                    );
                    continue;
                }
            };

            if self.cache_file_for(&record.root) != cache_file {
                prune_cache_file(&cache_file);
                report.pruned = report.pruned.saturating_add(1);
                continue;
            }

            let snapshot = match ScanCachePathSnapshot::read_from_path(&record.root) {
                Ok(snapshot) => snapshot,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                    prune_cache_file(&cache_file);
                    report.pruned = report.pruned.saturating_add(1);
                    continue;
                }
                Err(_) => {
                    report.retained = report.retained.saturating_add(1);
                    continue;
                }
            };

            match record.miss_reason(&record.root, &snapshot, policy, unix_now()) {
                Some(reason) if reason.should_prune_cache_file() => {
                    prune_cache_file(&cache_file);
                    report.pruned = report.pruned.saturating_add(1);
                }
                Some(_) => {
                    report.retained = report.retained.saturating_add(1);
                }
                None => {
                    report.retained = report.retained.saturating_add(1);
                }
            }
        }

        report
    }

    pub fn load_with_policy(&self, root: &Path, policy: ScanCachePolicy) -> ScanCacheLookup {
        self.load_with_policy_and_optional_usn_validator(root, policy, None)
    }

    pub fn load_with_policy_and_usn_validator(
        &self,
        root: &Path,
        policy: ScanCachePolicy,
        usn_validator: &dyn ScanCacheUsnValidator,
    ) -> ScanCacheLookup {
        self.load_with_policy_and_optional_usn_validator(root, policy, Some(usn_validator))
    }

    fn load_with_policy_and_optional_usn_validator(
        &self,
        root: &Path,
        policy: ScanCachePolicy,
        usn_validator: Option<&dyn ScanCacheUsnValidator>,
    ) -> ScanCacheLookup {
        let cache_file = self.cache_file_for(root);
        let snapshot = match ScanCachePathSnapshot::read_from_path(root) {
            Ok(snapshot) => snapshot,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                prune_cache_file(&cache_file);
                return ScanCacheLookup::pruned_miss(ScanCacheMiss::Missing);
            }
            Err(_) => return ScanCacheLookup::miss(ScanCacheMiss::MetadataUnavailable),
        };
        let raw = match std::fs::read_to_string(&cache_file) {
            Ok(raw) => raw,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return ScanCacheLookup::miss(ScanCacheMiss::Missing);
            }
            Err(_) => {
                prune_cache_file(&cache_file);
                return ScanCacheLookup::pruned_miss(ScanCacheMiss::Corrupted);
            }
        };
        let record: ScanCacheRecord = match serde_json::from_str(&raw) {
            Ok(record) => record,
            Err(_) => {
                prune_cache_file(&cache_file);
                return ScanCacheLookup::pruned_miss(ScanCacheMiss::Corrupted);
            }
        };

        match record.miss_reason(root, &snapshot, policy, unix_now()) {
            Some(reason) => {
                if reason.should_prune_cache_file() {
                    prune_cache_file(&cache_file);
                    return ScanCacheLookup::pruned_miss(reason);
                }
                ScanCacheLookup::miss(reason)
            }
            None => {
                if let Some(usn_validator) = usn_validator {
                    if let ScanCacheUsnValidation::Invalidated(reason) =
                        usn_validator.validate_record(&record)
                    {
                        let reason = ScanCacheMiss::from(reason);
                        prune_cache_file(&cache_file);
                        return ScanCacheLookup::pruned_miss(reason);
                    }
                }

                ScanCacheLookup::Hit(record.report)
            }
        }
    }

    pub fn store(&self, root: &Path, report: ScanReport) -> Result<ScanCacheRecord> {
        self.store_measured_scan(
            root,
            MeasuredScan::exact(report, ScanBackendKind::PortableRecursive),
        )
    }

    pub fn store_measured_scan(
        &self,
        root: &Path,
        measured_scan: MeasuredScan,
    ) -> Result<ScanCacheRecord> {
        self.store_measured_scan_with_policy(root, measured_scan, ScanCachePolicy::default())
    }

    pub fn store_measured_scan_with_policy(
        &self,
        root: &Path,
        measured_scan: MeasuredScan,
        policy: ScanCachePolicy,
    ) -> Result<ScanCacheRecord> {
        let snapshot = ScanCachePathSnapshot::from_path(root)?;
        let record = ScanCacheRecord::new(root.to_path_buf(), snapshot, measured_scan);
        let cache_file = self.cache_file_for(root);
        let parent = cache_file.parent().ok_or_else(|| {
            RebeccaError::ScanCacheUnavailable(format!(
                "scan cache path has no parent: {}",
                cache_file.display()
            ))
        })?;
        std::fs::create_dir_all(parent).map_err(|err| {
            RebeccaError::ScanCacheUnavailable(format!(
                "scan cache directory unavailable at {}: {}",
                parent.display(),
                err
            ))
        })?;
        let raw = serde_json::to_vec(&record)?;
        write_cache_file(&cache_file, &raw, policy.write_durability())?;

        Ok(record)
    }
}

fn write_cache_file(
    cache_file: &Path,
    raw: &[u8],
    durability: ScanCacheWriteDurability,
) -> Result<()> {
    let temp_file = temp_cache_file(cache_file);
    let write_result = (|| -> std::io::Result<()> {
        let mut file = std::fs::File::create(&temp_file)?;
        file.write_all(raw)?;
        if durability == ScanCacheWriteDurability::Strict {
            sync_cache_file(&file)?;
        }
        replace_file(&temp_file, cache_file)
    })();

    if let Err(err) = write_result {
        let _ = std::fs::remove_file(&temp_file);
        return Err(RebeccaError::ScanCacheUnavailable(format!(
            "scan cache write failed at {}: {}",
            cache_file.display(),
            err
        )));
    }

    Ok(())
}

fn sync_cache_file(file: &std::fs::File) -> std::io::Result<()> {
    #[cfg(test)]
    STRICT_SYNC_CALLS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

    file.sync_all()
}

fn prune_cache_file(cache_file: &Path) {
    if let Err(err) = std::fs::remove_file(cache_file)
        && err.kind() != std::io::ErrorKind::NotFound
    {
        tracing::debug!(
            path = %cache_file.display(),
            error = %err,
            "scan cache prune skipped"
        );
    }
}

fn replace_file(temp_file: &Path, cache_file: &Path) -> std::io::Result<()> {
    match std::fs::rename(temp_file, cache_file) {
        Ok(()) => Ok(()),
        Err(_) if cache_file.exists() => {
            std::fs::remove_file(cache_file)?;
            std::fs::rename(temp_file, cache_file)
        }
        Err(err) => Err(err),
    }
}

fn temp_cache_file(cache_file: &Path) -> PathBuf {
    let file_name = cache_file
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_else(|| "scan-cache.json".into());
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();

    cache_file.with_file_name(format!("{file_name}.tmp-{}-{unique}", std::process::id()))
}

fn path_hash(path: &Path) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in comparable_path(path).as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }

    hash
}

fn comparable_path(path: &Path) -> String {
    let value = path.to_string_lossy();
    if cfg!(windows) {
        value.to_ascii_lowercase()
    } else {
        value.into_owned()
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{STRICT_SYNC_CALLS, path_hash, unix_now};
    use crate::scan::{ScanBackendKind, ScanEstimateConfidence, ScanReport};
    use crate::scan_cache::{
        DEFAULT_DIRECTORY_SCAN_CACHE_MAX_AGE_SECONDS, SCAN_CACHE_VERSION, ScanCacheFileType,
        ScanCacheLookup, ScanCacheMiss, ScanCachePolicy, ScanCacheStore, ScanCacheUsnChange,
        ScanCacheUsnCheckpoint, ScanCacheUsnInvalidationReason, ScanCacheUsnJournalState,
        ScanCacheUsnValidation, ScanCacheUsnValidator, ScanCacheWriteDurability,
    };
    use std::sync::atomic::Ordering;

    struct StaticUsnValidator {
        validation: ScanCacheUsnValidation,
    }

    impl StaticUsnValidator {
        fn new(validation: ScanCacheUsnValidation) -> Self {
            Self { validation }
        }
    }

    impl ScanCacheUsnValidator for StaticUsnValidator {
        fn validate_record(
            &self,
            _record: &crate::scan_cache::ScanCacheRecord,
        ) -> ScanCacheUsnValidation {
            self.validation
        }
    }

    #[test]
    fn scan_cache_round_trips_current_fingerprint() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("target");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("file.bin"), b"abc").unwrap();
        let store = ScanCacheStore::new(temp.path().join("cache").join("scan"));
        let report = ScanReport {
            bytes_scanned: 3,
            files_scanned: 1,
            directories_scanned: 1,
        };

        let record = store.store(&root, report).unwrap();
        let lookup = store.load(&root);

        assert_eq!(record.version, SCAN_CACHE_VERSION);
        assert_eq!(record.root, root);
        assert_eq!(record.backend, ScanBackendKind::PortableRecursive);
        assert_eq!(record.confidence, ScanEstimateConfidence::Exact);
        assert!(store.cache_file_for(&record.root).exists());
        assert_eq!(lookup, ScanCacheLookup::Hit(report));
    }

    #[test]
    fn scan_cache_round_trips_exact_native_backend_records() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("target");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("file.bin"), b"abc").unwrap();
        let store = ScanCacheStore::new(temp.path().join("cache").join("scan"));
        let report = ScanReport {
            bytes_scanned: 3,
            files_scanned: 1,
            directories_scanned: 1,
        };

        let record = store
            .store_measured_scan(
                &root,
                crate::scan::MeasuredScan::exact(report, ScanBackendKind::WindowsNative),
            )
            .unwrap();
        let lookup = store.load(&root);

        assert_eq!(record.backend, ScanBackendKind::WindowsNative);
        assert_eq!(lookup, ScanCacheLookup::Hit(report));
    }

    #[test]
    fn scan_cache_writes_compact_v1_records_with_identity_metadata() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("target.txt");
        std::fs::write(&root, b"abc").unwrap();
        let store = ScanCacheStore::new(temp.path().join("cache").join("scan"));
        let report = ScanReport {
            bytes_scanned: 3,
            files_scanned: 1,
            directories_scanned: 0,
        };

        let record = store.store(&root, report).unwrap();
        let raw = std::fs::read_to_string(store.cache_file_for(&root)).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap();

        assert_eq!(record.version, SCAN_CACHE_VERSION);
        assert!(!raw.contains('\n'), "cache records should be compact JSON");
        assert_eq!(parsed["version"], SCAN_CACHE_VERSION);
        assert_eq!(parsed["backend"], "portable-recursive");
        assert_eq!(parsed["confidence"], "exact");
        assert_eq!(parsed["identity"].as_object().unwrap().len(), 2);
        assert!(parsed["identity"].get("usn_checkpoint").is_none());
    }

    #[test]
    fn scan_cache_usn_journal_range_accepts_clean_range() {
        let checkpoint = ScanCacheUsnCheckpoint {
            journal_id: 7,
            next_usn: 100,
        };
        let state = ScanCacheUsnJournalState {
            journal_id: 7,
            first_usn: 1,
            next_usn: 150,
        };
        let changes = [ScanCacheUsnChange::new(11, Some(10), 120)];

        assert_eq!(
            checkpoint.validate_journal_range(Some(42), &state, &changes),
            ScanCacheUsnValidation::Clean
        );
    }

    #[test]
    fn scan_cache_usn_journal_id_mismatch_invalidates() {
        let checkpoint = ScanCacheUsnCheckpoint {
            journal_id: 7,
            next_usn: 100,
        };
        let state = ScanCacheUsnJournalState {
            journal_id: 8,
            first_usn: 1,
            next_usn: 150,
        };

        assert_eq!(
            checkpoint.validate_journal_range(Some(42), &state, &[]),
            ScanCacheUsnValidation::Invalidated(ScanCacheUsnInvalidationReason::JournalChanged)
        );
    }

    #[test]
    fn scan_cache_usn_unreadable_range_invalidates_conservatively() {
        let checkpoint = ScanCacheUsnCheckpoint {
            journal_id: 7,
            next_usn: 100,
        };
        let state = ScanCacheUsnJournalState {
            journal_id: 7,
            first_usn: 120,
            next_usn: 150,
        };

        assert_eq!(
            checkpoint.validate_journal_range(Some(42), &state, &[]),
            ScanCacheUsnValidation::Invalidated(ScanCacheUsnInvalidationReason::RangeUnavailable)
        );
    }

    #[test]
    fn scan_cache_usn_target_subtree_change_invalidates() {
        let checkpoint = ScanCacheUsnCheckpoint {
            journal_id: 7,
            next_usn: 100,
        };
        let state = ScanCacheUsnJournalState {
            journal_id: 7,
            first_usn: 1,
            next_usn: 150,
        };
        let changes =
            [ScanCacheUsnChange::new(11, Some(10), 120).with_ancestor_file_ids(vec![10, 42])];

        assert_eq!(
            checkpoint.validate_journal_range(Some(42), &state, &changes),
            ScanCacheUsnValidation::Invalidated(ScanCacheUsnInvalidationReason::TargetChanged)
        );
    }

    #[test]
    fn scan_cache_missing_usn_support_falls_back_to_normal_hit() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("target.txt");
        std::fs::write(&root, b"abc").unwrap();
        let store = ScanCacheStore::new(temp.path().join("cache").join("scan"));
        let report = ScanReport {
            bytes_scanned: 3,
            files_scanned: 1,
            directories_scanned: 0,
        };
        store.store(&root, report).unwrap();
        let validator = StaticUsnValidator::new(ScanCacheUsnValidation::Unsupported);

        let lookup =
            store.load_with_policy_and_usn_validator(&root, ScanCachePolicy::default(), &validator);

        assert_eq!(lookup, ScanCacheLookup::Hit(report));
    }

    #[test]
    fn scan_cache_usn_invalidated_lookup_prunes_record() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("target.txt");
        std::fs::write(&root, b"abc").unwrap();
        let store = ScanCacheStore::new(temp.path().join("cache").join("scan"));
        let report = ScanReport {
            bytes_scanned: 3,
            files_scanned: 1,
            directories_scanned: 0,
        };
        store.store(&root, report).unwrap();
        let validator = StaticUsnValidator::new(ScanCacheUsnValidation::Invalidated(
            ScanCacheUsnInvalidationReason::TargetChanged,
        ));

        let lookup =
            store.load_with_policy_and_usn_validator(&root, ScanCachePolicy::default(), &validator);

        assert_eq!(
            lookup,
            ScanCacheLookup::pruned_miss(ScanCacheMiss::UsnTargetChanged)
        );
        assert!(!store.cache_file_for(&root).exists());
    }

    #[test]
    fn scan_cache_default_write_durability_skips_strict_sync() {
        STRICT_SYNC_CALLS.store(0, Ordering::SeqCst);
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("target.txt");
        std::fs::write(&root, b"abc").unwrap();
        let store = ScanCacheStore::new(temp.path().join("cache").join("scan"));
        let report = ScanReport {
            bytes_scanned: 3,
            files_scanned: 1,
            directories_scanned: 0,
        };

        store.store(&root, report).unwrap();

        assert_eq!(STRICT_SYNC_CALLS.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn scan_cache_strict_write_durability_syncs_file() {
        STRICT_SYNC_CALLS.store(0, Ordering::SeqCst);
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("target.txt");
        std::fs::write(&root, b"abc").unwrap();
        let store = ScanCacheStore::new(temp.path().join("cache").join("scan"));
        let report = ScanReport {
            bytes_scanned: 3,
            files_scanned: 1,
            directories_scanned: 0,
        };
        let policy =
            ScanCachePolicy::default().with_write_durability(ScanCacheWriteDurability::Strict);

        store
            .store_measured_scan_with_policy(
                &root,
                crate::scan::MeasuredScan::exact(report, ScanBackendKind::PortableRecursive),
                policy,
            )
            .unwrap();

        assert_eq!(STRICT_SYNC_CALLS.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn scan_cache_store_is_derived_from_app_cache_dir() {
        let temp = tempfile::tempdir().unwrap();
        let paths = crate::config::AppPaths {
            config_dir: temp.path().join("config"),
            config_file: temp.path().join("config").join("config.toml"),
            state_dir: temp.path().join("state"),
            cache_dir: temp.path().join("cache"),
            history_file: temp.path().join("state").join("history.jsonl"),
        };

        let store = ScanCacheStore::from_app_paths(&paths);

        assert_eq!(store.root_dir(), paths.cache_dir.join("scan"));
    }

    #[test]
    fn scan_cache_missing_file_is_cache_miss() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("target");
        std::fs::create_dir_all(&root).unwrap();
        let store = ScanCacheStore::new(temp.path().join("cache").join("scan"));

        let lookup = store.load(&root);

        assert_eq!(lookup, ScanCacheLookup::miss(ScanCacheMiss::Missing));
    }

    #[test]
    fn scan_cache_missing_root_prunes_existing_record_on_load() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("target.txt");
        std::fs::write(&root, b"abc").unwrap();
        let store = ScanCacheStore::new(temp.path().join("cache").join("scan"));
        let report = ScanReport {
            bytes_scanned: 3,
            files_scanned: 1,
            directories_scanned: 0,
        };
        store.store(&root, report).unwrap();
        std::fs::remove_file(&root).unwrap();

        let lookup = store.load(&root);

        assert_eq!(lookup, ScanCacheLookup::pruned_miss(ScanCacheMiss::Missing));
        assert!(!store.cache_file_for(&root).exists());
    }

    #[test]
    fn scan_cache_corrupted_file_is_cache_miss() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("target");
        std::fs::create_dir_all(&root).unwrap();
        let store = ScanCacheStore::new(temp.path().join("cache").join("scan"));
        std::fs::create_dir_all(store.root_dir()).unwrap();
        std::fs::write(store.cache_file_for(&root), b"not json").unwrap();

        let lookup = store.load(&root);

        assert_eq!(
            lookup,
            ScanCacheLookup::pruned_miss(ScanCacheMiss::Corrupted)
        );
        assert!(!store.cache_file_for(&root).exists());
    }

    #[test]
    fn scan_cache_future_version_is_stale() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("target");
        std::fs::create_dir_all(&root).unwrap();
        let store = ScanCacheStore::new(temp.path().join("cache").join("scan"));
        let report = ScanReport {
            bytes_scanned: 0,
            files_scanned: 0,
            directories_scanned: 1,
        };
        let mut record = store.store(&root, report).unwrap();
        record.version = SCAN_CACHE_VERSION + 1;
        std::fs::write(
            store.cache_file_for(&root),
            serde_json::to_vec_pretty(&record).unwrap(),
        )
        .unwrap();

        let lookup = store.load(&root);

        assert_eq!(lookup, ScanCacheLookup::pruned_miss(ScanCacheMiss::Stale));
        assert!(!store.cache_file_for(&root).exists());
    }

    #[test]
    fn scan_cache_store_overwrites_existing_record() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("target");
        std::fs::write(&root, b"abc").unwrap();
        let store = ScanCacheStore::new(temp.path().join("cache").join("scan"));
        let first = ScanReport {
            bytes_scanned: 1,
            files_scanned: 1,
            directories_scanned: 0,
        };
        let second = ScanReport {
            bytes_scanned: 3,
            files_scanned: 1,
            directories_scanned: 0,
        };

        store.store(&root, first).unwrap();
        store.store(&root, second).unwrap();

        assert_eq!(store.load(&root), ScanCacheLookup::Hit(second));
    }

    #[test]
    fn scan_cache_metadata_change_is_stale() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("target.txt");
        std::fs::write(&root, b"abc").unwrap();
        let store = ScanCacheStore::new(temp.path().join("cache").join("scan"));
        let report = ScanReport {
            bytes_scanned: 3,
            files_scanned: 1,
            directories_scanned: 0,
        };
        store.store(&root, report).unwrap();
        std::fs::write(&root, b"abcdef").unwrap();

        let lookup = store.load(&root);

        assert_eq!(lookup, ScanCacheLookup::pruned_miss(ScanCacheMiss::Stale));
        assert!(!store.cache_file_for(&root).exists());
    }

    #[test]
    fn scan_cache_directory_record_survives_root_metadata_churn_within_freshness_window() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("target");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("file.bin"), b"abc").unwrap();
        let store = ScanCacheStore::new(temp.path().join("cache").join("scan"));
        let report = ScanReport {
            bytes_scanned: 3,
            files_scanned: 1,
            directories_scanned: 1,
        };
        let mut record = store.store(&root, report).unwrap();
        assert_eq!(record.fingerprint.file_type, ScanCacheFileType::Directory);
        record.fingerprint.len = record.fingerprint.len.saturating_add(1);
        record.fingerprint.modified_unix_seconds = Some(
            record
                .fingerprint
                .modified_unix_seconds
                .unwrap_or_default()
                .saturating_sub(1),
        );
        std::fs::write(
            store.cache_file_for(&root),
            serde_json::to_vec_pretty(&record).unwrap(),
        )
        .unwrap();
        std::fs::write(root.join("new-file.bin"), b"changed").unwrap();

        let lookup = store.load(&root);

        assert_eq!(lookup, ScanCacheLookup::Hit(report));
        assert!(store.cache_file_for(&root).exists());
    }

    #[test]
    fn scan_cache_directory_record_expires_after_freshness_window() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("target");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("file.bin"), b"abc").unwrap();
        let store = ScanCacheStore::new(temp.path().join("cache").join("scan"));
        let policy = ScanCachePolicy::new(1);
        let report = ScanReport {
            bytes_scanned: 3,
            files_scanned: 1,
            directories_scanned: 1,
        };
        let mut record = store.store(&root, report).unwrap();
        record.written_at_unix_seconds =
            unix_now().saturating_sub(policy.directory_record_max_age_seconds() + 1);
        std::fs::write(
            store.cache_file_for(&root),
            serde_json::to_vec_pretty(&record).unwrap(),
        )
        .unwrap();

        let lookup = store.load_with_policy(&root, policy);

        assert_eq!(lookup, ScanCacheLookup::pruned_miss(ScanCacheMiss::Expired));
        assert!(!store.cache_file_for(&root).exists());
    }

    #[test]
    fn scan_cache_file_record_does_not_expire_by_age() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("target.txt");
        std::fs::write(&root, b"abc").unwrap();
        let store = ScanCacheStore::new(temp.path().join("cache").join("scan"));
        let policy = ScanCachePolicy::new(1);
        let report = ScanReport {
            bytes_scanned: 3,
            files_scanned: 1,
            directories_scanned: 0,
        };
        let mut record = store.store(&root, report).unwrap();
        record.written_at_unix_seconds =
            unix_now().saturating_sub(DEFAULT_DIRECTORY_SCAN_CACHE_MAX_AGE_SECONDS + 1);
        std::fs::write(
            store.cache_file_for(&root),
            serde_json::to_vec_pretty(&record).unwrap(),
        )
        .unwrap();

        let lookup = store.load_with_policy(&root, policy);

        assert_eq!(lookup, ScanCacheLookup::Hit(report));
        assert!(store.cache_file_for(&root).exists());
    }

    #[test]
    fn scan_cache_prune_removes_expired_directory_records() {
        let temp = tempfile::tempdir().unwrap();
        let fresh_root = temp.path().join("fresh");
        let stale_root = temp.path().join("stale");
        std::fs::create_dir_all(&fresh_root).unwrap();
        std::fs::create_dir_all(&stale_root).unwrap();
        std::fs::write(fresh_root.join("file.bin"), b"fresh").unwrap();
        std::fs::write(stale_root.join("file.bin"), b"stale").unwrap();
        let store = ScanCacheStore::new(temp.path().join("cache").join("scan"));
        let policy = ScanCachePolicy::new(1);
        let report = ScanReport {
            bytes_scanned: 5,
            files_scanned: 1,
            directories_scanned: 1,
        };
        store.store(&fresh_root, report).unwrap();
        let mut stale_record = store.store(&stale_root, report).unwrap();
        stale_record.written_at_unix_seconds = 0;
        std::fs::write(
            store.cache_file_for(&stale_root),
            serde_json::to_vec_pretty(&stale_record).unwrap(),
        )
        .unwrap();

        let prune_report = store.prune_with_policy(policy);

        assert_eq!(prune_report.inspected, 2);
        assert_eq!(prune_report.pruned, 1);
        assert_eq!(prune_report.retained, 1);
        assert!(store.cache_file_for(&fresh_root).exists());
        assert!(!store.cache_file_for(&stale_root).exists());
    }

    #[test]
    fn scan_cache_prune_removes_records_for_missing_roots() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("target.txt");
        std::fs::write(&root, b"abc").unwrap();
        let store = ScanCacheStore::new(temp.path().join("cache").join("scan"));
        let report = ScanReport {
            bytes_scanned: 3,
            files_scanned: 1,
            directories_scanned: 0,
        };
        store.store(&root, report).unwrap();
        std::fs::remove_file(&root).unwrap();

        let prune_report = store.prune();

        assert_eq!(prune_report.inspected, 1);
        assert_eq!(prune_report.pruned, 1);
        assert_eq!(prune_report.retained, 0);
        assert!(!store.cache_file_for(&root).exists());
    }

    #[test]
    fn scan_cache_hash_is_case_insensitive_on_windows_only() {
        let left = std::path::Path::new(r"C:\Temp\Cache");
        let right = std::path::Path::new(r"c:\temp\cache");

        if cfg!(windows) {
            assert_eq!(path_hash(left), path_hash(right));
        } else {
            assert_ne!(path_hash(left), path_hash(right));
        }
    }
}

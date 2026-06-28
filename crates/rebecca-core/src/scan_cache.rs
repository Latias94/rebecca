use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::config::AppPaths;
use crate::error::{RebeccaError, Result};
use crate::scan::ScanReport;

pub const SCAN_CACHE_VERSION: u32 = 1;
pub const DEFAULT_DIRECTORY_SCAN_CACHE_MAX_AGE_SECONDS: u64 = 5 * 60;
const SCAN_CACHE_PRUNE_BATCH_LIMIT: usize = 64;
const SCAN_CACHE_DIR: &str = "scan";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScanCachePolicy {
    directory_record_max_age_seconds: u64,
}

impl ScanCachePolicy {
    pub const fn new(directory_record_max_age_seconds: u64) -> Self {
        Self {
            directory_record_max_age_seconds,
        }
    }

    pub const fn directory_record_max_age_seconds(self) -> u64 {
        self.directory_record_max_age_seconds
    }
}

impl Default for ScanCachePolicy {
    fn default() -> Self {
        Self::new(DEFAULT_DIRECTORY_SCAN_CACHE_MAX_AGE_SECONDS)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanCacheRecord {
    pub version: u32,
    pub root: PathBuf,
    pub fingerprint: ScanCacheFingerprint,
    pub report: ScanReport,
    pub written_at_unix_seconds: u64,
}

impl ScanCacheRecord {
    pub fn new(root: PathBuf, fingerprint: ScanCacheFingerprint, report: ScanReport) -> Self {
        Self {
            version: SCAN_CACHE_VERSION,
            root,
            fingerprint,
            report,
            written_at_unix_seconds: unix_now(),
        }
    }

    fn miss_reason(
        &self,
        root: &Path,
        fingerprint: &ScanCacheFingerprint,
        policy: ScanCachePolicy,
        now_unix_seconds: u64,
    ) -> Option<ScanCacheMiss> {
        if self.version != SCAN_CACHE_VERSION
            || self.root != root
            || &self.fingerprint != fingerprint
        {
            return Some(ScanCacheMiss::Stale);
        }

        if self.is_expired_directory_record(policy, now_unix_seconds) {
            return Some(ScanCacheMiss::Expired);
        }

        None
    }

    fn is_expired_directory_record(&self, policy: ScanCachePolicy, now_unix_seconds: u64) -> bool {
        self.fingerprint.file_type == ScanCacheFileType::Directory
            && now_unix_seconds.saturating_sub(self.written_at_unix_seconds)
                > policy.directory_record_max_age_seconds()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanCacheFingerprint {
    pub file_type: ScanCacheFileType,
    pub len: u64,
    pub modified_unix_seconds: Option<u64>,
}

impl ScanCacheFingerprint {
    pub fn from_path(path: &Path) -> Result<Self> {
        let metadata = std::fs::symlink_metadata(path).map_err(|err| {
            RebeccaError::ScanCacheUnavailable(format!(
                "scan cache metadata unavailable for {}: {}",
                path.display(),
                err
            ))
        })?;

        let file_type = metadata.file_type();
        let file_type = if file_type.is_symlink() {
            ScanCacheFileType::Symlink
        } else if metadata.is_file() {
            ScanCacheFileType::File
        } else if metadata.is_dir() {
            ScanCacheFileType::Directory
        } else {
            ScanCacheFileType::Other
        };

        let modified_unix_seconds = metadata
            .modified()
            .ok()
            .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs());

        Ok(Self {
            file_type,
            len: metadata.len(),
            modified_unix_seconds,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScanCacheFileType {
    File,
    Directory,
    Symlink,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanCacheLookup {
    Hit(ScanReport),
    Miss(ScanCacheMiss),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScanCachePruneReport {
    pub inspected: usize,
    pub pruned: usize,
    pub retained: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanCacheMiss {
    Missing,
    Stale,
    Expired,
    Corrupted,
    MetadataUnavailable,
}

impl ScanCacheMiss {
    pub fn label(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Stale => "stale",
            Self::Expired => "expired",
            Self::Corrupted => "corrupted",
            Self::MetadataUnavailable => "metadata-unavailable",
        }
    }

    fn should_prune_cache_file(self) -> bool {
        matches!(self, Self::Stale | Self::Expired | Self::Corrupted)
    }
}

#[derive(Debug, Clone)]
pub struct ScanCacheStore {
    root_dir: PathBuf,
}

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

            let fingerprint = match ScanCacheFingerprint::from_path(&record.root) {
                Ok(fingerprint) => fingerprint,
                Err(_) => {
                    report.retained = report.retained.saturating_add(1);
                    continue;
                }
            };

            match record.miss_reason(&record.root, &fingerprint, policy, unix_now()) {
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
        let fingerprint = match ScanCacheFingerprint::from_path(root) {
            Ok(fingerprint) => fingerprint,
            Err(_) => return ScanCacheLookup::Miss(ScanCacheMiss::MetadataUnavailable),
        };
        let cache_file = self.cache_file_for(root);
        let raw = match std::fs::read_to_string(&cache_file) {
            Ok(raw) => raw,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return ScanCacheLookup::Miss(ScanCacheMiss::Missing);
            }
            Err(_) => {
                prune_cache_file(&cache_file);
                return ScanCacheLookup::Miss(ScanCacheMiss::Corrupted);
            }
        };
        let record: ScanCacheRecord = match serde_json::from_str(&raw) {
            Ok(record) => record,
            Err(_) => {
                prune_cache_file(&cache_file);
                return ScanCacheLookup::Miss(ScanCacheMiss::Corrupted);
            }
        };

        match record.miss_reason(root, &fingerprint, policy, unix_now()) {
            Some(reason) => {
                if reason.should_prune_cache_file() {
                    prune_cache_file(&cache_file);
                }
                ScanCacheLookup::Miss(reason)
            }
            None => ScanCacheLookup::Hit(record.report),
        }
    }

    pub fn store(&self, root: &Path, report: ScanReport) -> Result<ScanCacheRecord> {
        let fingerprint = ScanCacheFingerprint::from_path(root)?;
        let record = ScanCacheRecord::new(root.to_path_buf(), fingerprint, report);
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
        let raw = serde_json::to_vec_pretty(&record)?;
        write_cache_file(&cache_file, &raw)?;

        Ok(record)
    }
}

fn write_cache_file(cache_file: &Path, raw: &[u8]) -> Result<()> {
    let temp_file = temp_cache_file(cache_file);
    let write_result = (|| -> std::io::Result<()> {
        let mut file = std::fs::File::create(&temp_file)?;
        file.write_all(raw)?;
        file.sync_all()?;
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

fn prune_cache_file(cache_file: &Path) {
    if let Err(err) = std::fs::remove_file(cache_file) {
        if err.kind() != std::io::ErrorKind::NotFound {
            tracing::debug!(
                path = %cache_file.display(),
                error = %err,
                "scan cache prune skipped"
            );
        }
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
    use super::{
        DEFAULT_DIRECTORY_SCAN_CACHE_MAX_AGE_SECONDS, SCAN_CACHE_VERSION, ScanCacheLookup,
        ScanCacheMiss, ScanCachePolicy, ScanCacheStore, path_hash, unix_now,
    };
    use crate::scan::ScanReport;

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
        assert!(store.cache_file_for(&record.root).exists());
        assert_eq!(lookup, ScanCacheLookup::Hit(report));
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

        assert_eq!(lookup, ScanCacheLookup::Miss(ScanCacheMiss::Missing));
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

        assert_eq!(lookup, ScanCacheLookup::Miss(ScanCacheMiss::Corrupted));
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

        assert_eq!(lookup, ScanCacheLookup::Miss(ScanCacheMiss::Stale));
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

        assert_eq!(lookup, ScanCacheLookup::Miss(ScanCacheMiss::Stale));
        assert!(!store.cache_file_for(&root).exists());
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

        assert_eq!(lookup, ScanCacheLookup::Miss(ScanCacheMiss::Expired));
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

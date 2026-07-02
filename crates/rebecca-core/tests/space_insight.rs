use std::ffi::OsString;
use std::sync::{Mutex, MutexGuard, OnceLock};

use rebecca_core::EstimateSource;
use rebecca_core::inspect::{
    SpaceInsightDiagnosticKind, SpaceInsightRequest, SpaceInsightScanCache, inspect_space,
};
use rebecca_core::scan::{ScanBackendKind, ScanCancellationToken, ScanEstimateConfidence};
use rebecca_core::scan_cache::{ScanCachePolicy, ScanCacheStore};

static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
const TEST_DISABLE_LIVE_NTFS_MFT_ENV: &str = "REBECCA_TEST_DISABLE_LIVE_NTFS_MFT";

struct EnvVarGuard {
    key: &'static str,
    previous: Option<OsString>,
    _guard: MutexGuard<'static, ()>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let guard = ENV_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .expect("test environment lock is poisoned");
        let previous = std::env::var_os(key);
        unsafe {
            std::env::set_var(key, value);
        }
        Self {
            key,
            previous,
            _guard: guard,
        }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        unsafe {
            if let Some(previous) = &self.previous {
                std::env::set_var(self.key, previous);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }
}

fn write_file(path: impl AsRef<std::path::Path>, bytes: &[u8]) {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, bytes).unwrap();
}

#[test]
fn space_insight_reports_top_entries_in_deterministic_order() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    write_file(root.join("zeta").join("data.bin"), b"abc");
    write_file(root.join("alpha").join("data.bin"), b"abc");
    write_file(root.join("small.txt"), b"x");

    let request = SpaceInsightRequest::new(vec![root.clone()]).with_top_limit(2);
    let report = inspect_space(&request, &ScanCancellationToken::new()).unwrap();

    assert_eq!(report.roots.len(), 1);
    assert_eq!(report.totals.estimated_bytes, 7);
    assert_eq!(report.totals.files, 3);
    assert_eq!(report.top_entries.len(), 2);
    assert_eq!(report.top_entries[0].path, root.join("alpha"));
    assert_eq!(report.top_entries[1].path, root.join("zeta"));
    assert_eq!(
        report.top_entries[0].estimate_provenance.estimate_backend,
        Some(ScanBackendKind::PortableRecursive)
    );
    assert_eq!(
        report.top_entries[0]
            .estimate_provenance
            .estimate_confidence,
        Some(ScanEstimateConfidence::Exact)
    );
}

#[test]
fn space_insight_top_limit_zero_preserves_totals_without_entries() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    write_file(root.join("large.bin"), b"abc");
    write_file(root.join("small.bin"), b"x");

    let request = SpaceInsightRequest::new(vec![root]).with_top_limit(0);
    let report = inspect_space(&request, &ScanCancellationToken::new()).unwrap();

    assert_eq!(report.totals.estimated_bytes, 4);
    assert_eq!(report.totals.files, 2);
    assert!(report.top_entries.is_empty());
}

#[test]
fn space_insight_reports_root_diagnostics_without_failing() {
    let temp = tempfile::tempdir().unwrap();
    let missing = temp.path().join("missing");

    let request = SpaceInsightRequest::new(vec![missing.clone()]);
    let report = inspect_space(&request, &ScanCancellationToken::new()).unwrap();

    assert_eq!(report.roots.len(), 1);
    assert_eq!(report.roots[0].path, missing);
    assert_eq!(report.roots[0].status.label(), "skipped");
    assert_eq!(report.diagnostics.len(), 1);
    assert_eq!(
        report.diagnostics[0].kind,
        SpaceInsightDiagnosticKind::RootMissing
    );
}

#[test]
fn space_insight_preserves_scan_cache_estimate_source() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    write_file(root.join("target").join("debug").join("app.bin"), b"abcd");
    let store = ScanCacheStore::new(temp.path().join("cache").join("scan"));
    let scan_cache = SpaceInsightScanCache::new(store.clone(), ScanCachePolicy::default());
    let request = SpaceInsightRequest::new(vec![root.clone()]).with_scan_cache(scan_cache);

    let first = inspect_space(&request, &ScanCancellationToken::new()).unwrap();
    assert_eq!(
        first.top_entries[0].estimate_source,
        EstimateSource::FreshScan
    );

    let second = inspect_space(&request, &ScanCancellationToken::new()).unwrap();
    assert_eq!(
        second.top_entries[0].estimate_source,
        EstimateSource::ScanCache
    );
    assert_eq!(
        second.top_entries[0].estimate_provenance.estimate_backend,
        Some(ScanBackendKind::PortableRecursive)
    );
    assert_eq!(
        second.top_entries[0]
            .estimate_provenance
            .estimate_confidence,
        Some(ScanEstimateConfidence::Exact)
    );
    assert_eq!(second.top_entries[0].estimated_bytes, 4);
}

#[test]
fn space_insight_reports_experimental_backend_provenance() {
    let _env = EnvVarGuard::set(TEST_DISABLE_LIVE_NTFS_MFT_ENV, "1");

    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    write_file(root.join("target").join("app.bin"), b"abcd");
    let request = SpaceInsightRequest::new(vec![root])
        .with_scan_backend(ScanBackendKind::WindowsNtfsMftExperimental);

    let report = inspect_space(&request, &ScanCancellationToken::new()).unwrap();

    assert_eq!(
        report.top_entries[0].estimate_source,
        EstimateSource::FreshScan
    );
    assert!(
        report.top_entries[0]
            .estimate_provenance
            .estimate_backend
            .is_some()
    );
    let provenance = &report.top_entries[0].estimate_provenance;
    if let Some(reason) = provenance.estimate_fallback_reason.as_deref() {
        assert!(reason.contains("windows-ntfs-mft-experimental"));
        assert!(
            provenance
                .estimate_caveats
                .iter()
                .any(|caveat| caveat.code == "experimental-ntfs-mft-fallback")
        );
    } else {
        assert_eq!(
            provenance.estimate_backend,
            Some(ScanBackendKind::WindowsNtfsMftExperimental)
        );
        assert!(
            provenance
                .estimate_backend_source
                .as_deref()
                .is_some_and(|source| source.starts_with("windows-ntfs-mft-experimental-"))
        );
    }
}

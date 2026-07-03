use std::ffi::OsString;
use std::sync::{Mutex, MutexGuard, OnceLock};

use rebecca_core::disk_map::{DiskMapDiagnosticKind, DiskMapRequest, inspect_map};
use rebecca_core::scan::{ScanBackendKind, ScanCancellationToken, ScanEstimateConfidence};

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
fn disk_map_reports_ranked_entries_in_deterministic_order() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    write_file(root.join("zeta").join("data.bin"), b"abc");
    write_file(root.join("alpha").join("data.bin"), b"abc");
    write_file(root.join("small.txt"), b"x");

    let request = DiskMapRequest::new(vec![root.clone()])
        .with_scan_backend(ScanBackendKind::PortableRecursive)
        .with_top_limit(2);
    let report = inspect_map(&request, &ScanCancellationToken::new()).unwrap();

    assert_eq!(report.roots.len(), 1);
    assert_eq!(report.totals.logical_bytes, 7);
    assert_eq!(report.totals.allocated_bytes, None);
    assert_eq!(report.totals.files, 3);
    assert_eq!(report.totals.directories, 2);
    assert_eq!(report.top_entries.len(), 2);
    assert_eq!(report.top_entries[0].path, root.join("alpha"));
    assert_eq!(report.top_entries[1].path, root.join("zeta"));
    assert_eq!(report.top_entries[0].depth, 1);
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
fn disk_map_top_limit_zero_preserves_totals_without_entries() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    write_file(root.join("large.bin"), b"abc");
    write_file(root.join("small.bin"), b"x");

    let request = DiskMapRequest::new(vec![root])
        .with_scan_backend(ScanBackendKind::PortableRecursive)
        .with_top_limit(0);
    let report = inspect_map(&request, &ScanCancellationToken::new()).unwrap();

    assert_eq!(report.totals.logical_bytes, 4);
    assert_eq!(report.totals.files, 2);
    assert!(report.top_entries.is_empty());
}

#[test]
fn disk_map_file_root_is_reported_as_depth_zero_entry() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("single.bin");
    write_file(&root, b"abcdef");

    let request = DiskMapRequest::new(vec![root.clone()])
        .with_scan_backend(ScanBackendKind::PortableRecursive)
        .with_top_limit(10);
    let report = inspect_map(&request, &ScanCancellationToken::new()).unwrap();

    assert_eq!(report.totals.logical_bytes, 6);
    assert_eq!(report.totals.files, 1);
    assert_eq!(report.totals.directories, 0);
    assert_eq!(report.top_entries.len(), 1);
    assert_eq!(report.top_entries[0].path, root);
    assert_eq!(report.top_entries[0].depth, 0);
    assert_eq!(report.top_entries[0].logical_bytes, 6);
}

#[test]
fn disk_map_max_depth_limits_entries_but_not_totals() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    write_file(root.join("alpha").join("nested").join("data.bin"), b"abcd");

    let request = DiskMapRequest::new(vec![root.clone()])
        .with_scan_backend(ScanBackendKind::PortableRecursive)
        .with_top_limit(10)
        .with_max_depth(Some(1));
    let report = inspect_map(&request, &ScanCancellationToken::new()).unwrap();

    assert_eq!(report.totals.logical_bytes, 4);
    assert_eq!(report.totals.files, 1);
    assert_eq!(report.totals.directories, 2);
    assert_eq!(report.top_entries.len(), 1);
    assert_eq!(report.top_entries[0].path, root.join("alpha"));
    assert_eq!(report.top_entries[0].logical_bytes, 4);
}

#[test]
fn disk_map_reports_missing_root_without_failing() {
    let temp = tempfile::tempdir().unwrap();
    let missing = temp.path().join("missing");

    let request = DiskMapRequest::new(vec![missing.clone()])
        .with_scan_backend(ScanBackendKind::PortableRecursive);
    let report = inspect_map(&request, &ScanCancellationToken::new()).unwrap();

    assert_eq!(report.roots.len(), 1);
    assert_eq!(report.roots[0].path, missing);
    assert_eq!(report.roots[0].status.label(), "skipped");
    assert_eq!(report.diagnostics.len(), 1);
    assert_eq!(
        report.diagnostics[0].kind,
        DiskMapDiagnosticKind::RootMissing
    );
}

#[test]
fn disk_map_experimental_backend_records_portable_fallback() {
    let _env = EnvVarGuard::set(TEST_DISABLE_LIVE_NTFS_MFT_ENV, "1");

    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    write_file(root.join("target").join("app.bin"), b"abcd");

    let request = DiskMapRequest::new(vec![root])
        .with_scan_backend(ScanBackendKind::WindowsNtfsMftExperimental);
    let report = inspect_map(&request, &ScanCancellationToken::new()).unwrap();

    assert_eq!(report.totals.logical_bytes, 4);
    assert_eq!(
        report.roots[0].estimate_provenance.estimate_backend,
        Some(ScanBackendKind::PortableRecursive)
    );
    assert!(
        report.roots[0]
            .estimate_provenance
            .estimate_fallback_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("windows-ntfs-mft-experimental"))
    );
    assert_eq!(report.diagnostics[0].kind, DiskMapDiagnosticKind::Fallback);
}

#[cfg(windows)]
#[test]
fn disk_map_windows_native_backend_reports_native_provenance() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    write_file(root.join("alpha").join("data.bin"), b"abcd");
    write_file(root.join("beta.bin"), b"xyz");

    let request = DiskMapRequest::new(vec![root.clone()])
        .with_scan_backend(ScanBackendKind::WindowsNative)
        .with_top_limit(10);
    let report = inspect_map(&request, &ScanCancellationToken::new()).unwrap();

    assert_eq!(report.totals.logical_bytes, 7);
    assert!(
        report
            .totals
            .allocated_bytes
            .is_some_and(|bytes| bytes >= report.totals.logical_bytes),
        "windows-native disk maps should report file allocation when the host API exposes it: {:?}",
        report.totals.allocated_bytes
    );
    assert_eq!(report.totals.files, 2);
    assert_eq!(report.totals.directories, 1);
    assert!(report.diagnostics.is_empty());
    assert_eq!(
        report.roots[0].estimate_provenance.estimate_backend,
        Some(ScanBackendKind::WindowsNative)
    );
    assert_eq!(
        report.roots[0].estimate_provenance.estimate_confidence,
        Some(ScanEstimateConfidence::Exact)
    );
    assert_eq!(
        report.top_entries[0].estimate_provenance.estimate_backend,
        Some(ScanBackendKind::WindowsNative)
    );
    assert!(
        report.top_entries[0]
            .allocated_bytes
            .is_some_and(|bytes| bytes >= report.top_entries[0].logical_bytes)
    );
    assert_eq!(report.top_entries[0].path, root.join("alpha"));
}

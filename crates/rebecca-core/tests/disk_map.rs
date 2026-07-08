use std::ffi::OsString;
use std::sync::{Mutex, MutexGuard, OnceLock};

use rebecca_core::disk_map::{
    DiskMapDiagnosticKind, DiskMapEntryKind, DiskMapGroupKind, DiskMapRequest, DiskMapSortField,
    inspect_map,
};
use rebecca_core::inventory::{
    InventoryDiagnosticKind, InventoryEntryKind, InventoryGroupKind, InventoryMetrics,
    InventorySortField,
};
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
fn disk_map_public_domain_types_are_inventory_types() {
    let metrics: InventoryMetrics = rebecca_core::disk_map::DiskMapMetrics::default();
    let _: rebecca_core::disk_map::DiskMapMetrics = metrics;

    assert_eq!(DiskMapEntryKind::File, InventoryEntryKind::File);
    assert_eq!(DiskMapGroupKind::Extension, InventoryGroupKind::Extension);
    assert_eq!(DiskMapSortField::Allocated, InventorySortField::Allocated);
    assert_eq!(
        DiskMapDiagnosticKind::Fallback,
        InventoryDiagnosticKind::Fallback
    );
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
    #[cfg(unix)]
    assert!(report.totals.allocated_bytes.is_some());
    #[cfg(not(unix))]
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

#[cfg(unix)]
#[test]
fn disk_map_portable_unix_reports_allocated_and_unique_hardlinks() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    let original = root.join("original.bin");
    let linked = root.join("linked.bin");
    write_file(&original, b"abcd");
    std::fs::hard_link(&original, &linked).unwrap();

    let request = DiskMapRequest::new(vec![root])
        .with_scan_backend(ScanBackendKind::PortableRecursive)
        .with_top_limit(10);
    let report = inspect_map(&request, &ScanCancellationToken::new()).unwrap();

    assert_eq!(report.totals.logical_bytes, 8);
    assert_eq!(report.totals.unique_logical_bytes, Some(4));
    assert_eq!(report.roots[0].metrics.unique_logical_bytes, Some(4));
    let allocated_bytes = report
        .totals
        .allocated_bytes
        .expect("Unix portable disk maps should report st_blocks allocation");
    let unique_allocated_bytes = report
        .totals
        .unique_allocated_bytes
        .expect("Unix portable disk maps should deduplicate hardlink allocation");
    assert!(allocated_bytes >= unique_allocated_bytes);
    assert!(
        report.roots[0]
            .estimate_provenance
            .estimate_caveats
            .iter()
            .any(|caveat| caveat.code == "hardlink-file")
    );
}

#[test]
fn disk_map_sorts_top_entries_by_requested_field() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    write_file(root.join("many").join("a.txt"), b"x");
    write_file(root.join("many").join("b.txt"), b"x");
    write_file(root.join("large.bin"), b"abcdefghij");

    let request = DiskMapRequest::new(vec![root.clone()])
        .with_scan_backend(ScanBackendKind::PortableRecursive)
        .with_top_limit(2)
        .with_top_sort(DiskMapSortField::Files);
    let report = inspect_map(&request, &ScanCancellationToken::new()).unwrap();

    assert_eq!(report.top_entries.len(), 2);
    assert_eq!(report.top_entries[0].path, root.join("many"));
    assert_eq!(report.top_entries[0].files, 2);
    assert_eq!(report.top_entries[1].path, root.join("large.bin"));
    assert_eq!(report.top_entries[1].logical_bytes, 10);
}

#[test]
fn disk_map_groups_files_by_type_extension_depth_and_age() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    write_file(root.join("alpha").join("src").join("main.rs"), b"abc");
    write_file(root.join("beta").join("readme.md"), b"abcde");
    write_file(root.join("LICENSE"), b"xy");

    let request = DiskMapRequest::new(vec![root])
        .with_scan_backend(ScanBackendKind::PortableRecursive)
        .with_top_limit(0)
        .with_group_kinds(vec![
            DiskMapGroupKind::Extension,
            DiskMapGroupKind::Type,
            DiskMapGroupKind::Depth,
            DiskMapGroupKind::Age,
        ])
        .with_group_limit(10);
    let report = inspect_map(&request, &ScanCancellationToken::new()).unwrap();

    assert_eq!(report.top_entries.len(), 0);
    assert_eq!(report.groups.len(), 9);
    assert_group_metrics(&report, DiskMapGroupKind::Extension, ".md", 5, 1);
    assert_group_metrics(&report, DiskMapGroupKind::Extension, ".rs", 3, 1);
    assert_group_metrics(&report, DiskMapGroupKind::Extension, "[no-extension]", 2, 1);
    assert_group_metrics(&report, DiskMapGroupKind::Type, "file", 10, 3);
    assert_group_directories(&report, DiskMapGroupKind::Type, "directory", 3);
    assert_group_metrics(&report, DiskMapGroupKind::Depth, "depth-1", 2, 1);
    assert_group_metrics(&report, DiskMapGroupKind::Depth, "depth-2", 5, 1);
    assert_group_metrics(&report, DiskMapGroupKind::Depth, "depth-3", 3, 1);
    assert_group_metrics(&report, DiskMapGroupKind::Age, "modified-7d", 10, 3);
}

#[test]
fn disk_map_sorts_groups_by_requested_field() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    write_file(root.join("a.txt"), b"x");
    write_file(root.join("b.txt"), b"x");
    write_file(root.join("large.bin"), b"abcdefghij");

    let request = DiskMapRequest::new(vec![root])
        .with_scan_backend(ScanBackendKind::PortableRecursive)
        .with_top_limit(0)
        .with_group_kinds(vec![DiskMapGroupKind::Extension])
        .with_group_limit(2)
        .with_group_sort(DiskMapSortField::Files);
    let report = inspect_map(&request, &ScanCancellationToken::new()).unwrap();

    assert_eq!(report.groups.len(), 2);
    assert_eq!(report.groups[0].key, ".txt");
    assert_eq!(report.groups[0].metrics.files, 2);
    assert_eq!(report.groups[1].key, ".bin");
    assert_eq!(report.groups[1].metrics.logical_bytes, 10);
}

fn assert_group_metrics(
    report: &rebecca_core::disk_map::DiskMapReport,
    kind: DiskMapGroupKind,
    key: &str,
    logical_bytes: u64,
    files: u64,
) {
    let group = report
        .groups
        .iter()
        .find(|group| group.kind == kind && group.key == key)
        .unwrap_or_else(|| panic!("missing group {kind:?}:{key}"));
    assert_eq!(group.metrics.logical_bytes, logical_bytes);
    assert_eq!(group.metrics.files, files);
}

fn assert_group_directories(
    report: &rebecca_core::disk_map::DiskMapReport,
    kind: DiskMapGroupKind,
    key: &str,
    directories: u64,
) {
    let group = report
        .groups
        .iter()
        .find(|group| group.kind == kind && group.key == key)
        .unwrap_or_else(|| panic!("missing group {kind:?}:{key}"));
    assert_eq!(group.metrics.logical_bytes, 0);
    assert_eq!(group.metrics.files, 0);
    assert_eq!(group.metrics.directories, directories);
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
fn disk_map_filters_ranked_entries_by_min_logical_bytes_without_changing_totals() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    write_file(root.join("large.bin"), b"abcdef");
    write_file(root.join("small.bin"), b"x");

    let request = DiskMapRequest::new(vec![root.clone()])
        .with_scan_backend(ScanBackendKind::PortableRecursive)
        .with_top_limit(10)
        .with_min_logical_bytes(Some(2));
    let report = inspect_map(&request, &ScanCancellationToken::new()).unwrap();

    assert_eq!(report.totals.logical_bytes, 7);
    assert_eq!(report.totals.files, 2);
    assert_eq!(report.top_entries.len(), 1);
    assert_eq!(report.top_entries[0].path, root.join("large.bin"));
}

#[test]
fn disk_map_filters_ranked_entries_by_kind_without_changing_totals() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    write_file(root.join("node_modules").join("cache.bin"), b"abcd");
    write_file(root.join("plain.bin"), b"xy");

    let request = DiskMapRequest::new(vec![root.clone()])
        .with_scan_backend(ScanBackendKind::PortableRecursive)
        .with_top_limit(10)
        .with_entry_kind(Some(DiskMapEntryKind::Directory));
    let report = inspect_map(&request, &ScanCancellationToken::new()).unwrap();

    assert_eq!(report.totals.logical_bytes, 6);
    assert_eq!(report.totals.files, 2);
    assert_eq!(report.totals.directories, 1);
    assert_eq!(report.top_entries.len(), 1);
    assert_eq!(report.top_entries[0].kind, DiskMapEntryKind::Directory);
    assert_eq!(report.top_entries[0].path, root.join("node_modules"));
}

#[test]
fn disk_map_filters_ranked_entries_by_case_insensitive_path_substring() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    write_file(root.join("AlphaCache").join("data.bin"), b"abcd");
    write_file(root.join("beta").join("data.bin"), b"xyz");

    let request = DiskMapRequest::new(vec![root.clone()])
        .with_scan_backend(ScanBackendKind::PortableRecursive)
        .with_top_limit(10)
        .with_path_contains(Some("alpha".to_string()));
    let report = inspect_map(&request, &ScanCancellationToken::new()).unwrap();

    assert_eq!(report.totals.logical_bytes, 7);
    assert_eq!(report.top_entries.len(), 2);
    assert!(
        report
            .top_entries
            .iter()
            .all(|entry| entry.path.to_string_lossy().contains("AlphaCache"))
    );
    assert!(report.top_entries.iter().any(|entry| {
        entry.kind == DiskMapEntryKind::Directory && entry.path == root.join("AlphaCache")
    }));
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

#[test]
fn disk_map_experimental_backend_fallback_preserves_groups() {
    let _env = EnvVarGuard::set(TEST_DISABLE_LIVE_NTFS_MFT_ENV, "1");

    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    write_file(root.join("target").join("app.bin"), b"abcd");

    let request = DiskMapRequest::new(vec![root])
        .with_scan_backend(ScanBackendKind::WindowsNtfsMftExperimental)
        .with_group_kinds(vec![DiskMapGroupKind::Extension]);
    let report = inspect_map(&request, &ScanCancellationToken::new()).unwrap();

    assert_group_metrics(&report, DiskMapGroupKind::Extension, ".bin", 4, 1);
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

#[cfg(windows)]
#[test]
fn disk_map_windows_native_reports_hardlink_caveats() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    let original = root.join("original.bin");
    let linked = root.join("linked.bin");
    write_file(&original, b"abcd");
    std::fs::hard_link(&original, &linked).unwrap();

    let request = DiskMapRequest::new(vec![root])
        .with_scan_backend(ScanBackendKind::WindowsNative)
        .with_top_limit(10);
    let report = inspect_map(&request, &ScanCancellationToken::new()).unwrap();

    assert_eq!(report.totals.logical_bytes, 8);
    assert_eq!(report.totals.unique_logical_bytes, Some(4));
    assert_eq!(report.roots[0].metrics.unique_logical_bytes, Some(4));
    let allocated_bytes = report
        .totals
        .allocated_bytes
        .expect("native hardlink fixture should report path-ranked allocated bytes");
    let unique_allocated_bytes = report
        .totals
        .unique_allocated_bytes
        .expect("native hardlink fixture should report unique allocated bytes");
    assert!(
        allocated_bytes >= unique_allocated_bytes,
        "path-ranked allocation should be at least unique allocation"
    );
    assert!(
        unique_allocated_bytes >= 4,
        "unique allocation should include the hardlinked file payload"
    );
    assert!(
        report.roots[0]
            .estimate_provenance
            .estimate_caveats
            .iter()
            .any(|caveat| caveat.code == "hardlink-file")
    );
    assert!(report.top_entries.iter().any(|entry| {
        entry
            .estimate_provenance
            .estimate_caveats
            .iter()
            .any(|caveat| caveat.code == "hardlink-file")
    }));
}

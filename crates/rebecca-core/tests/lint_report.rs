use std::fs;
use std::path::{Path, PathBuf};

use rebecca_core::inventory::InventoryEntryRole;
use rebecca_core::lint::{LintReportRequest, inspect_lint};
use rebecca_core::scan::ScanCancellationToken;

fn write_file(path: impl AsRef<Path>, bytes: &[u8]) {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, bytes).unwrap();
}

#[test]
fn singleton_file_sizes_are_not_hashed_into_duplicate_groups() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    write_file(root.join("only.bin"), b"abc");

    let report = inspect_lint(
        &LintReportRequest::new(vec![root]),
        &ScanCancellationToken::new(),
    )
    .unwrap();

    assert_eq!(report.summary.files_scanned, 1);
    assert_eq!(report.summary.duplicate_groups, 0);
    assert!(report.duplicate_groups.is_empty());
}

#[test]
fn identical_files_form_duplicate_group_after_full_hash() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    write_file(root.join("a.bin"), b"same");
    write_file(root.join("nested").join("b.bin"), b"same");
    write_file(root.join("different.bin"), b"diff");

    let report = inspect_lint(
        &LintReportRequest::new(vec![root.clone()]),
        &ScanCancellationToken::new(),
    )
    .unwrap();

    assert_eq!(report.summary.duplicate_groups, 1);
    assert_eq!(report.summary.duplicate_files, 2);
    assert_eq!(report.summary.conservative_reclaim_bytes, 4);
    let group = &report.duplicate_groups[0];
    assert_eq!(group.size_bytes, 4);
    assert_eq!(group.total_files, 2);
    assert_eq!(group.conservative_reclaim_bytes, 4);
    assert_eq!(
        group
            .files
            .iter()
            .map(|file| file.path.clone())
            .collect::<Vec<_>>(),
        vec![root.join("a.bin"), root.join("nested").join("b.bin")]
    );
}

#[test]
fn same_size_different_content_does_not_group() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    write_file(root.join("a.bin"), b"abcd");
    write_file(root.join("b.bin"), b"wxyz");

    let report = inspect_lint(
        &LintReportRequest::new(vec![root]),
        &ScanCancellationToken::new(),
    )
    .unwrap();

    assert_eq!(report.summary.duplicate_groups, 0);
    assert_eq!(report.summary.conservative_reclaim_bytes, 0);
}

#[test]
fn reference_and_protected_roots_are_keep_candidates() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    let protected = root.join("protected");
    let reference = root.join("reference");
    let scanned = root.join("scan");
    write_file(protected.join("master.bin"), b"same");
    write_file(reference.join("copy.bin"), b"same");
    write_file(scanned.join("copy.bin"), b"same");

    let report = inspect_lint(
        &LintReportRequest::new(vec![root])
            .with_protected_roots(vec![protected.clone()])
            .with_reference_roots(vec![reference.clone()]),
        &ScanCancellationToken::new(),
    )
    .unwrap();

    let group = &report.duplicate_groups[0];
    assert_eq!(group.total_files, 3);
    assert_eq!(group.keep_candidates, 2);
    assert_eq!(group.conservative_reclaim_bytes, 4);
    assert_eq!(group.files[0].role, InventoryEntryRole::Protected);
    assert_eq!(group.files[1].role, InventoryEntryRole::Reference);
    assert_eq!(group.files[2].role, InventoryEntryRole::Scanned);
}

#[test]
fn empty_files_large_files_and_empty_directories_are_reported() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    let deepest = root.join("empty").join("nested").join("deep");
    fs::create_dir_all(&deepest).unwrap();
    fs::create_dir_all(root.join("empty").join("sibling")).unwrap();
    write_file(root.join("empty-file.txt"), b"");
    write_file(root.join("large.bin"), b"abcdef");

    let report = inspect_lint(
        &LintReportRequest::new(vec![root.clone()]).with_large_file_threshold_bytes(5),
        &ScanCancellationToken::new(),
    )
    .unwrap();

    assert_eq!(report.summary.empty_files, 1);
    assert_eq!(report.empty_files[0].path, root.join("empty-file.txt"));
    assert_eq!(report.summary.large_files, 1);
    assert_eq!(report.large_files[0].path, root.join("large.bin"));
    assert_eq!(
        report
            .empty_directories
            .iter()
            .map(|directory| directory.path.clone())
            .take(2)
            .collect::<Vec<PathBuf>>(),
        vec![deepest, root.join("empty").join("sibling")]
    );
}

#[test]
fn excluded_paths_are_not_reported_or_grouped() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    let excluded = root.join("excluded");
    write_file(root.join("kept.bin"), b"same");
    write_file(excluded.join("copy.bin"), b"same");

    let report = inspect_lint(
        &LintReportRequest::new(vec![root])
            .with_exclude_paths(vec![excluded])
            .with_large_file_threshold_bytes(1),
        &ScanCancellationToken::new(),
    )
    .unwrap();

    assert_eq!(report.summary.files_scanned, 1);
    assert_eq!(report.summary.duplicate_groups, 0);
    assert_eq!(report.summary.large_files, 1);
    assert!(report.diagnostics.iter().any(|diagnostic| {
        diagnostic.kind.label() == "excluded" && diagnostic.path.ends_with("excluded")
    }));
}

#[test]
fn excluding_a_file_does_not_exclude_its_parent_directory() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    let excluded_file = root.join("nested").join("ignored.bin");
    write_file(&excluded_file, b"ignored");
    write_file(root.join("kept.bin"), b"kept");

    let report = inspect_lint(
        &LintReportRequest::new(vec![root.clone()]).with_exclude_paths(vec![excluded_file]),
        &ScanCancellationToken::new(),
    )
    .unwrap();

    assert_eq!(report.summary.files_scanned, 1);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.path.ends_with("ignored.bin"))
    );
    assert!(
        report
            .empty_directories
            .iter()
            .any(|directory| directory.path == root.join("nested"))
    );
}

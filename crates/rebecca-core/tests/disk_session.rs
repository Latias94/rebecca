use std::path::Path;

use rebecca_core::disk_map::{
    DiskMapEntryKind, DiskMapGroupKind, DiskMapRequest, DiskMapSortField, inspect_map,
};
use rebecca_core::disk_session::{DiskMapDistributionFilter, DiskMapSession, DiskMapSessionFilter};
use rebecca_core::scan::{ScanBackendKind, ScanCancellationToken};

fn write_file(path: impl AsRef<Path>, bytes: &[u8]) {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, bytes).unwrap();
}

#[test]
fn disk_session_preserves_root_metrics_and_navigates_children() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    write_file(root.join("big").join("data.bin"), b"abcdef");
    write_file(root.join("small.txt"), b"x");

    let report = inspect_map(
        &DiskMapRequest::new(vec![root.clone()])
            .with_scan_backend(ScanBackendKind::PortableRecursive)
            .with_top_limit(10),
        &ScanCancellationToken::new(),
    )
    .unwrap();
    let session = DiskMapSession::from_report(report);

    let root_id = session.root_ids()[0];
    let root_node = session.node(root_id).unwrap();
    assert_eq!(root_node.path, root);
    assert_eq!(root_node.metrics.logical_bytes, 7);

    let rows = session.visible_rows(
        Some(root_id),
        DiskMapSortField::Logical,
        DiskMapSessionFilter::default(),
    );
    assert_eq!(rows[0].name, "big");
    assert_eq!(rows[0].metrics.logical_bytes, 6);
    assert_eq!(rows[1].name, "small.txt");
    assert_eq!(rows[1].metrics.logical_bytes, 1);
}

#[test]
fn disk_session_reconstructs_visible_parent_chain_from_ranked_entries() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    write_file(
        root.join("parent").join("child").join("large.bin"),
        b"abcdef",
    );

    let report = inspect_map(
        &DiskMapRequest::new(vec![root.clone()])
            .with_scan_backend(ScanBackendKind::PortableRecursive)
            .with_entry_kind(Some(DiskMapEntryKind::File))
            .with_top_limit(1),
        &ScanCancellationToken::new(),
    )
    .unwrap();
    assert_eq!(report.top_entries.len(), 1);

    let session = DiskMapSession::from_report(report);
    let root_id = session.root_ids()[0];
    let rows = session.visible_rows(
        Some(root_id),
        DiskMapSortField::Logical,
        DiskMapSessionFilter::default(),
    );

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name, "parent");
    assert!(rows[0].synthetic);
    assert!(rows[0].has_children);
}

#[test]
fn disk_session_filters_rows_by_path_text() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    write_file(root.join("target").join("data.bin"), b"abc");
    write_file(root.join("other").join("data.bin"), b"abc");

    let report = inspect_map(
        &DiskMapRequest::new(vec![root])
            .with_scan_backend(ScanBackendKind::PortableRecursive)
            .with_top_limit(10),
        &ScanCancellationToken::new(),
    )
    .unwrap();
    let session = DiskMapSession::from_report(report);
    let root_id = session.root_ids()[0];
    let rows = session.visible_rows(
        Some(root_id),
        DiskMapSortField::Logical,
        DiskMapSessionFilter {
            path_contains: Some("target"),
            ..DiskMapSessionFilter::default()
        },
    );

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name, "target");
}

#[test]
fn disk_session_filters_rows_by_kind_and_extension() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    write_file(root.join("cache.tmp"), b"abc");
    write_file(root.join("log.txt"), b"abc");
    write_file(root.join("nested").join("ignored.tmp"), b"abc");

    let report = inspect_map(
        &DiskMapRequest::new(vec![root])
            .with_scan_backend(ScanBackendKind::PortableRecursive)
            .with_top_limit(10),
        &ScanCancellationToken::new(),
    )
    .unwrap();
    let session = DiskMapSession::from_report(report);
    let root_id = session.root_ids()[0];

    let rows = session.visible_rows(
        Some(root_id),
        DiskMapSortField::Logical,
        DiskMapSessionFilter {
            path_contains: None,
            entry_kind: Some(DiskMapEntryKind::File),
            extension: Some(".tmp"),
        },
    );

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name, "cache.tmp");
}

#[test]
fn disk_session_restores_node_identity_by_path() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    let parent = root.join("target");
    let child = parent.join("data.bin");
    write_file(&child, b"abc");

    let report = inspect_map(
        &DiskMapRequest::new(vec![root.clone()])
            .with_scan_backend(ScanBackendKind::PortableRecursive)
            .with_top_limit(10),
        &ScanCancellationToken::new(),
    )
    .unwrap();
    let session = DiskMapSession::from_report(report);

    let parent_id = session.node_id_by_path(&parent).expect("parent id");
    assert_eq!(session.node_path(parent_id), Some(parent.as_path()));
    assert_eq!(
        session.nearest_existing_ancestor(parent.join("missing.bin")),
        Some(parent_id)
    );
    assert_eq!(
        session.restore_parent_by_path(Some(&parent)),
        Some(parent_id)
    );
    assert_eq!(
        session.restore_parent_by_path(Some(&root.join("missing"))),
        session.root_ids().first().copied()
    );
    assert!(session.node_id_by_path(root.join("missing")).is_none());
}

#[test]
fn disk_session_preserves_extension_distribution_rows() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    write_file(root.join("src").join("main.rs"), b"abc");
    write_file(root.join("docs").join("readme.md"), b"abcde");
    write_file(root.join("LICENSE"), b"xy");

    let report = inspect_map(
        &DiskMapRequest::new(vec![root])
            .with_scan_backend(ScanBackendKind::PortableRecursive)
            .with_top_limit(10)
            .with_group_kinds(vec![DiskMapGroupKind::Extension])
            .with_group_limit(10),
        &ScanCancellationToken::new(),
    )
    .unwrap();
    let session = DiskMapSession::from_report(report);
    let rows = session.distribution_rows(
        DiskMapGroupKind::Extension,
        DiskMapSortField::Logical,
        DiskMapDistributionFilter::default(),
    );

    assert_eq!(
        rows.iter()
            .map(|row| (
                row.key.as_str(),
                row.label.as_str(),
                row.metrics.logical_bytes
            ))
            .collect::<Vec<_>>(),
        vec![
            (".md", ".md", 5),
            (".rs", ".rs", 3),
            ("[no-extension]", "No extension", 2),
        ]
    );
    assert_eq!(rows.iter().map(|row| row.metrics.files).sum::<u64>(), 3);
}

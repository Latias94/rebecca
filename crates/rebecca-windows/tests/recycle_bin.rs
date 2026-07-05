#![cfg(windows)]

use std::fs;

use rebecca_core::DeleteMode;
use rebecca_core::applications::ApplicationDiscovery;
use rebecca_core::executor::CleanupBackend;
use rebecca_core::plan::{CleanupTarget, CleanupTargetDeletionStyle};
use rebecca_windows::{PrivilegeLevel, WindowsRecycleBinBackend};

#[test]
fn privilege_detection_returns_known_shape() {
    let level = rebecca_windows::current_privilege_level();

    assert!(matches!(
        level,
        PrivilegeLevel::StandardUser | PrivilegeLevel::Elevated | PrivilegeLevel::Unknown
    ));
}

#[test]
fn recycle_bin_backend_moves_file_when_supported() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("delete-me.tmp");
    fs::write(&file, b"trash").unwrap();

    let target =
        CleanupTarget::allowed("windows.user-temp", file.clone(), 5, DeleteMode::RecycleBin);
    let backend = WindowsRecycleBinBackend::new();
    let outcome = backend.delete(&target).unwrap();

    assert_eq!(outcome.pending_reclaim_bytes, 5);
    assert!(!file.exists());
}

#[test]
fn recycle_bin_backend_preserves_target_directory() {
    let temp = tempfile::tempdir().unwrap();
    let cache = temp.path().join("cache");
    let child = cache.join("entry.tmp");
    fs::create_dir_all(&cache).unwrap();
    fs::write(&child, b"trash").unwrap();

    let target = CleanupTarget::allowed(
        "windows.user-temp",
        cache.clone(),
        5,
        DeleteMode::RecycleBin,
    );
    let backend = WindowsRecycleBinBackend::new();
    let outcome = backend.delete(&target).unwrap();

    assert_eq!(outcome.pending_reclaim_bytes, 5);
    assert!(cache.exists());
    assert!(!child.exists());
}

#[test]
fn recycle_bin_backend_preserves_target_directory_after_batching_multiple_entries() {
    let temp = tempfile::tempdir().unwrap();
    let cache = temp.path().join("cache");
    fs::create_dir_all(cache.join("nested")).unwrap();
    fs::write(cache.join("entry-a.tmp"), b"trash").unwrap();
    fs::write(cache.join("entry-b.tmp"), b"trash").unwrap();
    fs::write(cache.join("nested").join("entry-c.tmp"), b"trash").unwrap();

    let target = CleanupTarget::allowed(
        "windows.user-temp",
        cache.clone(),
        15,
        DeleteMode::RecycleBin,
    );
    let backend = WindowsRecycleBinBackend::new();
    let outcome = backend.delete(&target).unwrap();

    assert_eq!(outcome.pending_reclaim_bytes, 15);
    assert!(cache.exists());
    assert_eq!(fs::read_dir(&cache).unwrap().count(), 0);
}

#[test]
fn recycle_bin_backend_refuses_preserve_root_reparse_children() {
    let temp = tempfile::tempdir().unwrap();
    let cache = temp.path().join("cache");
    let outside = temp.path().join("outside");
    let normal_child = cache.join("entry.tmp");
    let linked_child = cache.join("linked");
    fs::create_dir_all(&cache).unwrap();
    fs::create_dir_all(&outside).unwrap();
    fs::write(&normal_child, b"trash").unwrap();
    std::os::windows::fs::symlink_dir(&outside, &linked_child).unwrap();

    let target = CleanupTarget::allowed(
        "windows.user-temp",
        cache.clone(),
        5,
        DeleteMode::RecycleBin,
    );
    let backend = WindowsRecycleBinBackend::new();
    let err = backend.delete(&target).unwrap_err();

    assert!(err.to_string().contains("refused reparse child"));
    assert!(cache.exists());
    assert!(normal_child.exists());
    assert!(linked_child.exists());
}

#[test]
fn recycle_bin_backend_batch_deletes_multiple_targets() {
    let temp = tempfile::tempdir().unwrap();
    let first = temp.path().join("first.tmp");
    let second = temp.path().join("second.tmp");
    fs::write(&first, b"trash").unwrap();
    fs::write(&second, b"trash").unwrap();

    let first_target =
        CleanupTarget::allowed("windows.first", first.clone(), 5, DeleteMode::RecycleBin);
    let second_target =
        CleanupTarget::allowed("windows.second", second.clone(), 7, DeleteMode::RecycleBin);
    let backend = WindowsRecycleBinBackend::new();
    let outcomes = backend.delete_batch(&[&first_target, &second_target]);

    assert_eq!(outcomes.len(), 2);
    assert_eq!(outcomes[0].as_ref().unwrap().pending_reclaim_bytes, 5);
    assert_eq!(outcomes[1].as_ref().unwrap().pending_reclaim_bytes, 7);
    assert!(!first.exists());
    assert!(!second.exists());
}

#[test]
fn recycle_bin_backend_batch_preserves_directory_roots() {
    let temp = tempfile::tempdir().unwrap();
    let cache = temp.path().join("cache");
    let child = cache.join("entry.tmp");
    fs::create_dir_all(&cache).unwrap();
    fs::write(&child, b"trash").unwrap();

    let target = CleanupTarget::allowed(
        "windows.user-temp",
        cache.clone(),
        5,
        DeleteMode::RecycleBin,
    );
    let backend = WindowsRecycleBinBackend::new();
    let outcomes = backend.delete_batch(&[&target]);

    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0].as_ref().unwrap().pending_reclaim_bytes, 5);
    assert!(cache.exists());
    assert!(!child.exists());
}

#[test]
fn recycle_bin_backend_deletes_whole_path_when_requested() {
    let temp = tempfile::tempdir().unwrap();
    let cache = temp.path().join("cache");
    let child = cache.join("entry.tmp");
    fs::create_dir_all(&cache).unwrap();
    fs::write(&child, b"trash").unwrap();

    let target = CleanupTarget::allowed(
        "windows.user-temp",
        cache.clone(),
        5,
        DeleteMode::RecycleBin,
    )
    .with_deletion_style(CleanupTargetDeletionStyle::DeleteWholePath);
    let backend = WindowsRecycleBinBackend::new();
    let outcome = backend.delete(&target).unwrap();

    assert_eq!(outcome.pending_reclaim_bytes, 5);
    assert!(!cache.exists());
    assert!(!child.exists());
}

#[test]
fn steam_discovery_returns_a_known_shape() {
    let discovery = rebecca_windows::steam::WindowsApplicationDiscovery::new();
    assert!(discovery.steam_installation().is_ok());
}

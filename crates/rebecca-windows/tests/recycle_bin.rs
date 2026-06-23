#![cfg(windows)]

use std::fs;

use rebecca_core::DeleteMode;
use rebecca_core::applications::ApplicationDiscovery;
use rebecca_core::executor::CleanupBackend;
use rebecca_core::plan::CleanupTarget;
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
fn steam_discovery_returns_a_known_shape() {
    let discovery = rebecca_windows::steam::WindowsApplicationDiscovery::new();
    assert!(discovery.steam_installation().is_ok());
}

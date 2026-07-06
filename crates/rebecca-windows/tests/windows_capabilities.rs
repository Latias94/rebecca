#![cfg(windows)]

use rebecca_core::applications::ApplicationDiscovery;
use rebecca_windows::PrivilegeLevel;

#[test]
fn privilege_detection_returns_known_shape() {
    let level = rebecca_windows::current_privilege_level();

    assert!(matches!(
        level,
        PrivilegeLevel::StandardUser | PrivilegeLevel::Elevated | PrivilegeLevel::Unknown
    ));
}

#[test]
fn steam_discovery_returns_a_known_shape() {
    let discovery = rebecca_windows::steam::WindowsApplicationDiscovery::new();
    assert!(discovery.steam_installation().is_ok());
}

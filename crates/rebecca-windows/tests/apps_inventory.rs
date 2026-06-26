#![cfg(windows)]

#[test]
fn installed_applications_inventory_is_best_effort() {
    let inventory = rebecca_windows::apps::discover_installed_applications();

    assert!(
        inventory.is_ok(),
        "inventory discovery should skip bad uninstall entries instead of failing the run"
    );
}

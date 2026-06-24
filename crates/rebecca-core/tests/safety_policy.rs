use std::path::PathBuf;

use rebecca_core::RuleTargetSpec;
use rebecca_core::config::AppPaths;
use rebecca_core::protection::{
    ProtectedCategory, ProtectionAssessment, ProtectionBlockKind, ProtectionPolicy,
};
use rebecca_core::safety::{PathDisposition, assess_existing_path, assess_path};

#[test]
fn allows_user_cache_subdirectories() {
    let disposition = assess_path(&PathBuf::from(
        "C:/Users/Alice/AppData/Local/Temp/rebecca-test",
    ));

    assert!(matches!(disposition, PathDisposition::Allowed));
}

#[test]
fn blocks_traversal_drive_roots_and_system_paths() {
    let cases = [
        "../Windows",
        "C:/",
        "C:/Windows/System32",
        "C:/Program Files/App",
        "C:/Users/Alice",
    ];

    for case in cases {
        let disposition = assess_path(&PathBuf::from(case));
        assert!(
            matches!(disposition, PathDisposition::Blocked(_)),
            "{case} should be blocked, got {disposition:?}"
        );
    }
}

#[test]
fn missing_existing_path_is_skipped() {
    let disposition = assess_existing_path(&PathBuf::from("C:/Rebecca/definitely-missing"));

    assert!(matches!(disposition, PathDisposition::Skipped(_)));
}

#[test]
fn browser_cache_paths_remain_allowed_while_private_data_is_blocked() {
    let policy = ProtectionPolicy::new();

    assert!(matches!(
        policy.assess_path(&PathBuf::from(
            "C:/Users/Alice/AppData/Local/Google/Chrome/User Data/Default/Cache"
        )),
        ProtectionAssessment::Allowed
    ));
    assert!(matches!(
        policy.assess_path(&PathBuf::from(
            "C:/Users/Alice/AppData/Local/Google/Chrome/User Data/Default/History"
        )),
        ProtectionAssessment::Blocked(block)
            if block.kind == ProtectionBlockKind::ProtectedCategory(ProtectedCategory::BrowserPrivateData)
    ));
    assert!(matches!(
        policy.assess_path(&PathBuf::from(
            "C:/Users/Alice/AppData/Roaming/Mozilla/Firefox/Profiles/abcd1234.cache2/cache2"
        )),
        ProtectionAssessment::Allowed
    ));
    assert!(matches!(
        policy.assess_path(&PathBuf::from(
            "C:/Users/Alice/AppData/Roaming/Mozilla/Firefox/Profiles/abcd1234.default/cookies.sqlite"
        )),
        ProtectionAssessment::Blocked(block)
            if block.kind == ProtectionBlockKind::ProtectedCategory(ProtectedCategory::BrowserPrivateData)
    ));
}

#[test]
fn credentials_ai_cloud_runtime_and_startup_data_are_blocked() {
    let policy = ProtectionPolicy::new();

    for (path, expected_category) in [
        (
            "C:/Users/Alice/AppData/Roaming/Microsoft/Credentials",
            ProtectedCategory::Credentials,
        ),
        (
            "C:/Users/Alice/AppData/Roaming/Clash Verge",
            ProtectedCategory::VpnProxyState,
        ),
        (
            "C:/Users/Alice/AppData/Roaming/Cursor/User",
            ProtectedCategory::AiToolDurableState,
        ),
        (
            "C:/Users/Alice/OneDrive/Documents",
            ProtectedCategory::CloudSyncedData,
        ),
        (
            "C:/Users/Alice/AppData/Local/Docker",
            ProtectedCategory::ContainerRuntimeState,
        ),
        (
            "C:/Users/Alice/AppData/Roaming/Microsoft/Windows/Start Menu/Programs/Startup",
            ProtectedCategory::StartupAutomation,
        ),
        (
            "C:/Users/Alice/AppData/Roaming/Steam/userdata/12345/config.vdf",
            ProtectedCategory::ApplicationDurableData,
        ),
    ] {
        assert!(
            matches!(
                policy.assess_path(&PathBuf::from(path)),
                ProtectionAssessment::Blocked(block)
                    if block.kind == ProtectionBlockKind::ProtectedCategory(expected_category)
            ),
            "{path} should be blocked"
        );
    }
}

#[test]
fn rebecca_owned_storage_is_blocked_even_when_the_path_exists_only_logically() {
    let app_paths = AppPaths {
        config_dir: PathBuf::from("C:/Users/Alice/AppData/Roaming/Rebecca"),
        config_file: PathBuf::from("C:/Users/Alice/AppData/Roaming/Rebecca/config.toml"),
        state_dir: PathBuf::from("C:/Users/Alice/AppData/Local/Rebecca/state"),
        cache_dir: PathBuf::from("C:/Users/Alice/AppData/Local/Rebecca/cache"),
        history_file: PathBuf::from("C:/Users/Alice/AppData/Local/Rebecca/state/history.jsonl"),
    };
    let storage_entries = app_paths.storage_entries();
    let policy = ProtectionPolicy::new().with_protected_storage(&storage_entries);

    assert!(matches!(
        policy.assess_path(&app_paths.cache_dir.join("scan")),
        ProtectionAssessment::Blocked(block)
            if block.kind == ProtectionBlockKind::RebeccaOwnedStorage
    ));
    assert!(matches!(
        policy.assess_path(&app_paths.history_file),
        ProtectionAssessment::Blocked(block)
            if block.kind == ProtectionBlockKind::RebeccaOwnedStorage
    ));
}

#[test]
fn maintenance_allowlists_keep_known_cache_paths_open() {
    let policy = ProtectionPolicy::new();

    for path in [
        "C:/Users/Alice/AppData/Local/pip/Cache",
        "C:/Users/Alice/AppData/Local/JetBrains/RustRover2024.3/caches",
        "C:/Users/Alice/AppData/Roaming/Code/Cache",
        "C:/Users/Alice/AppData/Roaming/discord/GPUCache",
        "C:/Users/Alice/AppData/Local/Steam/htmlcache/Default/Cache",
        "C:/Users/Alice/AppData/Local/Temp/rebecca-test",
    ] {
        assert!(
            matches!(
                policy.assess_path(&PathBuf::from(path)),
                ProtectionAssessment::Allowed
            ),
            "{path} should remain allowed"
        );
    }
}

#[test]
fn catalog_target_shapes_keep_known_maintenance_targets_open() {
    let policy = ProtectionPolicy::new();

    for target in [
        RuleTargetSpec::template("%LOCALAPPDATA%\\Google\\Chrome\\User Data\\Default\\Cache"),
        RuleTargetSpec::glob_template("%APPDATA%\\Mozilla\\Firefox\\Profiles\\*\\cache2"),
        RuleTargetSpec::steam_install_template("appcache\\httpcache"),
        RuleTargetSpec::steam_install_template("logs"),
        RuleTargetSpec::steam_library_template("steamapps\\downloading"),
    ] {
        assert!(
            matches!(
                policy.assess_catalog_target_shape(&target),
                ProtectionAssessment::Allowed
            ),
            "{target:?} should be an allowed catalog target shape"
        );
    }
}

#[test]
fn catalog_target_shapes_reject_protected_categories_and_unsafe_steam_targets() {
    let policy = ProtectionPolicy::new();

    for (target, expected_category) in [
        (
            RuleTargetSpec::template("%USERPROFILE%\\.ssh"),
            ProtectedCategory::Credentials,
        ),
        (
            RuleTargetSpec::glob_template(
                "%APPDATA%\\Mozilla\\Firefox\\Profiles\\*\\cookies.sqlite",
            ),
            ProtectedCategory::BrowserPrivateData,
        ),
        (
            RuleTargetSpec::template(
                "%LOCALAPPDATA%\\Google\\Chrome\\User Data\\Default\\Local Storage",
            ),
            ProtectedCategory::BrowserPrivateData,
        ),
        (
            RuleTargetSpec::steam_install_template("userdata"),
            ProtectedCategory::ApplicationDurableData,
        ),
        (
            RuleTargetSpec::steam_library_template("steamapps\\common"),
            ProtectedCategory::ApplicationDurableData,
        ),
        (
            RuleTargetSpec::steam_library_template("steamapps\\workshop"),
            ProtectedCategory::ApplicationDurableData,
        ),
    ] {
        assert!(
            matches!(
                policy.assess_catalog_target_shape(&target),
                ProtectionAssessment::Blocked(block)
                    if block.kind == ProtectionBlockKind::ProtectedCategory(expected_category)
            ),
            "{target:?} should be blocked as {expected_category:?}"
        );
    }
}

use rebecca_core::Platform;
use rebecca_core::safety_catalog::{
    SafetyCategory, default_safety_knowledge, parse_safety_catalog_file,
};

#[test]
fn default_safety_catalog_loads_auditable_windows_knowledge() {
    let knowledge = default_safety_knowledge();

    assert_eq!(knowledge.platform(), Platform::Windows);
    assert!(
        knowledge
            .warning_kinds()
            .iter()
            .any(|warning| warning.id() == "active-process")
    );
    assert_eq!(
        knowledge.category_description(SafetyCategory::Credentials),
        Some("credential and password-manager data")
    );
    assert!(
        knowledge
            .critical_path_prefixes()
            .iter()
            .any(|prefix| prefix == "c:/windows")
    );
    assert!(knowledge.is_allowed_steam_install_target("appcache/httpcache"));
    assert!(knowledge.is_allowed_steam_library_target("steamapps/shadercache"));
}

#[test]
fn safety_catalog_rejects_unsupported_version() {
    let err = parse_safety_catalog_file(
        "test.toml",
        r#"
catalog_version = 2
platform = "windows"
critical_path_prefixes = ["C:/Windows"]
"#,
    )
    .unwrap_err();

    assert!(
        err.to_string()
            .contains("unsupported safety catalog version")
    );
}

#[test]
fn safety_catalog_rejects_duplicate_warning_kinds() {
    let err = parse_safety_catalog_file(
        "test.toml",
        r#"
catalog_version = 1
platform = "windows"
critical_path_prefixes = ["C:/Windows"]

[[warning_kinds]]
id = "active-process"
description = "one"

[[warning_kinds]]
id = "ACTIVE-PROCESS"
description = "two"
"#,
    )
    .unwrap_err();

    assert!(
        err.to_string()
            .contains("warning_kinds.id contains duplicate")
    );
}

#[test]
fn safety_catalog_requires_complete_category_descriptions() {
    let err = parse_safety_catalog_file(
        "test.toml",
        r#"
catalog_version = 1
platform = "windows"
critical_path_prefixes = ["C:/Windows"]

[[protected_categories]]
id = "credentials"
description = "credential data"
"#,
    )
    .unwrap_err();

    assert!(
        err.to_string()
            .contains("missing protected category vpn-proxy-state")
    );
}

#[test]
fn safety_catalog_pattern_sets_match_segments_and_sequences() {
    let knowledge = default_safety_knowledge();

    let credential_pattern = knowledge
        .protected_patterns()
        .iter()
        .find(|pattern| pattern.category() == SafetyCategory::Credentials)
        .expect("credentials pattern should exist");
    assert!(credential_pattern.matches(&["c:", "users", "alice", ".ssh"]));
    assert!(credential_pattern.matches(&[
        "c:",
        "users",
        "alice",
        "appdata",
        "roaming",
        "microsoft",
        "credentials",
    ]));

    assert!(
        knowledge
            .maintenance_allowlist()
            .matches(&["c:", "users", "alice", ".gradle", "caches"])
    );
}

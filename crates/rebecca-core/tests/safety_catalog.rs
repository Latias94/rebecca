use rebecca_core::Platform;
use rebecca_core::safety_catalog::{
    SafetyCategory, default_safety_catalog, default_safety_knowledge,
    default_safety_knowledge_for_platform, parse_safety_catalog_file,
};

#[test]
fn default_safety_catalog_loads_auditable_platform_knowledge() {
    let catalog = default_safety_catalog();
    let knowledge = default_safety_knowledge();

    assert_eq!(catalog.default_platform(), Platform::Windows);
    assert!(catalog.knowledge_for_platform(Platform::Linux).is_some());
    assert!(catalog.knowledge_for_platform(Platform::Macos).is_some());
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

    let linux = default_safety_knowledge_for_platform(Platform::Linux)
        .expect("Linux safety knowledge should load");
    assert!(
        linux
            .critical_path_prefixes()
            .iter()
            .any(|prefix| prefix == "/etc")
    );
    assert!(
        linux
            .maintenance_allowlist()
            .matches(&["home", "alice", ".cache", "thumbnails"])
    );
    assert!(
        linux
            .maintenance_allowlist()
            .matches(&["var", "cache", "apt", "archives"])
    );
    assert!(linux.protected_patterns().iter().any(|pattern| {
        pattern.category() == SafetyCategory::ApplicationDurableData
            && pattern.matches(&["home", "alice", ".config", "code", "user"])
    }));

    let macos = default_safety_knowledge_for_platform(Platform::Macos)
        .expect("macOS safety knowledge should load");
    assert!(
        macos
            .critical_path_prefixes()
            .iter()
            .any(|prefix| prefix == "/system")
    );
}

#[test]
fn safety_catalog_rejects_unsupported_version() {
    let err = parse_safety_catalog_file(
        "test.toml",
        r#"
catalog_version = 2
default_platform = "windows"

[[platforms]]
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
default_platform = "windows"

[[platforms]]
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
default_platform = "windows"

[[platforms]]
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

#[test]
fn linux_safety_catalog_covers_cache_and_durable_state_boundaries() {
    let knowledge = default_safety_knowledge_for_platform(Platform::Linux)
        .expect("Linux safety knowledge should load");

    for segments in [
        &["home", "alice", ".cache", "go-build"][..],
        &["home", "alice", ".local", "share", "pnpm", "store"][..],
        &["var", "cache", "apt", "archives"][..],
    ] {
        assert!(
            knowledge.maintenance_allowlist().matches(segments),
            "{segments:?} should be an allowlisted Linux maintenance shape"
        );
    }

    let protected_category = |segments: &[&str]| {
        knowledge
            .protected_patterns()
            .iter()
            .find(|pattern| pattern.matches(segments))
            .map(|pattern| pattern.category())
    };

    assert_eq!(
        protected_category(&["home", "alice", ".local", "share", "keyrings"]),
        Some(SafetyCategory::Credentials)
    );
    assert_eq!(
        protected_category(&["home", "alice", ".config", "autostart"]),
        Some(SafetyCategory::StartupAutomation)
    );
    assert_eq!(
        protected_category(&["home", "alice", ".config", "slack"]),
        Some(SafetyCategory::ApplicationDurableData)
    );
    assert_eq!(
        protected_category(&["home", "alice", ".var", "app", "com.slack.slack", "config"]),
        Some(SafetyCategory::ApplicationDurableData)
    );
}

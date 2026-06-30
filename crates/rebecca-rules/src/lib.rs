use rebecca_core::{
    Platform, RebeccaError, Result, RuleDefinition, RuleSource,
    manifest::parse_cleaner_manifest_file_with_safety_knowledge,
    planner::validate_rule_catalog,
    protection::{ProtectionAssessment, ProtectionPolicy},
    safety_catalog::{SafetyKnowledge, parse_safety_catalog_file},
};

macro_rules! builtin_rule_files {
    ($($path:literal),+ $(,)?) => {
        &[
            $(
                (
                    $path,
                    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/", $path)),
                ),
            )+
        ]
    };
}

const BUILTIN_RULE_FILES: &[(&str, &str)] = builtin_rule_files!(
    "rules/windows/user-temp.toml",
    "rules/windows/edge-cache.toml",
    "rules/windows/firefox-profile-cache.toml",
    "rules/windows/chrome-cache.toml",
    "rules/windows/brave-cache.toml",
    "rules/windows/directx-shader-cache.toml",
    "rules/windows/discord-cache.toml",
    "rules/windows/wechat-cache.toml",
    "rules/windows/wxwork-cache.toml",
    "rules/windows/qq-cache.toml",
    "rules/windows/feishu-cache.toml",
    "rules/windows/dingtalk-cache.toml",
    "rules/windows/wps-cache.toml",
    "rules/windows/baidunetdisk-cache.toml",
    "rules/windows/tencent-meeting-cache.toml",
    "rules/windows/qqmusic-cache.toml",
    "rules/windows/tencent-video-cache.toml",
    "rules/windows/postman-cache.toml",
    "rules/windows/notion-cache.toml",
    "rules/windows/figma-cache.toml",
    "rules/windows/slack-cache.toml",
    "rules/windows/steam-cache.toml",
    "rules/windows/steam-install-cache.toml",
    "rules/windows/steam-install-depot-cache.toml",
    "rules/windows/steam-install-logs.toml",
    "rules/windows/steam-install-avatar-cache.toml",
    "rules/windows/steam-install-stats-cache.toml",
    "rules/windows/steam-install-appinfo-cache.toml",
    "rules/windows/steam-install-localization-cache.toml",
    "rules/windows/steam-install-packageinfo-cache.toml",
    "rules/windows/steam-install-download-cache.toml",
    "rules/windows/steam-install-library-cache.toml",
    "rules/windows/steam-install-shader-cache.toml",
    "rules/windows/steam-library-shader-cache.toml",
    "rules/windows/steam-library-downloading-cache.toml",
    "rules/windows/steam-library-temp-cache.toml",
    "rules/windows/npm-cache.toml",
    "rules/windows/pnpm-cache.toml",
    "rules/windows/yarn-cache.toml",
    "rules/windows/bun-cache.toml",
    "rules/windows/corepack-cache.toml",
    "rules/windows/gradle-cache.toml",
    "rules/windows/android-cache.toml",
    "rules/windows/nuget-cache.toml",
    "rules/windows/maven-cache.toml",
    "rules/windows/pip-cache.toml",
    "rules/windows/uv-cache.toml",
    "rules/windows/poetry-cache.toml",
    "rules/windows/conda-cache.toml",
    "rules/windows/go-build-cache.toml",
    "rules/windows/go-module-cache.toml",
    "rules/windows/cargo-cache.toml",
    "rules/windows/rustup-cache.toml",
    "rules/windows/ccache-cache.toml",
    "rules/windows/sccache-cache.toml",
    "rules/windows/huggingface-cache.toml",
    "rules/windows/pytorch-cache.toml",
    "rules/windows/jetbrains-cache.toml",
    "rules/windows/thumbnail-cache.toml",
    "rules/windows/vscode-cache.toml",
    "rules/windows/wer-reports.toml",
    "rules/windows/system-temp.toml",
    "rules/windows/prefetch.toml",
    "rules/windows/update-download-cache.toml",
    "rules/windows/media-player-cache.toml",
);

const BUILTIN_SAFETY_CATALOG: (&str, &str) = (
    "safety/windows.toml",
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/safety/windows.toml")),
);

pub fn builtin_rules() -> Result<Vec<RuleDefinition>> {
    let mut rules = Vec::with_capacity(BUILTIN_RULE_FILES.len());
    let safety_knowledge = builtin_safety_knowledge()?;

    for (path, raw) in BUILTIN_RULE_FILES {
        rules.extend(parse_rule_file(path, raw, &safety_knowledge)?);
    }

    validate_builtin_rule_catalog(&rules)?;
    validate_rule_catalog(&rules)?;
    Ok(rules)
}

pub fn builtin_safety_knowledge() -> Result<SafetyKnowledge> {
    parse_safety_catalog_file(BUILTIN_SAFETY_CATALOG.0, BUILTIN_SAFETY_CATALOG.1)
}

pub fn validate_builtin_rules() -> Result<()> {
    builtin_rules().map(|_| ())
}

fn parse_rule_file(
    path: &str,
    raw: &str,
    safety_knowledge: &SafetyKnowledge,
) -> Result<Vec<RuleDefinition>> {
    parse_cleaner_manifest_file_with_safety_knowledge(path, raw, safety_knowledge)
}

fn validate_builtin_rule_catalog(rules: &[RuleDefinition]) -> Result<()> {
    let safety_knowledge = builtin_safety_knowledge()?;
    let policy = ProtectionPolicy::new().with_safety_knowledge(&safety_knowledge);

    for rule in rules {
        if rule.platform != Platform::Windows {
            return Err(RebeccaError::RuleCatalogInvalid(format!(
                "built-in rule {} must target the Windows platform",
                rule.id
            )));
        }

        if !rule.id.starts_with("windows.") {
            return Err(RebeccaError::RuleCatalogInvalid(format!(
                "built-in rule {} must use a windows. rule id prefix",
                rule.id
            )));
        }

        if rule
            .restore_hint
            .as_deref()
            .map(str::trim)
            .unwrap_or_default()
            .is_empty()
        {
            return Err(RebeccaError::RuleCatalogInvalid(format!(
                "built-in rule {} must define a restore hint",
                rule.id
            )));
        }

        if rule.provenance.source != RuleSource::Owned {
            return Err(RebeccaError::RuleCatalogInvalid(format!(
                "built-in rule {} must use owned provenance source",
                rule.id
            )));
        }

        if rule.provenance.license.trim() != "project-owned" {
            return Err(RebeccaError::RuleCatalogInvalid(format!(
                "built-in rule {} must use project-owned provenance license",
                rule.id
            )));
        }

        for spec in &rule.path_templates {
            if let ProtectionAssessment::Blocked(block) = policy.assess_catalog_target_shape(spec) {
                return Err(RebeccaError::RuleCatalogInvalid(format!(
                    "built-in rule {} target {} is blocked by {}: {}",
                    rule.id,
                    spec.placeholder_path().display(),
                    block.kind.label(),
                    block.message
                )));
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use rebecca_core::safety_catalog::default_safety_knowledge;
    use rebecca_core::{
        Platform, RuleDefinition, RuleProvenance, RuleSource, RuleTargetSpec, SafetyLevel,
    };

    use super::{builtin_rules, builtin_safety_knowledge, parse_rule_file};

    fn parse_single_rule_file(path: &str, raw: &str) -> RuleDefinition {
        let safety_knowledge =
            builtin_safety_knowledge().expect("built-in safety catalog should load");
        let rules = parse_rule_file(path, raw, &safety_knowledge).expect("test rule should parse");
        assert_eq!(rules.len(), 1);
        rules.into_iter().next().unwrap()
    }

    #[test]
    fn builtin_rule_ids_are_unique() {
        let rules = builtin_rules().expect("built-in rules should load");
        let ids = rules
            .iter()
            .map(|rule| rule.id.as_str())
            .collect::<HashSet<_>>();

        assert_eq!(ids.len(), rules.len());
    }

    #[test]
    fn builtin_rules_have_required_metadata() {
        super::validate_builtin_rules().expect("built-in rules should be valid");
    }

    #[test]
    fn builtin_safety_catalog_exposes_warning_and_category_knowledge() {
        let knowledge = builtin_safety_knowledge().expect("built-in safety catalog should load");

        assert!(
            knowledge
                .warning_kinds()
                .iter()
                .any(|warning| warning.id() == "active-process")
        );
        assert!(
            knowledge
                .categories()
                .iter()
                .any(|category| category.id().label() == "application-durable-data")
        );
        assert!(knowledge.is_allowed_steam_install_target("appcache/httpcache"));
        assert!(knowledge.is_allowed_steam_library_target("steamapps/downloading"));
    }

    #[test]
    fn builtin_safety_catalog_matches_core_default_catalog() {
        let builtin = builtin_safety_knowledge().expect("built-in safety catalog should load");
        let core_default = default_safety_knowledge();

        assert_eq!(builtin.platform(), core_default.platform());
        assert_eq!(
            builtin
                .warning_kinds()
                .iter()
                .map(|warning| warning.id())
                .collect::<Vec<_>>(),
            core_default
                .warning_kinds()
                .iter()
                .map(|warning| warning.id())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            builtin.critical_path_prefixes(),
            core_default.critical_path_prefixes()
        );
    }

    #[test]
    fn builtin_rules_use_owned_provenance_sources() {
        let rules = builtin_rules().expect("built-in rules should load");

        assert!(
            rules
                .iter()
                .all(|rule| rule.provenance.source == rebecca_core::RuleSource::Owned)
        );
    }

    #[test]
    fn builtin_rules_have_restore_hints_and_project_owned_provenance() {
        let rules = builtin_rules().expect("built-in rules should load");

        assert!(rules.iter().all(|rule| {
            rule.restore_hint
                .as_deref()
                .map(str::trim)
                .is_some_and(|hint| !hint.is_empty())
                && rule.provenance.license == "project-owned"
        }));
    }

    #[test]
    fn builtin_rules_are_loaded_from_toml_catalog_files() {
        let rules = builtin_rules().expect("built-in rules should load");
        let user_temp = rules
            .iter()
            .find(|rule| rule.id == "windows.user-temp")
            .expect("user temp rule should exist");

        assert_eq!(user_temp.platform, rebecca_core::Platform::Windows);
        assert_eq!(user_temp.category, "system");
        assert_eq!(user_temp.path_templates.len(), 2);
        assert_eq!(user_temp.provenance.source, RuleSource::Owned);
    }

    #[test]
    fn builtin_catalog_rejects_non_owned_provenance_sources() {
        let err = super::validate_builtin_rule_catalog(&[RuleDefinition {
            id: "windows.test".to_string(),
            platform: Platform::Windows,
            category: "system".to_string(),
            name: "Test".to_string(),
            safety_level: SafetyLevel::Safe,
            path_templates: vec![RuleTargetSpec::template("%TEMP%")],
            restore_hint: Some("Regenerated automatically.".to_string()),
            warnings: Vec::new(),
            provenance: RuleProvenance {
                source: RuleSource::ReferenceOnly,
                license: "project-owned".to_string(),
                notes: "test".to_string(),
            },
        }])
        .unwrap_err();

        assert!(err.to_string().contains("owned provenance source"));
    }

    #[test]
    fn builtin_rules_include_first_expansion_batch() {
        let rules = builtin_rules().expect("built-in rules should load");
        let ids = rules
            .iter()
            .map(|rule| rule.id.as_str())
            .collect::<HashSet<_>>();

        for expected in [
            "windows.chrome-cache",
            "windows.android-cache",
            "windows.brave-cache",
            "windows.bun-cache",
            "windows.cargo-cache",
            "windows.ccache-cache",
            "windows.corepack-cache",
            "windows.gradle-cache",
            "windows.directx-shader-cache",
            "windows.discord-cache",
            "windows.wechat-cache",
            "windows.wxwork-cache",
            "windows.qq-cache",
            "windows.feishu-cache",
            "windows.dingtalk-cache",
            "windows.wps-cache",
            "windows.baidunetdisk-cache",
            "windows.tencent-meeting-cache",
            "windows.qqmusic-cache",
            "windows.tencent-video-cache",
            "windows.figma-cache",
            "windows.firefox-profile-cache",
            "windows.go-build-cache",
            "windows.go-module-cache",
            "windows.huggingface-cache",
            "windows.maven-cache",
            "windows.jetbrains-cache",
            "windows.nuget-cache",
            "windows.notion-cache",
            "windows.pip-cache",
            "windows.poetry-cache",
            "windows.conda-cache",
            "windows.postman-cache",
            "windows.pytorch-cache",
            "windows.pnpm-cache",
            "windows.rustup-cache",
            "windows.sccache-cache",
            "windows.slack-cache",
            "windows.steam-cache",
            "windows.steam-install-cache",
            "windows.steam-install-download-cache",
            "windows.steam-install-library-cache",
            "windows.steam-install-shader-cache",
            "windows.steam-install-logs",
            "windows.steam-install-avatar-cache",
            "windows.steam-install-stats-cache",
            "windows.steam-install-appinfo-cache",
            "windows.steam-install-localization-cache",
            "windows.steam-install-packageinfo-cache",
            "windows.steam-library-downloading-cache",
            "windows.steam-library-shader-cache",
            "windows.steam-library-temp-cache",
            "windows.thumbnail-cache",
            "windows.uv-cache",
            "windows.vscode-cache",
            "windows.wer-reports",
            "windows.yarn-cache",
            "windows.system-temp",
            "windows.prefetch",
            "windows.update-download-cache",
            "windows.media-player-cache",
        ] {
            assert!(ids.contains(expected), "missing built-in rule: {expected}");
        }
    }

    #[test]
    fn catalog_parser_rejects_unknown_fields() {
        let safety_knowledge =
            builtin_safety_knowledge().expect("built-in safety catalog should load");
        let err = parse_rule_file(
            "test.toml",
            r#"
manifest_version = 1
id = "windows.test"
platform = "windows"
category = "system"
name = "Test"
safety_level = "safe"
unexpected = "field"

[[targets]]
kind = "template"
value = "%TEMP%"

[provenance]
source = "owned"
license = "project-owned"
notes = "test"
"#,
            &safety_knowledge,
        )
        .unwrap_err();

        assert!(err.to_string().contains("unknown field"));
    }

    #[test]
    fn catalog_parser_supports_exact_path_targets() {
        let rule = parse_single_rule_file(
            "test.toml",
            r#"
manifest_version = 1
id = "windows.exact"
platform = "windows"
category = "system"
name = "Exact"
safety_level = "safe"

[[targets]]
kind = "exact-path"
value = "C:\\Users\\Example\\Cache"

[provenance]
source = "owned"
license = "project-owned"
notes = "test"
"#,
        );

        assert_eq!(rule.path_templates.len(), 1);
        assert!(matches!(
            rule.path_templates[0],
            rebecca_core::RuleTargetSpec::ExactPath(_)
        ));
    }

    #[test]
    fn catalog_parser_supports_glob_template_targets() {
        let rule = parse_single_rule_file(
            "test.toml",
            r#"
manifest_version = 1
id = "windows.glob"
platform = "windows"
category = "browser"
name = "Glob"
safety_level = "safe"

[[targets]]
kind = "glob-template"
value = "%APPDATA%\\Mozilla\\Firefox\\Profiles\\*\\cache2"

[provenance]
source = "owned"
license = "project-owned"
notes = "test"
"#,
        );

        assert!(matches!(
            rule.path_templates[0],
            rebecca_core::RuleTargetSpec::GlobTemplate(_)
        ));
    }

    #[test]
    fn catalog_parser_supports_steam_discovery_targets() {
        let rule = parse_single_rule_file(
            "test.toml",
            r#"
manifest_version = 1
id = "windows.steam-test"
platform = "windows"
category = "application"
name = "Steam test"
safety_level = "safe"

[[targets]]
kind = "steam-install-template"
value = "appcache\\httpcache"

[[targets]]
kind = "steam-library-template"
value = "steamapps\\shadercache"

[provenance]
source = "owned"
license = "project-owned"
notes = "test"
"#,
        );

        assert!(matches!(
            rule.path_templates[0],
            rebecca_core::RuleTargetSpec::SteamInstallTemplate(_)
        ));
        assert!(matches!(
            rule.path_templates[1],
            rebecca_core::RuleTargetSpec::SteamLibraryTemplate(_)
        ));
    }

    #[test]
    fn builtin_catalog_rejects_missing_restore_hints() {
        let err = super::validate_builtin_rule_catalog(&[RuleDefinition {
            id: "windows.test".to_string(),
            platform: Platform::Windows,
            category: "system".to_string(),
            name: "Test".to_string(),
            safety_level: SafetyLevel::Safe,
            path_templates: vec![RuleTargetSpec::template("%TEMP%")],
            restore_hint: None,
            warnings: Vec::new(),
            provenance: RuleProvenance {
                source: RuleSource::Owned,
                license: "project-owned".to_string(),
                notes: "test".to_string(),
            },
        }])
        .unwrap_err();

        assert!(err.to_string().contains("restore hint"));
    }

    #[test]
    fn builtin_catalog_rejects_non_project_owned_licenses() {
        let err = super::validate_builtin_rule_catalog(&[RuleDefinition {
            id: "windows.test".to_string(),
            platform: Platform::Windows,
            category: "system".to_string(),
            name: "Test".to_string(),
            safety_level: SafetyLevel::Safe,
            path_templates: vec![RuleTargetSpec::template("%TEMP%")],
            restore_hint: Some("Regenerated automatically.".to_string()),
            warnings: Vec::new(),
            provenance: RuleProvenance {
                source: RuleSource::Owned,
                license: "reference-only".to_string(),
                notes: "test".to_string(),
            },
        }])
        .unwrap_err();

        assert!(err.to_string().contains("project-owned provenance license"));
    }

    #[test]
    fn catalog_parser_rejects_non_windows_platforms() {
        let safety_knowledge =
            builtin_safety_knowledge().expect("built-in safety catalog should load");
        let err = parse_rule_file(
            "test.toml",
            r#"
manifest_version = 1
id = "linux.test"
platform = "linux"
category = "system"
name = "Test"
safety_level = "safe"

[[targets]]
kind = "template"
value = "/tmp"

[provenance]
source = "owned"
license = "project-owned"
notes = "test"
"#,
            &safety_knowledge,
        )
        .unwrap_err();

        let message = err.to_string();
        assert!(
            message.contains("unknown variant") || message.contains("invalid value"),
            "{message}"
        );
    }

    #[test]
    fn builtin_catalog_rejects_non_windows_id_prefixes() {
        let err = super::validate_builtin_rule_catalog(&[RuleDefinition {
            id: "linux.test".to_string(),
            platform: Platform::Windows,
            category: "system".to_string(),
            name: "Test".to_string(),
            safety_level: SafetyLevel::Safe,
            path_templates: vec![RuleTargetSpec::template("%TEMP%")],
            restore_hint: Some("Regenerated automatically.".to_string()),
            warnings: Vec::new(),
            provenance: RuleProvenance {
                source: RuleSource::Owned,
                license: "project-owned".to_string(),
                notes: "test".to_string(),
            },
        }])
        .unwrap_err();

        assert!(err.to_string().contains("windows. rule id prefix"));
    }

    #[test]
    fn builtin_catalog_rejects_protected_target_shapes() {
        for (target, expected) in [
            (
                RuleTargetSpec::template("%USERPROFILE%\\.ssh"),
                "credentials",
            ),
            (
                RuleTargetSpec::template(
                    "%LOCALAPPDATA%\\Google\\Chrome\\User Data\\Default\\History",
                ),
                "browser-private-data",
            ),
            (
                RuleTargetSpec::steam_install_template("userdata"),
                "application-durable-data",
            ),
            (
                RuleTargetSpec::steam_library_template("steamapps\\common"),
                "application-durable-data",
            ),
        ] {
            let err = super::validate_builtin_rule_catalog(&[rule_with_target(target)])
                .expect_err("protected target shape should be rejected");
            assert!(
                err.to_string().contains(expected),
                "{err} should mention {expected}"
            );
        }
    }

    fn rule_with_target(target: RuleTargetSpec) -> RuleDefinition {
        RuleDefinition {
            id: "windows.test".to_string(),
            platform: Platform::Windows,
            category: "system".to_string(),
            name: "Test".to_string(),
            safety_level: SafetyLevel::Safe,
            path_templates: vec![target],
            restore_hint: Some("Regenerated automatically.".to_string()),
            warnings: Vec::new(),
            provenance: RuleProvenance {
                source: RuleSource::Owned,
                license: "project-owned".to_string(),
                notes: "test".to_string(),
            },
        }
    }
}

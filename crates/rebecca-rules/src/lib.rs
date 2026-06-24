use std::path::PathBuf;

use rebecca_core::{
    DeletePolicy, Platform, RebeccaError, Result, RuleDefinition, RuleProvenance, RuleSource,
    RuleTargetSpec, SafetyLevel,
    planner::validate_rule_catalog,
    protection::{ProtectionAssessment, ProtectionPolicy},
};
use serde::Deserialize;

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
    "rules/windows/directx-shader-cache.toml",
    "rules/windows/discord-cache.toml",
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
    "rules/windows/pip-cache.toml",
    "rules/windows/cargo-cache.toml",
    "rules/windows/jetbrains-cache.toml",
    "rules/windows/thumbnail-cache.toml",
    "rules/windows/vscode-cache.toml",
    "rules/windows/wer-reports.toml",
);

pub fn builtin_rules() -> Result<Vec<RuleDefinition>> {
    let mut rules = Vec::with_capacity(BUILTIN_RULE_FILES.len());

    for (path, raw) in BUILTIN_RULE_FILES {
        rules.push(parse_rule_file(path, raw)?);
    }

    validate_builtin_rule_catalog(&rules)?;
    validate_rule_catalog(&rules)?;
    Ok(rules)
}

pub fn validate_builtin_rules() -> Result<()> {
    builtin_rules().map(|_| ())
}

fn parse_rule_file(path: &str, raw: &str) -> Result<RuleDefinition> {
    let rule = toml::from_str::<CatalogRule>(raw).map_err(|err| {
        RebeccaError::RuleCatalogInvalid(format!("{path} is invalid TOML catalog data: {err}"))
    })?;

    Ok(rule.into_rule_definition())
}

fn validate_builtin_rule_catalog(rules: &[RuleDefinition]) -> Result<()> {
    let policy = ProtectionPolicy::new();

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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogRule {
    id: String,
    platform: Platform,
    category: String,
    name: String,
    safety_level: SafetyLevel,
    delete_policy: DeletePolicy,
    restore_hint: Option<String>,
    targets: Vec<CatalogTarget>,
    provenance: RuleProvenance,
}

impl CatalogRule {
    fn into_rule_definition(self) -> RuleDefinition {
        RuleDefinition {
            id: self.id,
            platform: self.platform,
            category: self.category,
            name: self.name,
            safety_level: self.safety_level,
            path_templates: self
                .targets
                .into_iter()
                .map(CatalogTarget::into_rule_target_spec)
                .collect(),
            delete_policy: self.delete_policy,
            restore_hint: self.restore_hint,
            provenance: self.provenance,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
enum CatalogTarget {
    Template { value: String },
    ExactPath { value: PathBuf },
    GlobTemplate { value: String },
    SteamInstallTemplate { value: String },
    SteamLibraryTemplate { value: String },
}

impl CatalogTarget {
    fn into_rule_target_spec(self) -> RuleTargetSpec {
        match self {
            Self::Template { value } => RuleTargetSpec::template(value),
            Self::ExactPath { value } => RuleTargetSpec::ExactPath(value),
            Self::GlobTemplate { value } => RuleTargetSpec::glob_template(value),
            Self::SteamInstallTemplate { value } => RuleTargetSpec::steam_install_template(value),
            Self::SteamLibraryTemplate { value } => RuleTargetSpec::steam_library_template(value),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use rebecca_core::{
        DeletePolicy, Platform, RuleDefinition, RuleProvenance, RuleSource, RuleTargetSpec,
        SafetyLevel,
    };

    use super::{builtin_rules, parse_rule_file};

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
            delete_policy: DeletePolicy::RecycleBin,
            restore_hint: Some("Regenerated automatically.".to_string()),
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
            "windows.cargo-cache",
            "windows.directx-shader-cache",
            "windows.discord-cache",
            "windows.firefox-profile-cache",
            "windows.jetbrains-cache",
            "windows.pip-cache",
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
            "windows.vscode-cache",
            "windows.wer-reports",
        ] {
            assert!(ids.contains(expected), "missing built-in rule: {expected}");
        }
    }

    #[test]
    fn catalog_parser_rejects_unknown_fields() {
        let err = parse_rule_file(
            "test.toml",
            r#"
id = "windows.test"
platform = "windows"
category = "system"
name = "Test"
safety_level = "safe"
delete_policy = "recycle-bin"
unexpected = "field"

[[targets]]
kind = "template"
value = "%TEMP%"

[provenance]
source = "owned"
license = "project-owned"
notes = "test"
"#,
        )
        .unwrap_err();

        assert!(err.to_string().contains("unknown field"));
    }

    #[test]
    fn catalog_parser_supports_exact_path_targets() {
        let rule = parse_rule_file(
            "test.toml",
            r#"
id = "windows.exact"
platform = "windows"
category = "system"
name = "Exact"
safety_level = "safe"
delete_policy = "recycle-bin"

[[targets]]
kind = "exact-path"
value = "C:\\Users\\Example\\Cache"

[provenance]
source = "owned"
license = "project-owned"
notes = "test"
"#,
        )
        .expect("exact-path target should parse");

        assert_eq!(rule.path_templates.len(), 1);
        assert!(matches!(
            rule.path_templates[0],
            rebecca_core::RuleTargetSpec::ExactPath(_)
        ));
    }

    #[test]
    fn catalog_parser_supports_glob_template_targets() {
        let rule = parse_rule_file(
            "test.toml",
            r#"
id = "windows.glob"
platform = "windows"
category = "browser"
name = "Glob"
safety_level = "safe"
delete_policy = "recycle-bin"

[[targets]]
kind = "glob-template"
value = "%APPDATA%\\Mozilla\\Firefox\\Profiles\\*\\cache2"

[provenance]
source = "owned"
license = "project-owned"
notes = "test"
"#,
        )
        .expect("glob-template target should parse");

        assert!(matches!(
            rule.path_templates[0],
            rebecca_core::RuleTargetSpec::GlobTemplate(_)
        ));
    }

    #[test]
    fn catalog_parser_supports_steam_discovery_targets() {
        let rule = parse_rule_file(
            "test.toml",
            r#"
id = "windows.steam-test"
platform = "windows"
category = "application"
name = "Steam test"
safety_level = "safe"
delete_policy = "recycle-bin"

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
        )
        .expect("Steam discovery targets should parse");

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
            delete_policy: DeletePolicy::RecycleBin,
            restore_hint: None,
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
            delete_policy: DeletePolicy::RecycleBin,
            restore_hint: Some("Regenerated automatically.".to_string()),
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
    fn builtin_catalog_rejects_non_windows_platforms() {
        let err = super::validate_builtin_rule_catalog(&[RuleDefinition {
            id: "linux.test".to_string(),
            platform: Platform::Linux,
            category: "system".to_string(),
            name: "Test".to_string(),
            safety_level: SafetyLevel::Safe,
            path_templates: vec![RuleTargetSpec::template("/tmp")],
            delete_policy: DeletePolicy::RecycleBin,
            restore_hint: Some("Regenerated automatically.".to_string()),
            provenance: RuleProvenance {
                source: RuleSource::Owned,
                license: "project-owned".to_string(),
                notes: "test".to_string(),
            },
        }])
        .unwrap_err();

        assert!(err.to_string().contains("Windows platform"));
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
            delete_policy: DeletePolicy::RecycleBin,
            restore_hint: Some("Regenerated automatically.".to_string()),
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
            delete_policy: DeletePolicy::RecycleBin,
            restore_hint: Some("Regenerated automatically.".to_string()),
            provenance: RuleProvenance {
                source: RuleSource::Owned,
                license: "project-owned".to_string(),
                notes: "test".to_string(),
            },
        }
    }
}

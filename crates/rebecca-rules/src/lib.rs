use std::path::PathBuf;

use rebecca_core::{
    DeletePolicy, Platform, RebeccaError, Result, RuleDefinition, RuleProvenance, RuleTargetSpec,
    SafetyLevel, planner::validate_rule_catalog,
};
use serde::Deserialize;

const BUILTIN_RULE_FILES: &[(&str, &str)] = &[
    (
        "rules/windows/user-temp.toml",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/rules/windows/user-temp.toml"
        )),
    ),
    (
        "rules/windows/edge-cache.toml",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/rules/windows/edge-cache.toml"
        )),
    ),
    (
        "rules/windows/firefox-profile-cache.toml",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/rules/windows/firefox-profile-cache.toml"
        )),
    ),
    (
        "rules/windows/chrome-cache.toml",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/rules/windows/chrome-cache.toml"
        )),
    ),
    (
        "rules/windows/directx-shader-cache.toml",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/rules/windows/directx-shader-cache.toml"
        )),
    ),
    (
        "rules/windows/discord-cache.toml",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/rules/windows/discord-cache.toml"
        )),
    ),
    (
        "rules/windows/steam-cache.toml",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/rules/windows/steam-cache.toml"
        )),
    ),
    (
        "rules/windows/npm-cache.toml",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/rules/windows/npm-cache.toml"
        )),
    ),
    (
        "rules/windows/pip-cache.toml",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/rules/windows/pip-cache.toml"
        )),
    ),
    (
        "rules/windows/cargo-cache.toml",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/rules/windows/cargo-cache.toml"
        )),
    ),
    (
        "rules/windows/jetbrains-cache.toml",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/rules/windows/jetbrains-cache.toml"
        )),
    ),
    (
        "rules/windows/thumbnail-cache.toml",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/rules/windows/thumbnail-cache.toml"
        )),
    ),
    (
        "rules/windows/vscode-cache.toml",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/rules/windows/vscode-cache.toml"
        )),
    ),
    (
        "rules/windows/wer-reports.toml",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/rules/windows/wer-reports.toml"
        )),
    ),
];

pub fn builtin_rules() -> Result<Vec<RuleDefinition>> {
    let mut rules = Vec::with_capacity(BUILTIN_RULE_FILES.len());

    for (path, raw) in BUILTIN_RULE_FILES {
        rules.push(parse_rule_file(path, raw)?);
    }

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
}

impl CatalogTarget {
    fn into_rule_target_spec(self) -> RuleTargetSpec {
        match self {
            Self::Template { value } => RuleTargetSpec::template(value),
            Self::ExactPath { value } => RuleTargetSpec::ExactPath(value),
            Self::GlobTemplate { value } => RuleTargetSpec::glob_template(value),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use rebecca_core::RuleSource;

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
            "windows.steam-cache",
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
}

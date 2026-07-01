use std::path::PathBuf;

use serde::Deserialize;

use crate::{
    Platform, RebeccaError, Result, RuleDefinition, RuleProvenance, RuleSearchKind, RuleTargetSpec,
    SafetyLevel,
    safety_catalog::{SafetyKnowledge, default_safety_knowledge},
};

const CLEANER_MANIFEST_VERSION: u16 = 1;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CleanerManifest {
    manifest_version: u16,
    id: String,
    platform: Platform,
    category: String,
    name: String,
    safety_level: SafetyLevel,
    restore_hint: Option<String>,
    #[serde(default)]
    warnings: Vec<String>,
    #[serde(default)]
    targets: Vec<ManifestTarget>,
    #[serde(default)]
    options: Vec<CleanerOption>,
    provenance: RuleProvenance,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CleanerOption {
    id: String,
    #[serde(default)]
    rule_id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    safety_level: Option<SafetyLevel>,
    #[serde(default)]
    restore_hint: Option<String>,
    #[serde(default)]
    warnings: Vec<String>,
    actions: Vec<ManifestAction>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
enum ManifestAction {
    Delete { target: ManifestTarget },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
enum ManifestTarget {
    Template {
        value: String,
        #[serde(default)]
        search_kind: Option<RuleSearchKind>,
    },
    ExactPath {
        value: PathBuf,
        #[serde(default)]
        search_kind: Option<RuleSearchKind>,
    },
    GlobTemplate {
        value: String,
        #[serde(default)]
        search_kind: Option<RuleSearchKind>,
    },
    SteamInstallTemplate {
        value: String,
        #[serde(default)]
        search_kind: Option<RuleSearchKind>,
    },
    SteamLibraryTemplate {
        value: String,
        #[serde(default)]
        search_kind: Option<RuleSearchKind>,
    },
}

pub fn parse_cleaner_manifest_file(path: &str, raw: &str) -> Result<Vec<RuleDefinition>> {
    parse_cleaner_manifest_file_with_safety_knowledge(path, raw, default_safety_knowledge())
}

pub fn parse_cleaner_manifest_file_with_safety_knowledge(
    path: &str,
    raw: &str,
    safety_knowledge: &SafetyKnowledge,
) -> Result<Vec<RuleDefinition>> {
    let manifest = toml::from_str::<CleanerManifest>(raw).map_err(|err| {
        RebeccaError::RuleCatalogInvalid(format!("{path} is invalid cleaner manifest data: {err}"))
    })?;

    compile_cleaner_manifest(path, manifest, safety_knowledge)
}

fn compile_cleaner_manifest(
    path: &str,
    manifest: CleanerManifest,
    safety_knowledge: &SafetyKnowledge,
) -> Result<Vec<RuleDefinition>> {
    if manifest.manifest_version != CLEANER_MANIFEST_VERSION {
        return Err(RebeccaError::RuleCatalogInvalid(format!(
            "{path} uses unsupported cleaner manifest version {}; expected {CLEANER_MANIFEST_VERSION}",
            manifest.manifest_version
        )));
    }

    validate_warnings(path, &manifest.id, &manifest.warnings, safety_knowledge)?;

    if manifest.options.is_empty() {
        if manifest.targets.is_empty() {
            return Err(RebeccaError::RuleCatalogInvalid(format!(
                "{path} cleaner {} must define targets or options",
                manifest.id
            )));
        }

        let path_templates = manifest
            .targets
            .into_iter()
            .map(|target| target.into_rule_target_spec(path, &manifest.id))
            .collect::<Result<Vec<_>>>()?;

        return Ok(vec![RuleDefinition {
            id: manifest.id,
            platform: manifest.platform,
            category: manifest.category,
            name: manifest.name,
            safety_level: manifest.safety_level,
            path_templates,
            restore_hint: manifest.restore_hint,
            warnings: manifest.warnings,
            provenance: manifest.provenance,
        }]);
    }

    if !manifest.targets.is_empty() {
        return Err(RebeccaError::RuleCatalogInvalid(format!(
            "{path} cleaner {} must not mix top-level targets with options",
            manifest.id
        )));
    }

    let cleaner_id = manifest.id;
    let platform = manifest.platform;
    let category = manifest.category;
    let name = manifest.name;
    let safety_level = manifest.safety_level;
    let restore_hint = manifest.restore_hint;
    let warnings = manifest.warnings;
    let provenance = manifest.provenance;

    manifest
        .options
        .into_iter()
        .map(|option| {
            compile_option_rule(
                path,
                CleanerDefaults {
                    id: &cleaner_id,
                    platform,
                    category: &category,
                    name: &name,
                    safety_level,
                    restore_hint: restore_hint.as_ref(),
                    warnings: &warnings,
                    provenance: &provenance,
                    safety_knowledge,
                },
                option,
            )
        })
        .collect()
}

#[derive(Debug, Clone, Copy)]
struct CleanerDefaults<'a> {
    id: &'a str,
    platform: Platform,
    category: &'a str,
    name: &'a str,
    safety_level: SafetyLevel,
    restore_hint: Option<&'a String>,
    warnings: &'a [String],
    provenance: &'a RuleProvenance,
    safety_knowledge: &'a SafetyKnowledge,
}

fn compile_option_rule(
    path: &str,
    defaults: CleanerDefaults<'_>,
    option: CleanerOption,
) -> Result<RuleDefinition> {
    if option.id.trim().is_empty() {
        return Err(RebeccaError::RuleCatalogInvalid(format!(
            "{path} cleaner {} contains an option with an empty id",
            defaults.id
        )));
    }
    if option.actions.is_empty() {
        return Err(RebeccaError::RuleCatalogInvalid(format!(
            "{path} cleaner {} option {} must define at least one action",
            defaults.id, option.id
        )));
    }

    validate_warnings(
        path,
        &format!("{}.{}", defaults.id, option.id),
        &option.warnings,
        defaults.safety_knowledge,
    )?;

    let rule_id = option
        .rule_id
        .unwrap_or_else(|| format!("{}.{}", defaults.id, option.id));
    let path_templates = option
        .actions
        .into_iter()
        .map(|action| action.into_rule_target_spec(path, &rule_id))
        .collect::<Result<Vec<_>>>()?;
    let warnings = merge_warnings(defaults.warnings, option.warnings);

    Ok(RuleDefinition {
        id: rule_id,
        platform: defaults.platform,
        category: defaults.category.to_string(),
        name: option
            .name
            .unwrap_or_else(|| format!("{} {}", defaults.name, option.id)),
        safety_level: option.safety_level.unwrap_or(defaults.safety_level),
        path_templates,
        restore_hint: option
            .restore_hint
            .or_else(|| defaults.restore_hint.cloned()),
        warnings,
        provenance: defaults.provenance.clone(),
    })
}

fn validate_warnings(
    path: &str,
    owner: &str,
    warnings: &[String],
    safety_knowledge: &SafetyKnowledge,
) -> Result<()> {
    for warning in warnings {
        if warning.trim().is_empty() {
            return Err(RebeccaError::RuleCatalogInvalid(format!(
                "{path} cleaner {owner} contains an empty warning kind"
            )));
        }
        if !safety_knowledge
            .warning_kinds()
            .iter()
            .any(|kind| kind.id().eq_ignore_ascii_case(warning))
        {
            return Err(RebeccaError::RuleCatalogInvalid(format!(
                "{path} cleaner {owner} uses unknown warning kind {warning}"
            )));
        }
    }

    Ok(())
}

fn merge_warnings(cleaner: &[String], option: Vec<String>) -> Vec<String> {
    let mut warnings = cleaner.to_vec();
    for warning in option {
        if !warnings
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(&warning))
        {
            warnings.push(warning);
        }
    }
    warnings
}

impl ManifestAction {
    fn into_rule_target_spec(self, path: &str, owner: &str) -> Result<RuleTargetSpec> {
        match self {
            Self::Delete { target } => target.into_rule_target_spec(path, owner),
        }
    }
}

impl ManifestTarget {
    fn into_rule_target_spec(self, path: &str, owner: &str) -> Result<RuleTargetSpec> {
        match self {
            Self::Template { value, search_kind } => {
                validate_search_kind(path, owner, search_kind, RuleSearchKind::File)?;
                Ok(RuleTargetSpec::template(value))
            }
            Self::ExactPath { value, search_kind } => {
                validate_search_kind(path, owner, search_kind, RuleSearchKind::File)?;
                Ok(RuleTargetSpec::ExactPath(value))
            }
            Self::GlobTemplate { value, search_kind } => {
                validate_search_kind(path, owner, search_kind, RuleSearchKind::Glob)?;
                Ok(RuleTargetSpec::glob_template(value))
            }
            Self::SteamInstallTemplate { value, search_kind } => {
                validate_search_kind(path, owner, search_kind, RuleSearchKind::SteamInstall)?;
                Ok(RuleTargetSpec::steam_install_template(value))
            }
            Self::SteamLibraryTemplate { value, search_kind } => {
                validate_search_kind(path, owner, search_kind, RuleSearchKind::SteamLibrary)?;
                Ok(RuleTargetSpec::steam_library_template(value))
            }
        }
    }
}

fn validate_search_kind(
    path: &str,
    owner: &str,
    declared: Option<RuleSearchKind>,
    expected: RuleSearchKind,
) -> Result<()> {
    if let Some(declared) = declared
        && declared != expected
    {
        return Err(RebeccaError::RuleCatalogInvalid(format!(
            "{path} cleaner {owner} declares incompatible search kind {}; expected {}",
            declared.label(),
            expected.label()
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::parse_cleaner_manifest_file;

    #[test]
    fn manifest_parser_rejects_missing_version() {
        let err = parse_cleaner_manifest_file(
            "test.toml",
            r#"
id = "windows.test"
platform = "windows"
category = "system"
name = "Test"
safety_level = "safe"

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

        assert!(err.to_string().contains("missing field `manifest_version`"));
    }

    #[test]
    fn manifest_parser_rejects_unsupported_version() {
        let err = parse_cleaner_manifest_file(
            "test.toml",
            r#"
manifest_version = 2
id = "windows.test"
platform = "windows"
category = "system"
name = "Test"
safety_level = "safe"

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

        assert!(
            err.to_string()
                .contains("unsupported cleaner manifest version")
        );
    }

    #[test]
    fn manifest_parser_compiles_option_actions_to_rules() {
        let rules = parse_cleaner_manifest_file(
            "test.toml",
            r#"
manifest_version = 1
id = "windows.example"
platform = "windows"
category = "system"
name = "Example"
safety_level = "safe"
restore_hint = "Regenerated automatically."
warnings = ["active-process"]

[[options]]
id = "cache"
rule_id = "windows.example-cache"
name = "Example cache"

[[options.actions]]
kind = "delete"
target = { kind = "template", value = "%TEMP%" }

[provenance]
source = "owned"
license = "project-owned"
notes = "test"
"#,
        )
        .expect("option manifest should parse");

        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].id, "windows.example-cache");
        assert_eq!(rules[0].name, "Example cache");
        assert_eq!(rules[0].path_templates.len(), 1);
    }

    #[test]
    fn manifest_parser_accepts_explicit_matching_search_kind() {
        let rules = parse_cleaner_manifest_file(
            "test.toml",
            r#"
manifest_version = 1
id = "windows.glob"
platform = "windows"
category = "system"
name = "Glob"
safety_level = "safe"

[[targets]]
kind = "glob-template"
value = "%TEMP%\\Profile*\\Cache"
search_kind = "glob"

[provenance]
source = "owned"
license = "project-owned"
notes = "test"
"#,
        )
        .expect("manifest should parse");

        assert_eq!(rules[0].path_templates[0].search_kind().label(), "glob");
    }

    #[test]
    fn manifest_parser_rejects_incompatible_search_kind() {
        let err = parse_cleaner_manifest_file(
            "test.toml",
            r#"
manifest_version = 1
id = "windows.bad-search"
platform = "windows"
category = "system"
name = "Bad Search"
safety_level = "safe"

[[targets]]
kind = "template"
value = "%TEMP%"
search_kind = "glob"

[provenance]
source = "owned"
license = "project-owned"
notes = "test"
"#,
        )
        .unwrap_err();

        assert!(err.to_string().contains("incompatible search kind"));
    }

    #[test]
    fn manifest_parser_rejects_unknown_warning_kind() {
        let err = parse_cleaner_manifest_file(
            "test.toml",
            r#"
manifest_version = 1
id = "windows.example"
platform = "windows"
category = "system"
name = "Example"
safety_level = "safe"
warnings = ["unknown-warning"]

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

        assert!(err.to_string().contains("unknown warning kind"));
    }

    #[test]
    fn manifest_parser_rejects_mixed_targets_and_options() {
        let err = parse_cleaner_manifest_file(
            "test.toml",
            r#"
manifest_version = 1
id = "windows.test"
platform = "windows"
category = "system"
name = "Test"
safety_level = "safe"

[[targets]]
kind = "template"
value = "%TEMP%"

[[options]]
id = "cache"

[[options.actions]]
kind = "delete"
target = { kind = "template", value = "%LOCALAPPDATA%\\Temp" }

[provenance]
source = "owned"
license = "project-owned"
notes = "test"
"#,
        )
        .unwrap_err();

        assert!(
            err.to_string()
                .contains("must not mix top-level targets with options")
        );
    }
}

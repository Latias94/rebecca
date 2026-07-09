use std::path::PathBuf;

use serde::Deserialize;

use crate::{
    Platform, RebeccaError, Result, RuleDefinition, RuleProvenance, RuleTargetSpec, SafetyLevel,
    safety_catalog::{SafetyKnowledge, default_safety_knowledge},
};

const CLEANER_MANIFEST_VERSION: u16 = 1;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CleanerManifest {
    manifest_version: u16,
    id: String,
    category: String,
    name: String,
    safety_level: SafetyLevel,
    restore_hint: Option<String>,
    #[serde(default)]
    warnings: Vec<String>,
    platforms: Vec<PlatformCleaner>,
    provenance: RuleProvenance,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlatformCleaner {
    platform: Platform,
    #[serde(default)]
    safety_level: Option<SafetyLevel>,
    #[serde(default)]
    restore_hint: Option<String>,
    #[serde(default)]
    warnings: Vec<String>,
    #[serde(default)]
    targets: Vec<ManifestTarget>,
    #[serde(default)]
    options: Vec<CleanerOption>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CleanerOption {
    id: String,
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
    Template { value: String },
    ExactPath { value: PathBuf },
    GlobTemplate { value: String },
    SteamInstallTemplate { value: String },
    SteamLibraryTemplate { value: String },
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

    if manifest.platforms.is_empty() {
        return Err(RebeccaError::RuleCatalogInvalid(format!(
            "{path} cleaner {} must define at least one platform",
            manifest.id
        )));
    }

    let defaults = CleanerSharedDefaults {
        id: manifest.id,
        category: manifest.category,
        name: manifest.name,
        safety_level: manifest.safety_level,
        restore_hint: manifest.restore_hint,
        warnings: manifest.warnings,
        provenance: manifest.provenance,
    };
    manifest
        .platforms
        .into_iter()
        .map(|platform| compile_platform_rules(path, &defaults, platform, safety_knowledge))
        .collect::<Result<Vec<_>>>()
        .map(|rules| rules.into_iter().flatten().collect())
}

#[derive(Debug)]
struct CleanerSharedDefaults {
    id: String,
    category: String,
    name: String,
    safety_level: SafetyLevel,
    restore_hint: Option<String>,
    warnings: Vec<String>,
    provenance: RuleProvenance,
}

fn compile_platform_rules(
    path: &str,
    defaults: &CleanerSharedDefaults,
    platform_cleaner: PlatformCleaner,
    safety_knowledge: &SafetyKnowledge,
) -> Result<Vec<RuleDefinition>> {
    let platform = platform_cleaner.platform;
    if platform == Platform::Unknown {
        return Err(RebeccaError::RuleCatalogInvalid(format!(
            "{path} cleaner {} platform block must target a supported platform",
            defaults.id
        )));
    }

    let rule_id = format!("{}.{}", platform.label(), defaults.id);
    validate_warnings(path, &rule_id, &platform_cleaner.warnings, safety_knowledge)?;
    let warnings = merge_warnings(&defaults.warnings, platform_cleaner.warnings);
    let safety_level = platform_cleaner
        .safety_level
        .unwrap_or(defaults.safety_level);
    let restore_hint = platform_cleaner
        .restore_hint
        .or_else(|| defaults.restore_hint.clone());

    if platform_cleaner.options.is_empty() {
        if platform_cleaner.targets.is_empty() {
            return Err(RebeccaError::RuleCatalogInvalid(format!(
                "{path} cleaner {rule_id} must define targets or options",
            )));
        }

        let path_templates = platform_cleaner
            .targets
            .into_iter()
            .map(|target| target.into_rule_target_spec(path, &rule_id))
            .collect::<Result<Vec<_>>>()?;

        return Ok(vec![RuleDefinition {
            id: rule_id,
            platform,
            category: defaults.category.clone(),
            name: defaults.name.clone(),
            safety_level,
            path_templates,
            restore_hint,
            warnings,
            provenance: defaults.provenance.clone(),
        }]);
    }

    if !platform_cleaner.targets.is_empty() {
        return Err(RebeccaError::RuleCatalogInvalid(format!(
            "{path} cleaner {rule_id} must not mix platform targets with options",
        )));
    }

    platform_cleaner
        .options
        .into_iter()
        .map(|option| {
            compile_option_rule(
                path,
                CleanerDefaults {
                    rule_id: &rule_id,
                    platform,
                    category: &defaults.category,
                    name: &defaults.name,
                    safety_level,
                    restore_hint: restore_hint.as_ref(),
                    warnings: &warnings,
                    provenance: &defaults.provenance,
                    safety_knowledge,
                },
                option,
            )
        })
        .collect()
}

#[derive(Debug, Clone, Copy)]
struct CleanerDefaults<'a> {
    rule_id: &'a str,
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
            defaults.rule_id
        )));
    }
    if option.actions.is_empty() {
        return Err(RebeccaError::RuleCatalogInvalid(format!(
            "{path} cleaner {} option {} must define at least one action",
            defaults.rule_id, option.id
        )));
    }

    validate_warnings(
        path,
        &format!("{}.{}", defaults.rule_id, option.id),
        &option.warnings,
        defaults.safety_knowledge,
    )?;

    let rule_id = format!("{}.{}", defaults.rule_id, option.id);
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

fn merge_warnings(cleaner: &[String], extra: Vec<String>) -> Vec<String> {
    let mut warnings = cleaner.to_vec();
    for warning in extra {
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
    fn into_rule_target_spec(self, _path: &str, _owner: &str) -> Result<RuleTargetSpec> {
        match self {
            Self::Template { value } => Ok(RuleTargetSpec::template(value)),
            Self::ExactPath { value } => Ok(RuleTargetSpec::ExactPath(value)),
            Self::GlobTemplate { value } => Ok(RuleTargetSpec::glob_template(value)),
            Self::SteamInstallTemplate { value } => {
                Ok(RuleTargetSpec::steam_install_template(value))
            }
            Self::SteamLibraryTemplate { value } => {
                Ok(RuleTargetSpec::steam_library_template(value))
            }
        }
    }
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
category = "system"
name = "Test"
safety_level = "safe"

[[platforms]]
platform = "windows"

[[platforms.targets]]
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
id = "test"
category = "system"
name = "Test"
safety_level = "safe"

[[platforms]]
platform = "windows"

[[platforms.targets]]
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
id = "example"
category = "system"
name = "Example"
safety_level = "safe"
restore_hint = "Regenerated automatically."
warnings = ["active-process"]

[[platforms]]
platform = "windows"

[[platforms.options]]
id = "cache"
name = "Example cache"

[[platforms.options.actions]]
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
        assert_eq!(rules[0].id, "windows.example.cache");
        assert_eq!(rules[0].name, "Example cache");
        assert_eq!(rules[0].path_templates.len(), 1);
    }

    #[test]
    fn manifest_parser_expands_shared_metadata_to_platform_rules() {
        let rules = parse_cleaner_manifest_file(
            "test.toml",
            r#"
manifest_version = 1
id = "user-temp"
category = "system"
name = "User temp"
safety_level = "safe"
restore_hint = "Regenerated automatically."

[[platforms]]
platform = "windows"

[[platforms.targets]]
kind = "template"
value = "%TEMP%"

[[platforms]]
platform = "linux"

[[platforms.targets]]
kind = "template"
value = "%TMPDIR%"

[provenance]
source = "owned"
license = "project-owned"
notes = "test"
"#,
        )
        .expect("shared platform manifest should parse");

        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].id, "windows.user-temp");
        assert_eq!(rules[0].platform.label(), "windows");
        assert_eq!(rules[1].id, "linux.user-temp");
        assert_eq!(rules[1].platform.label(), "linux");
    }

    #[test]
    fn manifest_parser_rejects_deprecated_search_kind() {
        let err = parse_cleaner_manifest_file(
            "test.toml",
            r#"
manifest_version = 1
id = "bad-search"
category = "system"
name = "Bad Search"
safety_level = "safe"

[[platforms]]
platform = "windows"

[[platforms.targets]]
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

        assert!(err.to_string().contains("unknown field `search_kind`"));
    }

    #[test]
    fn manifest_parser_rejects_unknown_warning_kind() {
        let err = parse_cleaner_manifest_file(
            "test.toml",
            r#"
manifest_version = 1
id = "example"
category = "system"
name = "Example"
safety_level = "safe"
warnings = ["unknown-warning"]

[[platforms]]
platform = "windows"

[[platforms.targets]]
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
    fn manifest_parser_rejects_mixed_platform_targets_and_options() {
        let err = parse_cleaner_manifest_file(
            "test.toml",
            r#"
manifest_version = 1
id = "test"
category = "system"
name = "Test"
safety_level = "safe"

[[platforms]]
platform = "windows"

[[platforms.targets]]
kind = "template"
value = "%TEMP%"

[[platforms.options]]
id = "cache"

[[platforms.options.actions]]
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
                .contains("must not mix platform targets with options")
        );
    }
}

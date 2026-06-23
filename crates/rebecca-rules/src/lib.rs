use rebecca_core::{
    DeletePolicy, Platform, Result, RuleDefinition, RuleProvenance, RuleSource, RuleTargetSpec,
    SafetyLevel, planner::validate_rule_catalog,
};

pub fn builtin_rules() -> Vec<RuleDefinition> {
    vec![
        windows_rule(
            "windows.user-temp",
            "system",
            "User temporary files",
            SafetyLevel::Safe,
            vec![
                RuleTargetSpec::template("%TEMP%"),
                RuleTargetSpec::template("%LOCALAPPDATA%\\Temp"),
            ],
            "Temporary files owned by the current user.",
        ),
        windows_rule(
            "windows.edge-cache",
            "browser",
            "Microsoft Edge cache",
            SafetyLevel::Safe,
            vec![
                RuleTargetSpec::template(
                    "%LOCALAPPDATA%\\Microsoft\\Edge\\User Data\\Default\\Cache",
                ),
                RuleTargetSpec::template(
                    "%LOCALAPPDATA%\\Microsoft\\Edge\\User Data\\Default\\Code Cache",
                ),
            ],
            "Browser cache that can be regenerated.",
        ),
        windows_rule(
            "windows.npm-cache",
            "development",
            "npm cache",
            SafetyLevel::Moderate,
            vec![RuleTargetSpec::template("%APPDATA%\\npm-cache\\_cacache")],
            "Package manager cache; packages may need to be downloaded again.",
        ),
    ]
}

pub fn validate_builtin_rules() -> Result<()> {
    validate_rule_catalog(&builtin_rules())
}

fn windows_rule(
    id: &str,
    category: &str,
    name: &str,
    safety_level: SafetyLevel,
    path_templates: Vec<RuleTargetSpec>,
    restore_hint: &str,
) -> RuleDefinition {
    RuleDefinition {
        id: id.to_string(),
        platform: Platform::Windows,
        category: category.to_string(),
        name: name.to_string(),
        safety_level,
        path_templates,
        delete_policy: DeletePolicy::RecycleBin,
        restore_hint: Some(restore_hint.to_string()),
        provenance: RuleProvenance {
            source: RuleSource::Owned,
            license: "project-owned".to_string(),
            notes: "Initial owned catalog based on common Windows cache conventions.".to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::builtin_rules;

    #[test]
    fn builtin_rule_ids_are_unique() {
        let rules = builtin_rules();
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
}

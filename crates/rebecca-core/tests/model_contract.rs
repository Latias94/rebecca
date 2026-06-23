use std::path::PathBuf;

use rebecca_core::plan::{CleanupPlan, CleanupTarget};
use rebecca_core::{
    DeleteMode, PlanRequest, Platform, RuleDefinition, RuleProvenance, RuleSelection, RuleSource,
    RuleTargetSpec, SafetyLevel,
};

#[test]
fn cleanup_plan_serialization_preserves_target_contract() {
    let request = PlanRequest::for_platform(Platform::Windows, DeleteMode::DryRun);
    let mut plan = CleanupPlan::empty(request);
    plan.targets.push(
        CleanupTarget::allowed(
            "windows.user-temp",
            PathBuf::from("C:/Users/Alice/AppData/Local/Temp"),
            42,
            DeleteMode::DryRun,
        )
        .with_restore_hint(Some("Temporary files can be recreated.".to_string())),
    );
    plan.recompute_summary();

    let json = serde_json::to_string(&plan).expect("plan should serialize");
    let decoded: CleanupPlan = serde_json::from_str(&json).expect("plan should deserialize");

    assert_eq!(decoded.summary.allowed_targets, 1);
    assert_eq!(decoded.summary.estimated_bytes, 42);
    assert_eq!(decoded.targets[0].rule_id, "windows.user-temp");
    assert_eq!(
        decoded.targets[0].restore_hint.as_deref(),
        Some("Temporary files can be recreated.")
    );
}

#[test]
fn invalid_rule_catalog_rejects_duplicate_ids() {
    let rule = test_rule("windows.same");
    let rules = vec![rule.clone(), rule];

    let err = rebecca_core::planner::validate_rule_catalog(&rules).unwrap_err();
    assert!(err.to_string().contains("duplicate rule id"));
}

#[test]
fn invalid_rule_catalog_rejects_duplicate_target_specs() {
    let mut first = test_rule("windows.first");
    let mut second = test_rule("windows.second");
    first.path_templates = vec![RuleTargetSpec::template("%TEMP%")];
    second.path_templates = vec![RuleTargetSpec::template("%temp%")];

    let err = rebecca_core::planner::validate_rule_catalog(&[first, second]).unwrap_err();

    assert!(err.to_string().contains("duplicate target spec"));
}

#[test]
fn rule_target_spec_exposes_placeholder_path_and_dedupe_key() {
    let template = RuleTargetSpec::steam_library_template("steamapps\\shadercache");
    let exact = RuleTargetSpec::ExactPath(PathBuf::from(r"C:\Temp\Cache"));

    assert_eq!(
        template.placeholder_path(),
        PathBuf::from("steamapps\\shadercache")
    );
    assert_eq!(exact.placeholder_path(), PathBuf::from(r"C:\Temp\Cache"));
    assert_eq!(
        template.dedupe_key(Platform::Windows),
        "Windows:steam-library-template:steamapps/shadercache"
    );
    assert_eq!(
        exact.dedupe_key(Platform::Windows),
        "Windows:exact-path:c:/temp/cache"
    );
}

#[test]
fn safety_level_exposes_label_and_opt_in_flag() {
    assert_eq!(SafetyLevel::Safe.label(), "safe");
    assert_eq!(SafetyLevel::Moderate.label(), "moderate");
    assert_eq!(SafetyLevel::Risky.label(), "risky");
    assert_eq!(SafetyLevel::Dangerous.label(), "dangerous");

    assert_eq!(SafetyLevel::Safe.opt_in_flag(), None);
    assert_eq!(
        SafetyLevel::Moderate.opt_in_flag(),
        Some("--allow-moderate")
    );
    assert_eq!(SafetyLevel::Risky.opt_in_flag(), Some("--allow-risky"));
    assert_eq!(SafetyLevel::Dangerous.opt_in_flag(), Some("--allow-risky"));
}

#[test]
fn rule_selection_matches_rules_case_insensitively() {
    let selection = RuleSelection::new(
        vec!["SYSTEM".to_string()],
        vec!["WINDOWS.USER-TEMP".to_string()],
    );
    let rule = test_rule("windows.user-temp");
    let browser_rule = RuleDefinition {
        category: "browser".to_string(),
        ..test_rule("windows.browser-cache")
    };

    assert!(selection.matches_rule(&rule));
    assert!(!selection.matches_rule(&browser_rule));
}

#[test]
fn rule_selection_validation_rejects_unknown_category() {
    let selection = RuleSelection::new(vec!["missing".to_string()], Vec::new());
    let rules = vec![test_rule("windows.user-temp")];

    let err = selection.validate_against_rules(&rules).unwrap_err();

    assert!(err.to_string().contains("invalid category"));
}

#[test]
fn rule_selection_validation_rejects_unknown_rule_id() {
    let selection = RuleSelection::new(Vec::new(), vec!["missing.rule".to_string()]);
    let rules = vec![test_rule("windows.user-temp")];

    let err = selection.validate_against_rules(&rules).unwrap_err();

    assert!(err.to_string().contains("invalid rule id"));
}

#[test]
fn rule_selection_validation_is_case_insensitive() {
    let selection = RuleSelection::new(
        vec!["SYSTEM".to_string()],
        vec!["WINDOWS.USER-TEMP".to_string()],
    );
    let rules = vec![test_rule("windows.user-temp")];

    selection
        .validate_against_rules(&rules)
        .expect("selection should validate case-insensitively");
}

fn test_rule(id: &str) -> RuleDefinition {
    RuleDefinition {
        id: id.to_string(),
        platform: Platform::Windows,
        category: "system".to_string(),
        name: "Test rule".to_string(),
        safety_level: SafetyLevel::Safe,
        path_templates: vec![RuleTargetSpec::template("%TEMP%")],
        delete_policy: rebecca_core::DeletePolicy::RecycleBin,
        restore_hint: Some("Regenerated automatically.".to_string()),
        provenance: RuleProvenance {
            source: RuleSource::Owned,
            license: "project-owned".to_string(),
            notes: "test rule".to_string(),
        },
    }
}

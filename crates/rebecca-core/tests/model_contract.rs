use std::path::PathBuf;

use rebecca_core::plan::{CleanupPlan, CleanupTarget};
use rebecca_core::{
    DeleteMode, PlanRequest, Platform, RuleDefinition, RuleProvenance, RuleSource, RuleTargetSpec,
    SafetyLevel,
};

#[test]
fn cleanup_plan_serialization_preserves_target_contract() {
    let request = PlanRequest::for_platform(Platform::Windows, DeleteMode::DryRun);
    let mut plan = CleanupPlan::empty(request);
    plan.targets.push(CleanupTarget::allowed(
        "windows.user-temp",
        PathBuf::from("C:/Users/Alice/AppData/Local/Temp"),
        42,
        DeleteMode::DryRun,
    ));
    plan.recompute_summary();

    let json = serde_json::to_string(&plan).expect("plan should serialize");
    let decoded: CleanupPlan = serde_json::from_str(&json).expect("plan should deserialize");

    assert_eq!(decoded.summary.allowed_targets, 1);
    assert_eq!(decoded.summary.estimated_bytes, 42);
    assert_eq!(decoded.targets[0].rule_id, "windows.user-temp");
}

#[test]
fn invalid_rule_catalog_rejects_duplicate_ids() {
    let rule = test_rule("windows.same");
    let rules = vec![rule.clone(), rule];

    let err = rebecca_core::planner::validate_rule_catalog(&rules).unwrap_err();
    assert!(err.to_string().contains("duplicate rule id"));
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

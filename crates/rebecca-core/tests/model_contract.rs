use std::collections::BTreeMap;
use std::path::PathBuf;

use rebecca_core::plan::{
    CleanupPlan, CleanupTarget, CleanupTargetEvidence, CleanupTargetIssueReason,
    EstimateProvenance, EstimateSource,
};
use rebecca_core::project_artifacts::{
    ProjectArtifactDiscoveryDiagnostic, ProjectArtifactDiscoveryDiagnosticKind,
};
use rebecca_core::scan::{
    ScanBackendEvidence, ScanBackendKind, ScanEstimateCaveat, ScanEstimateConfidence,
};
use rebecca_core::{
    CleanupWorkflow, DeleteMode, PlanRequest, Platform, RuleDefinition, RuleProvenance,
    RuleSelection, RuleSource, RuleTargetSpec, SafetyLevel, TargetStatus,
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
    let value: serde_json::Value = serde_json::from_str(&json).expect("plan JSON should parse");
    assert_eq!(value["targets"][0]["estimate_source"], "fresh-scan");

    let decoded: CleanupPlan = serde_json::from_str(&json).expect("plan should deserialize");

    assert_eq!(decoded.summary.allowed_targets, 1);
    assert_eq!(decoded.summary.estimated_bytes, 42);
    assert_eq!(decoded.targets[0].rule_id, "windows.user-temp");
    assert_eq!(
        decoded.targets[0].restore_hint.as_deref(),
        Some("Temporary files can be recreated.")
    );
    assert_eq!(
        decoded.targets[0].deletion_style,
        rebecca_core::CleanupTargetDeletionStyle::PreserveRootContents
    );
    assert_eq!(decoded.targets[0].modified_at_unix_seconds, None);
    assert!(decoded.summary.issue_matrix.is_empty());
    assert!(decoded.summary.warning_matrix.is_empty());
    assert!(decoded.targets[0].reason_code.is_none());
    assert!(decoded.targets[0].warnings.is_empty());
    assert!(decoded.targets[0].evidence.is_empty());
    assert_eq!(
        decoded.targets[0].estimate_source,
        EstimateSource::FreshScan
    );
}

#[test]
fn cleanup_plan_summary_ignores_non_issue_status_evidence() {
    let request = PlanRequest::for_platform(Platform::Windows, DeleteMode::DryRun);
    let mut target = CleanupTarget::allowed(
        "windows.user-temp",
        PathBuf::from("C:/Users/Alice/AppData/Local/Temp"),
        42,
        DeleteMode::DryRun,
    );
    target.evidence.push(CleanupTargetEvidence::issue(
        TargetStatus::Allowed,
        CleanupTargetIssueReason::Unclassified,
        "malformed external evidence",
    ));

    let mut plan = CleanupPlan::empty(request);
    plan.targets.push(target);
    plan.recompute_summary();

    assert_eq!(plan.summary.allowed_targets, 1);
    assert!(plan.summary.issue_matrix.is_empty());
}

#[test]
fn cleanup_plan_serialization_preserves_warning_contract() {
    let request = PlanRequest::for_platform(Platform::Windows, DeleteMode::DryRun);
    let mut plan = CleanupPlan::empty(request);
    plan.targets.push(
        CleanupTarget::skipped_with_reason_code(
            "windows.slack-cache",
            PathBuf::from(r"C:\Users\Alice\AppData\Roaming\Slack\Cache"),
            DeleteMode::DryRun,
            CleanupTargetIssueReason::WarningGateRequired,
            "warning gate requires --allow-warning active-process",
        )
        .with_warnings(vec!["active-process".to_string()]),
    );
    plan.recompute_summary();

    let json = serde_json::to_value(&plan).expect("plan should serialize");
    assert_eq!(json["targets"][0]["warnings"][0], "active-process");
    assert_eq!(json["targets"][0]["reason_code"], "warning-gate-required");
    assert_eq!(json["targets"][0]["evidence"][0]["kind"], "issue");
    assert_eq!(
        json["targets"][0]["evidence"][0]["reason_code"],
        "warning-gate-required"
    );
    assert_eq!(json["targets"][0]["evidence"][1]["kind"], "warning");
    assert_eq!(
        json["targets"][0]["evidence"][1]["warning"],
        "active-process"
    );
    assert_eq!(
        json["summary"]["warning_matrix"][0]["warning"],
        "active-process"
    );
    assert_eq!(json["summary"]["warning_matrix"][0]["targets"], 1);

    let decoded: CleanupPlan = serde_json::from_value(json).expect("plan should deserialize");
    assert_eq!(
        decoded.targets[0].reason_code,
        Some(CleanupTargetIssueReason::WarningGateRequired)
    );
    assert_eq!(decoded.targets[0].warnings, ["active-process"]);
    assert_eq!(decoded.targets[0].evidence.len(), 2);
    assert_eq!(decoded.summary.warning_matrix[0].warning, "active-process");
}

#[test]
fn cleanup_plan_serialization_preserves_protected_issue_contract() {
    let request = PlanRequest::for_platform(Platform::Windows, DeleteMode::DryRun);
    let mut plan = CleanupPlan::empty(request);
    plan.targets.push(CleanupTarget::blocked_with_reason_code(
        "windows.custom-browser-history",
        PathBuf::from("C:/Users/Alice/AppData/Local/Google/Chrome/User Data/Default/History"),
        DeleteMode::DryRun,
        CleanupTargetIssueReason::SafetyPolicyBlocked,
        "browser private data is protected",
    ));
    plan.recompute_summary();

    let json = serde_json::to_string(&plan).expect("plan should serialize");
    let value: serde_json::Value = serde_json::from_str(&json).expect("plan JSON should parse");
    assert_eq!(
        value["summary"]["issue_matrix"][0]["reason_code"],
        "safety-policy-blocked"
    );
    assert_eq!(value["targets"][0]["reason_code"], "safety-policy-blocked");
    assert_eq!(value["targets"][0]["evidence"][0]["kind"], "issue");
    assert_eq!(
        value["targets"][0]["evidence"][0]["reason_code"],
        "safety-policy-blocked"
    );

    let decoded: CleanupPlan = serde_json::from_str(&json).expect("plan should deserialize");

    assert_eq!(decoded.summary.blocked_targets, 1);
    assert_eq!(decoded.summary.issue_matrix.len(), 1);
    assert_eq!(
        decoded.summary.issue_matrix[0].reason_code,
        CleanupTargetIssueReason::SafetyPolicyBlocked
    );
    assert_eq!(
        decoded.targets[0].reason_code,
        Some(CleanupTargetIssueReason::SafetyPolicyBlocked)
    );
    assert_eq!(decoded.targets[0].evidence.len(), 1);
    assert_eq!(
        decoded.targets[0].reason.as_deref(),
        Some("browser private data is protected")
    );
}

#[test]
fn cleanup_plan_serialization_preserves_permission_issue_contract() {
    let request = PlanRequest::for_platform(Platform::Macos, DeleteMode::RecoverableDelete);
    let mut plan = CleanupPlan::empty(request);
    plan.targets.push(CleanupTarget::failed_with_reason_code(
        "macos.mail-cache",
        PathBuf::from("/Users/alice/Library/Mail"),
        DeleteMode::RecoverableDelete,
        0,
        CleanupTargetIssueReason::ScanPermissionDenied,
        "permission-denied during root-metadata",
    ));
    plan.targets.push(CleanupTarget::failed_with_reason_code(
        "macos.safari-cache",
        PathBuf::from("/Users/alice/Library/Safari"),
        DeleteMode::RecoverableDelete,
        0,
        CleanupTargetIssueReason::ExecutionPermissionDenied,
        "permission denied",
    ));
    plan.recompute_summary();

    let json = serde_json::to_value(&plan).expect("plan should serialize");
    assert!(
        json["targets"]
            .as_array()
            .unwrap()
            .iter()
            .any(|target| { target["reason_code"] == "scan-permission-denied" })
    );
    assert!(
        json["targets"]
            .as_array()
            .unwrap()
            .iter()
            .any(|target| { target["reason_code"] == "execution-permission-denied" })
    );
    assert!(
        json["summary"]["issue_matrix"]
            .as_array()
            .unwrap()
            .iter()
            .any(|issue| { issue["reason_code"] == "scan-permission-denied" })
    );
    assert!(
        json["summary"]["issue_matrix"]
            .as_array()
            .unwrap()
            .iter()
            .any(|issue| { issue["reason_code"] == "execution-permission-denied" })
    );

    let decoded: CleanupPlan = serde_json::from_value(json).expect("plan should deserialize");
    assert!(decoded.targets.iter().any(|target| {
        target.reason_code == Some(CleanupTargetIssueReason::ScanPermissionDenied)
    }));
    assert!(decoded.targets.iter().any(|target| {
        target.reason_code == Some(CleanupTargetIssueReason::ExecutionPermissionDenied)
    }));
}

#[test]
fn cleanup_plan_deserializes_legacy_issue_fields() {
    let request = PlanRequest::for_platform(Platform::Windows, DeleteMode::DryRun);
    let mut plan = CleanupPlan::empty(request);
    plan.targets.push(CleanupTarget::skipped_with_reason_code(
        "windows.user-temp",
        PathBuf::from("C:/Users/Alice/AppData/Local/Temp"),
        DeleteMode::DryRun,
        CleanupTargetIssueReason::DuplicateTargetPath,
        "duplicate target path already covered",
    ));
    plan.recompute_summary();

    let mut value = serde_json::to_value(&plan).expect("plan should serialize");
    let root = value.as_object_mut().expect("plan should be object");
    root.get_mut("summary")
        .and_then(serde_json::Value::as_object_mut)
        .expect("summary should be object")
        .remove("issue_matrix");
    root.get_mut("summary")
        .and_then(serde_json::Value::as_object_mut)
        .expect("summary should be object")
        .remove("warning_matrix");
    root.get_mut("targets")
        .and_then(serde_json::Value::as_array_mut)
        .expect("targets should be array")[0]
        .as_object_mut()
        .expect("target should be object")
        .remove("reason_code");
    root.get_mut("targets")
        .and_then(serde_json::Value::as_array_mut)
        .expect("targets should be array")[0]
        .as_object_mut()
        .expect("target should be object")
        .remove("warnings");

    let decoded: CleanupPlan = serde_json::from_value(value).expect("legacy plan should load");

    assert_eq!(decoded.summary.skipped_targets, 1);
    assert!(decoded.summary.issue_matrix.is_empty());
    assert!(decoded.summary.warning_matrix.is_empty());
    assert_eq!(decoded.targets[0].reason_code, None);
    assert!(decoded.targets[0].warnings.is_empty());
}

#[test]
fn cleanup_plan_serialization_preserves_estimate_source_contract() {
    let request = PlanRequest::for_platform(Platform::Windows, DeleteMode::DryRun);
    let mut plan = CleanupPlan::empty(request);
    plan.targets.push(
        CleanupTarget::allowed(
            "windows.user-temp",
            PathBuf::from("C:/Users/Alice/AppData/Local/Temp"),
            42,
            DeleteMode::DryRun,
        )
        .with_estimate_source(EstimateSource::ScanCache),
    );
    plan.targets.push(CleanupTarget::skipped_with_reason_code(
        "windows.missing",
        PathBuf::from("C:/Missing"),
        DeleteMode::DryRun,
        CleanupTargetIssueReason::SafetyPolicySkipped,
        "path does not exist",
    ));
    plan.recompute_summary();

    let json = serde_json::to_value(&plan).expect("plan should serialize");
    assert_eq!(json["targets"][0]["estimate_source"], "scan-cache");
    assert_eq!(json["targets"][1]["estimate_source"], "not-measured");

    let decoded: CleanupPlan = serde_json::from_value(json).expect("plan should deserialize");
    assert_eq!(
        decoded.targets[0].estimate_source,
        EstimateSource::ScanCache
    );
    assert_eq!(
        decoded.targets[1].estimate_source,
        EstimateSource::NotMeasured
    );
}

#[test]
fn cleanup_plan_serialization_preserves_estimate_provenance_contract() {
    let request = PlanRequest::for_platform(Platform::Windows, DeleteMode::DryRun);
    let mut plan = CleanupPlan::empty(request);
    let mut timings_ms = BTreeMap::new();
    timings_ms.insert("read-volume-data".to_string(), 7);
    let mut counters = BTreeMap::new();
    counters.insert("parsed-records".to_string(), 42);
    let mut evidence = ScanBackendEvidence {
        timings_ms,
        counters,
        cache_events: Vec::new(),
    };
    evidence.record_cache_event(
        "ntfs-volume-index",
        "miss",
        Some("manifest-missing".to_string()),
    );
    plan.targets.push(
        CleanupTarget::allowed(
            "windows.user-temp",
            PathBuf::from("C:/Users/Alice/AppData/Local/Temp"),
            42,
            DeleteMode::DryRun,
        )
        .with_estimate_provenance(EstimateProvenance {
            estimate_backend: Some(ScanBackendKind::WindowsNative),
            estimate_backend_source: Some("windows-native-usn".to_string()),
            estimate_confidence: Some(ScanEstimateConfidence::Exact),
            estimate_fallback_reason: Some("windows-native: unavailable".to_string()),
            estimate_caveats: vec![ScanEstimateCaveat {
                code: "native-fallback".to_string(),
                message: "native backend fell back to portable scanning".to_string(),
            }],
            estimate_backend_evidence: evidence,
        }),
    );
    plan.recompute_summary();

    let json = serde_json::to_value(&plan).expect("plan should serialize");
    assert_eq!(json["targets"][0]["estimate_source"], "fresh-scan");
    assert_eq!(json["targets"][0]["estimate_backend"], "windows-native");
    assert_eq!(
        json["targets"][0]["estimate_backend_source"],
        "windows-native-usn"
    );
    assert_eq!(json["targets"][0]["estimate_confidence"], "exact");
    assert_eq!(
        json["targets"][0]["estimate_fallback_reason"],
        "windows-native: unavailable"
    );
    assert_eq!(
        json["targets"][0]["estimate_caveats"][0]["code"],
        "native-fallback"
    );
    assert_eq!(
        json["targets"][0]["estimate_backend_evidence"]["timings_ms"]["read-volume-data"],
        7
    );
    assert_eq!(
        json["targets"][0]["estimate_backend_evidence"]["counters"]["parsed-records"],
        42
    );
    assert_eq!(
        json["targets"][0]["estimate_backend_evidence"]["cache_events"][0]["reason"],
        "manifest-missing"
    );

    let decoded: CleanupPlan = serde_json::from_value(json).expect("plan should deserialize");
    assert_eq!(
        decoded.targets[0].estimate_provenance.estimate_backend,
        Some(ScanBackendKind::WindowsNative)
    );
    assert_eq!(
        decoded.targets[0]
            .estimate_provenance
            .estimate_backend_source
            .as_deref(),
        Some("windows-native-usn")
    );
    assert_eq!(
        decoded.targets[0]
            .estimate_provenance
            .estimate_fallback_reason
            .as_deref(),
        Some("windows-native: unavailable")
    );
    assert_eq!(
        decoded.targets[0].estimate_provenance.estimate_caveats[0].code,
        "native-fallback"
    );
    assert_eq!(
        decoded.targets[0]
            .estimate_provenance
            .estimate_backend_evidence
            .counters
            .get("parsed-records"),
        Some(&42)
    );
}

#[test]
fn cleanup_plan_deserializes_legacy_target_without_estimate_source() {
    let request = PlanRequest::for_platform(Platform::Windows, DeleteMode::DryRun);
    let mut plan = CleanupPlan::empty(request);
    plan.targets.push(CleanupTarget::allowed(
        "windows.user-temp",
        PathBuf::from("C:/Users/Alice/AppData/Local/Temp"),
        42,
        DeleteMode::DryRun,
    ));
    plan.recompute_summary();

    let mut value = serde_json::to_value(&plan).expect("plan should serialize");
    value["targets"]
        .as_array_mut()
        .expect("targets should be array")[0]
        .as_object_mut()
        .expect("target should be object")
        .remove("estimate_source");

    let decoded: CleanupPlan = serde_json::from_value(value).expect("legacy plan should load");

    assert_eq!(decoded.targets[0].estimate_source, EstimateSource::Unknown);
    assert!(decoded.targets[0].estimate_provenance.is_empty());
}

#[test]
fn cleanup_plan_serialization_preserves_discovery_diagnostics_contract() {
    let request = PlanRequest::for_platform(Platform::Windows, DeleteMode::DryRun)
        .with_workflow(CleanupWorkflow::ProjectArtifacts);
    let mut plan = CleanupPlan::empty(request);
    plan.discovery_diagnostics
        .push(ProjectArtifactDiscoveryDiagnostic::new(
            ProjectArtifactDiscoveryDiagnosticKind::RootMissing,
            PathBuf::from(r"C:\Missing"),
            "project artifact root does not exist",
        ));

    let json = serde_json::to_value(&plan).expect("plan should serialize");
    assert_eq!(json["discovery_diagnostics"][0]["kind"], "root-missing");
    assert_eq!(json["discovery_diagnostics"][0]["path"], r"C:\Missing");
    assert_eq!(
        json["discovery_diagnostics"][0]["detail"],
        "project artifact root does not exist"
    );

    let decoded: CleanupPlan = serde_json::from_value(json).expect("plan should deserialize");
    assert_eq!(decoded.discovery_diagnostics.len(), 1);
    assert_eq!(
        decoded.discovery_diagnostics[0].kind,
        ProjectArtifactDiscoveryDiagnosticKind::RootMissing
    );
}

#[test]
fn cleanup_plan_deserializes_legacy_without_discovery_diagnostics() {
    let request = PlanRequest::for_platform(Platform::Windows, DeleteMode::DryRun);
    let plan = CleanupPlan::empty(request);
    let mut value = serde_json::to_value(&plan).expect("plan should serialize");
    value
        .as_object_mut()
        .expect("plan should be object")
        .remove("discovery_diagnostics");

    let decoded: CleanupPlan = serde_json::from_value(value).expect("legacy plan should load");

    assert!(decoded.discovery_diagnostics.is_empty());
}

#[test]
fn cleanup_plan_serialization_preserves_workflow_contract() {
    let request = PlanRequest::for_platform(Platform::Windows, DeleteMode::DryRun)
        .with_workflow(CleanupWorkflow::AppLeftovers);
    let plan = CleanupPlan::empty(request);

    let json = serde_json::to_value(&plan).expect("plan should serialize");
    assert_eq!(json["request"]["workflow"], "app-leftovers");

    let decoded: CleanupPlan = serde_json::from_value(json).expect("plan should deserialize");
    assert_eq!(decoded.request.workflow, CleanupWorkflow::AppLeftovers);
}

#[test]
fn cleanup_plan_serialization_preserves_project_artifacts_request_contract() {
    let mut request = PlanRequest::for_platform(Platform::Windows, DeleteMode::DryRun)
        .with_workflow(CleanupWorkflow::ProjectArtifacts);
    request.project_artifact_roots = vec![PathBuf::from(r"C:\Projects")];
    request.project_artifact_max_depth = 3;
    request.project_artifact_min_age_days = 0;
    request.project_artifact_reclaim_limit_bytes = Some(4096);
    request.project_artifact_selectors = vec!["node_modules".to_string()];
    let plan = CleanupPlan::empty(request);

    let json = serde_json::to_value(&plan).expect("plan should serialize");
    assert_eq!(json["request"]["workflow"], "project-artifacts");
    assert_eq!(json["request"]["project_artifact_roots"][0], r"C:\Projects");
    assert_eq!(json["request"]["project_artifact_max_depth"], 3);
    assert_eq!(json["request"]["project_artifact_min_age_days"], 0);
    assert_eq!(
        json["request"]["project_artifact_reclaim_limit_bytes"],
        4096
    );
    assert_eq!(
        json["request"]["project_artifact_selectors"][0],
        "node_modules"
    );

    let decoded: CleanupPlan = serde_json::from_value(json).expect("plan should deserialize");
    assert_eq!(decoded.request.workflow, CleanupWorkflow::ProjectArtifacts);
    assert_eq!(
        decoded.request.project_artifact_roots,
        vec![PathBuf::from(r"C:\Projects")]
    );
    assert_eq!(decoded.request.project_artifact_max_depth, 3);
    assert_eq!(decoded.request.project_artifact_min_age_days, 0);
    assert_eq!(
        decoded.request.project_artifact_reclaim_limit_bytes,
        Some(4096)
    );
    assert_eq!(
        decoded.request.project_artifact_selectors,
        vec!["node_modules"]
    );
}

#[test]
fn cleanup_plan_deserializes_legacy_request_without_workflow() {
    let request = PlanRequest::for_platform(Platform::Windows, DeleteMode::DryRun);
    let plan = CleanupPlan::empty(request);
    let mut value = serde_json::to_value(&plan).expect("plan should serialize");
    value["request"]
        .as_object_mut()
        .expect("request should be object")
        .remove("workflow");

    let decoded: CleanupPlan = serde_json::from_value(value).expect("legacy plan should load");

    assert_eq!(decoded.request.workflow, CleanupWorkflow::Rules);
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
fn invalid_rule_catalog_rejects_empty_category() {
    let mut rule = test_rule("windows.empty-category");
    rule.category = "   ".to_string();

    let err = rebecca_core::planner::validate_rule_catalog(&[rule]).unwrap_err();

    assert!(err.to_string().contains("must define a category"));
}

#[test]
fn invalid_rule_catalog_rejects_empty_name() {
    let mut rule = test_rule("windows.empty-name");
    rule.name = String::new();

    let err = rebecca_core::planner::validate_rule_catalog(&[rule]).unwrap_err();

    assert!(err.to_string().contains("must define a name"));
}

#[test]
fn invalid_rule_catalog_rejects_empty_target_paths() {
    let mut rule = test_rule("windows.empty-target");
    rule.path_templates = vec![RuleTargetSpec::template("   ")];

    let err = rebecca_core::planner::validate_rule_catalog(&[rule]).unwrap_err();

    assert!(err.to_string().contains("empty target path"));
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
        "windows:steam-library-template:steamapps/shadercache"
    );
    assert_eq!(
        exact.dedupe_key(Platform::Windows),
        "windows:exact-path:c:/temp/cache"
    );
}

#[test]
fn rule_definition_serialization_preserves_warning_contract() {
    let mut rule = test_rule("windows.warning-test");
    rule.warnings = vec!["active-process".to_string(), "broad-discovery".to_string()];

    let json = serde_json::to_value(&rule).expect("rule should serialize");
    assert_eq!(json["warnings"][0], "active-process");
    assert_eq!(json["warnings"][1], "broad-discovery");

    let decoded: RuleDefinition = serde_json::from_value(json).expect("rule should deserialize");
    assert_eq!(
        decoded.warnings,
        vec!["active-process".to_string(), "broad-discovery".to_string()]
    );
}

#[test]
fn rule_definition_deserializes_legacy_without_warnings() {
    let rule = test_rule("windows.legacy-warning-test");
    let mut json = serde_json::to_value(&rule).expect("rule should serialize");
    json.as_object_mut()
        .expect("rule should be object")
        .remove("warnings");

    let decoded: RuleDefinition = serde_json::from_value(json).expect("legacy rule should load");

    assert!(decoded.warnings.is_empty());
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
        restore_hint: Some("Regenerated automatically.".to_string()),
        warnings: Vec::new(),
        provenance: RuleProvenance {
            source: RuleSource::Owned,
            license: "project-owned".to_string(),
            notes: "test rule".to_string(),
        },
    }
}

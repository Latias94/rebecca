use std::path::{Path, PathBuf};

use rebecca_core::applications::NoopApplicationDiscovery;
use rebecca_core::cleanup_advice::{
    CleanupAdviceBuildRequest, CleanupAdviceIndex, CleanupAdviceRelation, CleanupAdviceStatus,
};
use rebecca_core::environment::MapEnvironment;
use rebecca_core::model::{
    DeleteMode, PlanRequest, Platform, RuleDefinition, RuleProvenance, RuleSource, RuleTargetSpec,
    SafetyLevel,
};
use rebecca_core::project_artifacts::{
    ProjectArtifactCandidate, ProjectArtifactScanOptions,
    discover_project_artifacts_with_diagnostics,
};
use rebecca_core::protection::ProtectionPolicy;
use rebecca_core::scan::ScanCancellationToken;

fn rule(id: &str, target: impl AsRef<Path>, safety_level: SafetyLevel) -> RuleDefinition {
    RuleDefinition {
        id: id.to_string(),
        platform: Platform::Windows,
        category: "cache".to_string(),
        name: id.to_string(),
        safety_level,
        path_templates: vec![RuleTargetSpec::ExactPath(target.as_ref().to_path_buf())],
        restore_hint: Some("Rebuild the cache by rerunning the owning application.".to_string()),
        warnings: Vec::new(),
        provenance: RuleProvenance {
            source: RuleSource::Owned,
            license: "Project-owned".to_string(),
            notes: "test rule".to_string(),
        },
    }
}

fn warning_rule(id: &str, target: impl AsRef<Path>, warning: &str) -> RuleDefinition {
    RuleDefinition {
        warnings: vec![warning.to_string()],
        ..rule(id, target, SafetyLevel::Safe)
    }
}

fn build_index<'a>(
    rules: &[RuleDefinition],
    protection_policy: ProtectionPolicy<'a>,
) -> CleanupAdviceIndex<'a> {
    let request = PlanRequest::for_platform(Platform::Windows, DeleteMode::DryRun);
    CleanupAdviceIndex::build(
        CleanupAdviceBuildRequest::new(request, protection_policy),
        rules,
        &MapEnvironment::new(),
        &NoopApplicationDiscovery::new(),
    )
    .unwrap()
}

fn write_fixture_file(path: impl AsRef<Path>, bytes: &[u8]) {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, bytes).unwrap();
}

fn discover_artifacts(root: PathBuf) -> Vec<ProjectArtifactCandidate> {
    discover_project_artifacts_with_diagnostics(
        &ProjectArtifactScanOptions::new(vec![root]),
        &ScanCancellationToken::new(),
    )
    .unwrap()
    .candidates
}

#[test]
fn exact_safe_rule_target_is_cleanable() {
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("cache");
    let index = build_index(
        &[rule("test.cache", &target, SafetyLevel::Safe)],
        ProtectionPolicy::new(),
    );

    let advice = index.advise_path(&target);

    assert_eq!(advice.status, CleanupAdviceStatus::Cleanable);
    assert_eq!(advice.relation, Some(CleanupAdviceRelation::Exact));
    assert_eq!(advice.rule_id.as_deref(), Some("test.cache"));
    assert_eq!(advice.matched_path.as_deref(), Some(target.as_path()));
    assert_eq!(
        advice.suggested_command.as_ref().unwrap().args,
        ["clean", "--dry-run", "--rule", "test.cache"]
    );
}

#[test]
fn moderate_rule_target_is_maybe_cleanable_without_adding_risk_flags_to_command() {
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("cache");
    let index = build_index(
        &[rule("test.moderate", &target, SafetyLevel::Moderate)],
        ProtectionPolicy::new(),
    );

    let advice = index.advise_path(&target);

    assert_eq!(advice.status, CleanupAdviceStatus::MaybeCleanable);
    assert_eq!(advice.required_flags, ["--allow-moderate"]);
    assert_eq!(
        advice.suggested_command.as_ref().unwrap().args,
        ["clean", "--dry-run", "--rule", "test.moderate"]
    );
}

#[test]
fn warning_gated_rule_target_is_maybe_cleanable() {
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("cache");
    let index = build_index(
        &[warning_rule("test.warning", &target, "active-process")],
        ProtectionPolicy::new(),
    );

    let advice = index.advise_path(&target);

    assert_eq!(advice.status, CleanupAdviceStatus::MaybeCleanable);
    assert_eq!(advice.required_warnings, ["active-process"]);
}

#[test]
fn parent_directory_contains_cleanable_target_but_is_not_directly_cleanable() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    let target = root.join("cache");
    let index = build_index(
        &[rule("test.cache", &target, SafetyLevel::Safe)],
        ProtectionPolicy::new(),
    );

    let advice = index.advise_path(&root);

    assert_eq!(advice.status, CleanupAdviceStatus::ContainsCleanable);
    assert_eq!(advice.relation, Some(CleanupAdviceRelation::Ancestor));
    assert_eq!(advice.matched_path.as_deref(), Some(target.as_path()));
}

#[test]
fn project_artifact_candidate_is_cleanable_when_context_is_verified() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    let target = root.join("node_modules");
    write_fixture_file(root.join("package.json"), b"{}");
    write_fixture_file(target.join(".cache").join("entry.bin"), b"abcdef");

    let mut index = build_index(&[], ProtectionPolicy::new());
    index.add_project_artifact_candidates(discover_artifacts(root.clone()), 0);

    let advice = index.advise_path(&target);

    assert_eq!(advice.status, CleanupAdviceStatus::Cleanable);
    assert_eq!(advice.source.unwrap().label(), "project-artifact");
    assert_eq!(
        advice.rule_id.as_deref(),
        Some("windows.project-artifact-node-modules")
    );
    assert_eq!(advice.category.as_deref(), Some("project-artifact"));
    let args = &advice.suggested_command.as_ref().unwrap().args;
    assert_eq!(args[0], "purge");
    assert_eq!(args[1], "--dry-run");
    assert_eq!(args[2], "--root");
    assert_eq!(args[3], root.display().to_string());
    assert_eq!(args[4], "--artifact");
    assert_eq!(args[5], "node_modules");
}

#[test]
fn bare_project_artifact_name_without_context_remains_unknown() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    let target = root.join("node_modules");
    write_fixture_file(target.join(".cache").join("entry.bin"), b"abcdef");

    let mut index = build_index(&[], ProtectionPolicy::new());
    index.add_project_artifact_candidates(discover_artifacts(root), 0);

    let advice = index.advise_path(&target);

    assert_eq!(advice.status, CleanupAdviceStatus::Unknown);
}

#[test]
fn protected_path_wins_over_rule_match() {
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("cache");
    let protected_paths = vec![target.clone()];
    let policy = ProtectionPolicy::new().with_protected_paths(&protected_paths);
    let index = build_index(&[rule("test.cache", &target, SafetyLevel::Safe)], policy);

    let advice = index.advise_path(&target);

    assert_eq!(advice.status, CleanupAdviceStatus::Protected);
    assert_eq!(
        advice.protection_kind.as_deref(),
        Some("user-protected-path")
    );
    assert_eq!(advice.rule_id, None);
    assert_eq!(advice.suggested_command, None);
}

#[test]
fn unmatched_path_is_unknown() {
    let temp = tempfile::tempdir().unwrap();
    let index = build_index(&[], ProtectionPolicy::new());

    let advice = index.advise_path(&temp.path().join("unknown"));

    assert_eq!(advice.status, CleanupAdviceStatus::Unknown);
    assert_eq!(advice.source, None);
    assert_eq!(advice.suggested_command, None);
}

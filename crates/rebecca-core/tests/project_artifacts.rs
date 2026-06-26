use std::fs;
use std::path::Path;

use rebecca_core::applications::NoopApplicationDiscovery;
use rebecca_core::environment::SystemEnvironment;
use rebecca_core::plan::CleanupTargetIssueReason;
use rebecca_core::planner::{PlanBuildContext, build_cleanup_plan_with_context};
use rebecca_core::project_artifacts::{ProjectArtifactScanOptions, discover_project_artifacts};
use rebecca_core::scan::ScanCancellationToken;
use rebecca_core::{CleanupWorkflow, DeleteMode, PlanRequest, Platform, TargetStatus};

fn write_fixture_file(path: impl AsRef<Path>, bytes: &[u8]) {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, bytes).unwrap();
}

#[test]
fn discovers_known_project_artifacts_and_prunes_nested_artifacts() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    write_fixture_file(
        workspace.join("app").join("node_modules").join("pkg.bin"),
        b"abc",
    );
    write_fixture_file(
        workspace
            .join("app")
            .join("target")
            .join("node_modules")
            .join("nested.bin"),
        b"nested",
    );
    write_fixture_file(workspace.join("app").join("target.txt"), b"keep");
    write_fixture_file(
        workspace.join("app").join("vendor").join("dep.bin"),
        b"keep",
    );

    let options = ProjectArtifactScanOptions::new(vec![workspace]).with_max_depth(4);
    let artifacts = discover_project_artifacts(&options, &ScanCancellationToken::new()).unwrap();
    let paths = artifacts
        .iter()
        .map(|artifact| artifact.path.clone())
        .collect::<Vec<_>>();

    assert_eq!(paths.len(), 2);
    assert!(
        paths
            .iter()
            .any(|path| path.ends_with(Path::new("app").join("node_modules")))
    );
    assert!(
        paths
            .iter()
            .any(|path| path.ends_with(Path::new("app").join("target")))
    );
    assert!(
        paths
            .iter()
            .all(|path| !path.ends_with(Path::new("target").join("node_modules")))
    );
}

#[test]
fn project_artifact_scan_respects_max_depth() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    write_fixture_file(
        workspace
            .join("level1")
            .join("level2")
            .join("node_modules")
            .join("pkg.bin"),
        b"abc",
    );

    let shallow = ProjectArtifactScanOptions::new(vec![workspace.clone()]).with_max_depth(1);
    assert!(
        discover_project_artifacts(&shallow, &ScanCancellationToken::new())
            .unwrap()
            .is_empty()
    );

    let deep = ProjectArtifactScanOptions::new(vec![workspace]).with_max_depth(3);
    assert_eq!(
        discover_project_artifacts(&deep, &ScanCancellationToken::new())
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn project_artifact_plan_measures_allowed_targets_and_blocks_user_protected_paths() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    let node_modules = workspace.join("app").join("node_modules");
    let target = workspace.join("app").join("target");
    write_fixture_file(node_modules.join("pkg.bin"), b"abc");
    write_fixture_file(target.join("debug").join("app.bin"), b"blocked");

    let mut request = PlanRequest::for_platform(Platform::Windows, DeleteMode::DryRun)
        .with_workflow(CleanupWorkflow::ProjectArtifacts);
    request.project_artifact_roots = vec![workspace];
    request.project_artifact_max_depth = 4;
    request.project_artifact_min_age_days = 0;
    let protected_paths = vec![target.clone()];
    let cancellation = ScanCancellationToken::new();
    let applications = NoopApplicationDiscovery::new();

    let plan = build_cleanup_plan_with_context(
        &request,
        &[],
        &SystemEnvironment,
        &applications,
        PlanBuildContext::new(&cancellation).with_protected_paths(&protected_paths),
        |_| {},
    )
    .unwrap();

    assert_eq!(plan.summary.total_targets, 2);
    assert_eq!(plan.summary.allowed_targets, 1);
    assert_eq!(plan.summary.blocked_targets, 1);
    assert_eq!(plan.summary.estimated_bytes, 3);

    let allowed = plan
        .targets
        .iter()
        .find(|target| target.status == TargetStatus::Allowed)
        .unwrap();
    assert_eq!(allowed.rule_id, "windows.project-artifact-node-modules");
    assert!(allowed.restore_hint.is_some());

    let blocked = plan
        .targets
        .iter()
        .find(|target| target.status == TargetStatus::Blocked)
        .unwrap();
    assert_eq!(blocked.path, target);
    assert_eq!(
        blocked.reason_code,
        Some(CleanupTargetIssueReason::SafetyPolicyBlocked)
    );
    assert!(
        blocked
            .reason
            .as_deref()
            .is_some_and(|reason| reason.contains("user-protected path"))
    );
}

#[test]
fn project_artifact_plan_skips_recent_targets_before_sizing() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    write_fixture_file(
        workspace.join("app").join("node_modules").join("pkg.bin"),
        b"abc",
    );

    let mut request = PlanRequest::for_platform(Platform::Windows, DeleteMode::DryRun)
        .with_workflow(CleanupWorkflow::ProjectArtifacts);
    request.project_artifact_roots = vec![workspace];
    request.project_artifact_max_depth = 4;
    let cancellation = ScanCancellationToken::new();
    let applications = NoopApplicationDiscovery::new();

    let plan = build_cleanup_plan_with_context(
        &request,
        &[],
        &SystemEnvironment,
        &applications,
        PlanBuildContext::new(&cancellation),
        |_| {},
    )
    .unwrap();

    assert_eq!(plan.summary.total_targets, 1);
    assert_eq!(plan.summary.allowed_targets, 0);
    assert_eq!(plan.summary.skipped_targets, 1);
    assert_eq!(plan.summary.estimated_bytes, 0);
    assert_eq!(plan.targets[0].status, TargetStatus::Skipped);
    assert_eq!(
        plan.targets[0].reason_code,
        Some(CleanupTargetIssueReason::ProjectArtifactRecentlyModified)
    );
    assert!(
        plan.targets[0]
            .reason
            .as_deref()
            .is_some_and(|reason| reason.contains("modified within the last 7 days"))
    );
}

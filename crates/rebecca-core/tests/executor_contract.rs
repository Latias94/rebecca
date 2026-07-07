use std::cell::Cell;
use std::fs;
use std::path::PathBuf;
use std::sync::Barrier;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use rebecca_core::config::AppPaths;
use rebecca_core::executor::{
    CleanupBackend, ExecutionOutcome, execute_cleanup_plan,
    execute_cleanup_plan_parallel_with_policy, execute_cleanup_plan_with_policy,
};
use rebecca_core::plan::{CleanupPlan, CleanupTarget, CleanupTargetIssueReason};
use rebecca_core::protection::ProtectionPolicy;
use rebecca_core::safety_catalog::default_safety_knowledge_for_platform;
use rebecca_core::{CleanupWorkflow, DeleteMode, PlanRequest, Platform, Result, TargetStatus};

#[test]
fn executor_marks_allowed_targets_completed_and_keeps_blocked_targets() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("file.tmp");
    fs::write(&file, b"trash").unwrap();
    let mut plan = CleanupPlan::empty(PlanRequest::for_platform(
        Platform::Windows,
        DeleteMode::RecoverableDelete,
    ));
    plan.targets.push(CleanupTarget::allowed(
        "windows.user-temp",
        file,
        10,
        DeleteMode::RecoverableDelete,
    ));
    plan.targets.push(CleanupTarget::blocked_with_reason_code(
        "windows.user-temp",
        PathBuf::from("C:/Windows"),
        DeleteMode::RecoverableDelete,
        CleanupTargetIssueReason::SafetyPolicyBlocked,
        "protected",
    ));
    plan.recompute_summary();

    let backend = FakeBackend::success();
    let report = execute_cleanup_plan(&mut plan, &backend).unwrap();

    assert_eq!(backend.calls.get(), 1);
    assert_eq!(plan.execution_report.as_ref(), Some(&report));
    assert_eq!(plan.targets[0].status, TargetStatus::Completed);
    assert_eq!(plan.targets[0].pending_reclaim_bytes, 10);
    assert_eq!(plan.targets[1].status, TargetStatus::Blocked);
    assert_eq!(report.summary.completed_actions, 1);
    assert_eq!(report.summary.blocked_actions, 1);
    assert_eq!(
        report.actions[0].attempted_paths,
        vec![plan.targets[0].path.clone()]
    );
}

#[test]
fn executor_records_failure_without_aborting_plan() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("file.tmp");
    fs::write(&file, b"trash").unwrap();
    let mut plan = CleanupPlan::empty(PlanRequest::for_platform(
        Platform::Windows,
        DeleteMode::RecoverableDelete,
    ));
    plan.targets.push(CleanupTarget::allowed(
        "windows.user-temp",
        file,
        10,
        DeleteMode::RecoverableDelete,
    ));
    plan.recompute_summary();

    let backend = FakeBackend::failure("backend unavailable");
    execute_cleanup_plan(&mut plan, &backend).unwrap();

    assert_eq!(plan.targets[0].status, TargetStatus::Failed);
    assert_eq!(
        plan.targets[0].reason_code,
        Some(CleanupTargetIssueReason::ExecutionFailed)
    );
    assert_eq!(plan.summary.failed_targets, 1);
    assert_eq!(
        plan.summary.issue_matrix[0].reason_code,
        CleanupTargetIssueReason::ExecutionFailed
    );
}

#[test]
fn executor_records_permission_failure_reason_without_aborting_plan() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("file.tmp");
    fs::write(&file, b"trash").unwrap();
    let mut plan = CleanupPlan::empty(PlanRequest::for_platform(
        Platform::Windows,
        DeleteMode::RecoverableDelete,
    ));
    plan.targets.push(CleanupTarget::allowed(
        "windows.user-temp",
        file,
        10,
        DeleteMode::RecoverableDelete,
    ));
    plan.recompute_summary();

    let backend = FakeBackend::failure("permission denied");
    execute_cleanup_plan(&mut plan, &backend).unwrap();

    assert_eq!(plan.targets[0].status, TargetStatus::Failed);
    assert_eq!(
        plan.targets[0].reason_code,
        Some(CleanupTargetIssueReason::ExecutionPermissionDenied)
    );
    assert_eq!(plan.summary.failed_targets, 1);
    assert_eq!(
        plan.summary.issue_matrix[0].reason_code,
        CleanupTargetIssueReason::ExecutionPermissionDenied
    );
}

#[test]
fn executor_default_policy_uses_plan_platform_safety_knowledge() {
    let mut plan = CleanupPlan::empty(PlanRequest::for_platform(
        Platform::Macos,
        DeleteMode::RecoverableDelete,
    ));
    plan.targets.push(CleanupTarget::allowed(
        "macos.unsafe-system",
        PathBuf::from("/System/Library/Caches"),
        10,
        DeleteMode::RecoverableDelete,
    ));
    plan.recompute_summary();

    let backend = FakeBackend::success();
    let report = execute_cleanup_plan(&mut plan, &backend).unwrap();

    assert_eq!(backend.calls.get(), 0);
    assert_eq!(plan.targets[0].status, TargetStatus::Blocked);
    assert_eq!(
        plan.targets[0].reason_code,
        Some(CleanupTargetIssueReason::SafetyPolicyBlocked)
    );
    assert!(
        plan.targets[0]
            .reason
            .as_deref()
            .is_some_and(|reason| reason.contains("critical macos path"))
    );
    assert_eq!(report.summary.blocked_actions, 1);
    assert_eq!(plan.summary.blocked_targets, 1);
}

#[test]
fn executor_revalidates_targets_with_supplied_platform_safety_knowledge() {
    let mut plan = CleanupPlan::empty(PlanRequest::for_platform(
        Platform::Linux,
        DeleteMode::RecoverableDelete,
    ));
    plan.targets.push(CleanupTarget::allowed(
        "linux.unsafe-etc",
        PathBuf::from("/etc"),
        10,
        DeleteMode::RecoverableDelete,
    ));
    plan.recompute_summary();
    let linux_knowledge = default_safety_knowledge_for_platform(Platform::Linux)
        .expect("Linux safety knowledge should exist");
    let policy = ProtectionPolicy::new().with_safety_knowledge(linux_knowledge);
    let backend = FakeBackend::success();

    let report = execute_cleanup_plan_with_policy(&mut plan, &backend, policy).unwrap();

    assert_eq!(backend.calls.get(), 0);
    assert_eq!(plan.targets[0].status, TargetStatus::Blocked);
    assert_eq!(
        plan.targets[0].reason_code,
        Some(CleanupTargetIssueReason::SafetyPolicyBlocked)
    );
    assert!(
        plan.targets[0]
            .reason
            .as_deref()
            .is_some_and(|reason| reason.contains("critical linux path"))
    );
    assert_eq!(report.summary.blocked_actions, 1);
    assert_eq!(plan.summary.blocked_targets, 1);
}

#[test]
fn executor_shadows_child_targets_covered_by_parent_delete() {
    let temp = tempfile::tempdir().unwrap();
    let parent = temp.path().join("cache");
    let child = parent.join("child");
    fs::create_dir_all(&child).unwrap();
    let mut plan = CleanupPlan::empty(PlanRequest::for_platform(
        Platform::Windows,
        DeleteMode::RecoverableDelete,
    ));
    plan.targets.push(CleanupTarget::allowed(
        "windows.parent",
        parent,
        100,
        DeleteMode::RecoverableDelete,
    ));
    plan.targets.push(CleanupTarget::allowed(
        "windows.child",
        child,
        40,
        DeleteMode::RecoverableDelete,
    ));
    plan.recompute_summary();

    let backend = FakeBackend::success();
    let report = execute_cleanup_plan(&mut plan, &backend).unwrap();

    assert_eq!(backend.calls.get(), 1);
    assert_eq!(plan.execution_report.as_ref(), Some(&report));
    assert_eq!(plan.targets[0].status, TargetStatus::Completed);
    assert_eq!(plan.targets[1].status, TargetStatus::Skipped);
    assert_eq!(
        plan.targets[1].reason_code,
        Some(CleanupTargetIssueReason::ExecutionTargetShadowed)
    );
    assert_eq!(plan.summary.completed_targets, 1);
    assert_eq!(plan.summary.skipped_targets, 1);
    assert_eq!(report.summary.completed_actions, 1);
    assert_eq!(report.summary.skipped_actions, 1);
    assert_eq!(report.summary.shadowed_bytes, 40);
}

#[test]
fn executor_shadowing_is_order_independent_for_nested_targets() {
    let temp = tempfile::tempdir().unwrap();
    let parent = temp.path().join("cache");
    let child = parent.join("child");
    let grandchild = child.join("grandchild");
    fs::create_dir_all(&grandchild).unwrap();

    let mut plan = CleanupPlan::empty(PlanRequest::for_platform(
        Platform::Windows,
        DeleteMode::RecoverableDelete,
    ));
    plan.targets.push(CleanupTarget::allowed(
        "windows.child",
        child,
        10,
        DeleteMode::RecoverableDelete,
    ));
    plan.targets.push(CleanupTarget::allowed(
        "windows.grandchild",
        grandchild,
        20,
        DeleteMode::RecoverableDelete,
    ));
    plan.targets.push(CleanupTarget::allowed(
        "windows.parent",
        parent,
        100,
        DeleteMode::RecoverableDelete,
    ));
    plan.recompute_summary();

    let backend = FakeBackend::success();
    let report = execute_cleanup_plan(&mut plan, &backend).unwrap();

    assert_eq!(backend.calls.get(), 1);
    assert_eq!(plan.targets[0].status, TargetStatus::Skipped);
    assert_eq!(plan.targets[1].status, TargetStatus::Skipped);
    assert_eq!(plan.targets[2].status, TargetStatus::Completed);
    assert_eq!(
        plan.targets[0].reason_code,
        Some(CleanupTargetIssueReason::ExecutionTargetShadowed)
    );
    assert_eq!(
        plan.targets[1].reason_code,
        Some(CleanupTargetIssueReason::ExecutionTargetShadowed)
    );
    assert!(
        plan.targets[0]
            .reason
            .as_deref()
            .unwrap()
            .contains("windows.parent")
    );
    assert!(
        plan.targets[1]
            .reason
            .as_deref()
            .unwrap()
            .contains("windows.parent")
    );
    assert_eq!(report.summary.completed_actions, 1);
    assert_eq!(report.summary.skipped_actions, 2);
    assert_eq!(report.summary.shadowed_bytes, 30);
}

#[test]
fn executor_revalidates_protected_category_targets_before_backend_calls() {
    let mut plan = CleanupPlan::empty(PlanRequest::for_platform(
        Platform::Windows,
        DeleteMode::RecoverableDelete,
    ));
    plan.targets.push(CleanupTarget::allowed(
        "windows.custom-browser-history",
        PathBuf::from("C:/Users/Alice/AppData/Local/Google/Chrome/User Data/Default/History"),
        10,
        DeleteMode::RecoverableDelete,
    ));
    plan.recompute_summary();

    let backend = FakeBackend::success();
    execute_cleanup_plan_with_policy(&mut plan, &backend, ProtectionPolicy::new()).unwrap();

    assert_eq!(backend.calls.get(), 0);
    assert_eq!(plan.targets[0].status, TargetStatus::Blocked);
    assert_eq!(
        plan.targets[0].reason_code,
        Some(CleanupTargetIssueReason::SafetyPolicyBlocked)
    );
    assert!(
        plan.targets[0]
            .reason
            .as_deref()
            .unwrap()
            .contains("browser private data")
    );
    assert_eq!(plan.summary.blocked_targets, 1);
}

#[test]
fn executor_revalidates_rebecca_owned_storage_before_backend_calls() {
    let app_paths = AppPaths {
        config_dir: PathBuf::from("C:/Users/Alice/AppData/Roaming/Rebecca"),
        config_file: PathBuf::from("C:/Users/Alice/AppData/Roaming/Rebecca/config.toml"),
        state_dir: PathBuf::from("C:/Users/Alice/AppData/Local/Rebecca/state"),
        cache_dir: PathBuf::from("C:/Users/Alice/AppData/Local/Rebecca/cache"),
        history_file: PathBuf::from("C:/Users/Alice/AppData/Local/Rebecca/state/history.jsonl"),
    };
    let protected_storage = app_paths.storage_entries();
    let mut plan = CleanupPlan::empty(PlanRequest::for_platform(
        Platform::Windows,
        DeleteMode::RecoverableDelete,
    ));
    plan.targets.push(CleanupTarget::allowed(
        "windows.custom-rebecca-cache",
        app_paths.cache_dir.join("scan"),
        10,
        DeleteMode::RecoverableDelete,
    ));
    plan.recompute_summary();

    let backend = FakeBackend::success();
    let policy = ProtectionPolicy::new().with_protected_storage(&protected_storage);
    execute_cleanup_plan_with_policy(&mut plan, &backend, policy).unwrap();

    assert_eq!(backend.calls.get(), 0);
    assert_eq!(plan.targets[0].status, TargetStatus::Blocked);
    assert_eq!(
        plan.targets[0].reason_code,
        Some(CleanupTargetIssueReason::SafetyPolicyBlocked)
    );
    assert!(
        plan.targets[0]
            .reason
            .as_deref()
            .unwrap()
            .contains("Rebecca-owned Cache dir")
    );
    assert_eq!(plan.summary.blocked_targets, 1);
}

#[test]
fn executor_revalidates_user_protected_paths_before_backend_calls() {
    let temp = tempfile::tempdir().unwrap();
    let cache_dir = temp.path().join("Slack").join("Cache");
    fs::create_dir_all(&cache_dir).unwrap();
    fs::write(cache_dir.join("cache.bin"), b"trash").unwrap();
    let protected_paths = vec![cache_dir.clone()];
    let mut plan = CleanupPlan::empty(PlanRequest::for_platform(
        Platform::Windows,
        DeleteMode::RecoverableDelete,
    ));
    plan.targets.push(CleanupTarget::allowed(
        "windows.slack-cache",
        cache_dir,
        5,
        DeleteMode::RecoverableDelete,
    ));
    plan.recompute_summary();

    let backend = FakeBackend::success();
    let policy = ProtectionPolicy::new().with_protected_paths(&protected_paths);
    execute_cleanup_plan_with_policy(&mut plan, &backend, policy).unwrap();

    assert_eq!(backend.calls.get(), 0);
    assert_eq!(plan.targets[0].status, TargetStatus::Blocked);
    assert_eq!(
        plan.targets[0].reason_code,
        Some(CleanupTargetIssueReason::SafetyPolicyBlocked)
    );
    assert!(
        plan.targets[0]
            .reason
            .as_deref()
            .unwrap()
            .contains("user-protected path")
    );
    assert_eq!(plan.summary.blocked_targets, 1);
}

#[test]
fn executor_revalidates_project_artifact_targets_before_backend_calls() {
    let temp = tempfile::tempdir().unwrap();
    let target_dir = temp.path().join("project").join("target");
    fs::create_dir_all(&target_dir).unwrap();
    fs::write(target_dir.join("cache.bin"), b"trash").unwrap();
    let protected_paths = vec![target_dir.clone()];
    let mut request = PlanRequest::for_platform(Platform::Windows, DeleteMode::RecoverableDelete);
    request.workflow = CleanupWorkflow::ProjectArtifacts;
    let mut plan = CleanupPlan::empty(request);
    plan.targets.push(CleanupTarget::allowed(
        "portable.project-artifact-target",
        target_dir,
        5,
        DeleteMode::RecoverableDelete,
    ));
    plan.recompute_summary();

    let backend = FakeBackend::success();
    let policy = ProtectionPolicy::new().with_protected_paths(&protected_paths);
    execute_cleanup_plan_with_policy(&mut plan, &backend, policy).unwrap();

    assert_eq!(backend.calls.get(), 0);
    assert_eq!(plan.targets[0].status, TargetStatus::Blocked);
    assert_eq!(
        plan.targets[0].reason_code,
        Some(CleanupTargetIssueReason::SafetyPolicyBlocked)
    );
}

#[test]
fn executor_skips_missing_targets_before_backend_calls() {
    let temp = tempfile::tempdir().unwrap();
    let missing_file = temp.path().join("definitely-missing.tmp");
    let mut plan = CleanupPlan::empty(PlanRequest::for_platform(
        Platform::Windows,
        DeleteMode::RecoverableDelete,
    ));
    plan.targets.push(CleanupTarget::allowed(
        "windows.user-temp",
        missing_file,
        10,
        DeleteMode::RecoverableDelete,
    ));
    plan.recompute_summary();

    let backend = FakeBackend::success();
    execute_cleanup_plan_with_policy(&mut plan, &backend, ProtectionPolicy::new()).unwrap();

    assert_eq!(backend.calls.get(), 0);
    assert_eq!(plan.targets[0].status, TargetStatus::Skipped);
    assert_eq!(
        plan.targets[0].reason_code,
        Some(CleanupTargetIssueReason::ExecutionTargetMissing)
    );
    assert!(
        plan.targets[0]
            .reason
            .as_deref()
            .unwrap()
            .contains("path does not exist")
    );
    assert_eq!(plan.summary.skipped_targets, 1);
    assert_eq!(
        plan.summary.issue_matrix[0].reason_code,
        CleanupTargetIssueReason::ExecutionTargetMissing
    );
}

#[test]
fn executor_allows_app_leftover_cache_targets_after_revalidation() {
    let temp = tempfile::tempdir().unwrap();
    let cache_dir = temp
        .path()
        .join("AppData")
        .join("Local")
        .join("Example App")
        .join("Cache");
    fs::create_dir_all(&cache_dir).unwrap();
    fs::write(cache_dir.join("cache.bin"), b"trash").unwrap();
    let mut plan = CleanupPlan::empty(
        PlanRequest::for_platform(Platform::Windows, DeleteMode::RecoverableDelete)
            .with_workflow(CleanupWorkflow::AppLeftovers),
    );
    plan.targets.push(CleanupTarget::allowed(
        "windows.app-leftover-local-cache",
        cache_dir,
        5,
        DeleteMode::RecoverableDelete,
    ));
    plan.recompute_summary();

    let backend = FakeBackend::success();
    execute_cleanup_plan_with_policy(&mut plan, &backend, ProtectionPolicy::new()).unwrap();

    assert_eq!(backend.calls.get(), 1);
    assert_eq!(plan.targets[0].status, TargetStatus::Completed);
    assert_eq!(plan.targets[0].pending_reclaim_bytes, 5);
}

#[test]
fn executor_skips_missing_app_leftover_targets_before_backend_calls() {
    let temp = tempfile::tempdir().unwrap();
    let missing_cache_dir = temp
        .path()
        .join("AppData")
        .join("Local")
        .join("Example App")
        .join("Cache");
    let mut plan = CleanupPlan::empty(
        PlanRequest::for_platform(Platform::Windows, DeleteMode::RecoverableDelete)
            .with_workflow(CleanupWorkflow::AppLeftovers),
    );
    plan.targets.push(CleanupTarget::allowed(
        "windows.app-leftover-local-cache",
        missing_cache_dir,
        5,
        DeleteMode::RecoverableDelete,
    ));
    plan.recompute_summary();

    let backend = FakeBackend::success();
    execute_cleanup_plan_with_policy(&mut plan, &backend, ProtectionPolicy::new()).unwrap();

    assert_eq!(backend.calls.get(), 0);
    assert_eq!(plan.targets[0].status, TargetStatus::Skipped);
    assert_eq!(
        plan.targets[0].reason_code,
        Some(CleanupTargetIssueReason::ExecutionTargetMissing)
    );
    assert_eq!(
        plan.targets[0].reason.as_deref(),
        Some("path does not exist")
    );
    assert_eq!(plan.summary.skipped_targets, 1);
}

#[test]
fn executor_parallel_batches_independent_targets() {
    let temp = tempfile::tempdir().unwrap();
    let first = temp.path().join("first.tmp");
    let second = temp.path().join("second.tmp");
    fs::write(&first, b"trash").unwrap();
    fs::write(&second, b"trash").unwrap();
    let mut plan = CleanupPlan::empty(PlanRequest::for_platform(
        Platform::Windows,
        DeleteMode::RecoverableDelete,
    ));
    plan.targets.push(CleanupTarget::allowed(
        "windows.first",
        first,
        10,
        DeleteMode::RecoverableDelete,
    ));
    plan.targets.push(CleanupTarget::allowed(
        "windows.second",
        second,
        20,
        DeleteMode::RecoverableDelete,
    ));
    plan.recompute_summary();

    let backend = Arc::new(BlockingBackend::new());
    let backend_for_thread = Arc::clone(&backend);
    let delete_thread = thread::spawn(move || {
        execute_cleanup_plan_parallel_with_policy(
            &mut plan,
            backend_for_thread.as_ref(),
            ProtectionPolicy::new(),
        )
        .unwrap();
        plan
    });

    let deadline = Instant::now() + Duration::from_secs(1);
    while backend.started.load(Ordering::SeqCst) < 2 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }

    assert_eq!(
        backend.started.load(Ordering::SeqCst),
        2,
        "expected both deletes to be in flight at once"
    );
    backend.gate.wait();

    let plan = delete_thread.join().unwrap();
    assert_eq!(backend.max_active.load(Ordering::SeqCst), 2);
    assert_eq!(plan.summary.completed_targets, 2);
    assert_eq!(plan.targets[0].status, TargetStatus::Completed);
    assert_eq!(plan.targets[1].status, TargetStatus::Completed);
}

#[test]
fn executor_parallel_passes_safe_batches_to_batch_backend() {
    let temp = tempfile::tempdir().unwrap();
    let parent = temp.path().join("parent");
    let child = parent.join("child");
    let sibling = temp.path().join("sibling");
    fs::create_dir_all(&child).unwrap();
    fs::create_dir_all(&sibling).unwrap();
    let mut plan = CleanupPlan::empty(PlanRequest::for_platform(
        Platform::Windows,
        DeleteMode::RecoverableDelete,
    ));
    plan.targets.push(CleanupTarget::allowed(
        "windows.parent",
        parent.clone(),
        10,
        DeleteMode::RecoverableDelete,
    ));
    plan.targets.push(CleanupTarget::allowed(
        "windows.child",
        child.clone(),
        20,
        DeleteMode::RecoverableDelete,
    ));
    plan.targets.push(CleanupTarget::allowed(
        "windows.sibling",
        sibling.clone(),
        30,
        DeleteMode::RecoverableDelete,
    ));
    plan.recompute_summary();

    let backend = BatchBackend::new(BatchBehavior::Success);
    execute_cleanup_plan_parallel_with_policy(&mut plan, &backend, ProtectionPolicy::new())
        .unwrap();

    assert_eq!(backend.single_deletes.load(Ordering::SeqCst), 0);
    assert_eq!(plan.summary.completed_targets, 2);
    assert_eq!(plan.summary.skipped_targets, 1);
    assert_eq!(plan.summary.failed_targets, 0);
    assert_eq!(
        plan.targets[1].reason_code,
        Some(CleanupTargetIssueReason::ExecutionTargetShadowed)
    );
    let batches = backend.batches.lock().unwrap().clone();
    assert_eq!(batches.len(), 1);
    let mut actual_batch = batches[0].clone();
    actual_batch.sort();
    let mut expected_batch = vec![parent, sibling];
    expected_batch.sort();
    assert_eq!(actual_batch, expected_batch);
}

#[test]
fn executor_parallel_maps_partial_batch_failures_per_target() {
    let temp = tempfile::tempdir().unwrap();
    let first = temp.path().join("first.tmp");
    let second = temp.path().join("second.tmp");
    fs::write(&first, b"trash").unwrap();
    fs::write(&second, b"trash").unwrap();
    let mut plan = CleanupPlan::empty(PlanRequest::for_platform(
        Platform::Windows,
        DeleteMode::RecoverableDelete,
    ));
    plan.targets.push(CleanupTarget::allowed(
        "windows.first",
        first,
        10,
        DeleteMode::RecoverableDelete,
    ));
    plan.targets.push(CleanupTarget::allowed(
        "windows.second",
        second,
        20,
        DeleteMode::RecoverableDelete,
    ));
    plan.recompute_summary();

    let backend = BatchBackend::new(BatchBehavior::FailSecond);
    execute_cleanup_plan_parallel_with_policy(&mut plan, &backend, ProtectionPolicy::new())
        .unwrap();

    assert_eq!(plan.targets[0].status, TargetStatus::Completed);
    assert_eq!(plan.targets[0].pending_reclaim_bytes, 10);
    assert_eq!(plan.targets[1].status, TargetStatus::Failed);
    assert_eq!(
        plan.targets[1].reason_code,
        Some(CleanupTargetIssueReason::ExecutionPermissionDenied)
    );
    assert_eq!(plan.summary.completed_targets, 1);
    assert_eq!(plan.summary.failed_targets, 1);
    assert_eq!(
        plan.summary.issue_matrix[0].reason_code,
        CleanupTargetIssueReason::ExecutionPermissionDenied
    );
}

#[test]
fn executor_parallel_rejects_mismatched_batch_outcome_count() {
    let temp = tempfile::tempdir().unwrap();
    let first = temp.path().join("first.tmp");
    let second = temp.path().join("second.tmp");
    fs::write(&first, b"trash").unwrap();
    fs::write(&second, b"trash").unwrap();
    let mut plan = CleanupPlan::empty(PlanRequest::for_platform(
        Platform::Windows,
        DeleteMode::RecoverableDelete,
    ));
    plan.targets.push(CleanupTarget::allowed(
        "windows.first",
        first,
        10,
        DeleteMode::RecoverableDelete,
    ));
    plan.targets.push(CleanupTarget::allowed(
        "windows.second",
        second,
        20,
        DeleteMode::RecoverableDelete,
    ));
    plan.recompute_summary();

    let backend = BatchBackend::new(BatchBehavior::MismatchedOutcomeCount);
    execute_cleanup_plan_parallel_with_policy(&mut plan, &backend, ProtectionPolicy::new())
        .unwrap();

    assert_eq!(plan.summary.completed_targets, 0);
    assert_eq!(plan.summary.failed_targets, 2);
    assert!(plan.targets.iter().all(|target| {
        target.status == TargetStatus::Failed
            && target.reason_code == Some(CleanupTargetIssueReason::ExecutionFailed)
            && target
                .reason
                .as_deref()
                .is_some_and(|reason| reason.contains("returned 1 outcome(s) for 2 target(s)"))
    }));
}

#[test]
fn executor_blocks_app_leftover_durable_paths_before_backend_calls() {
    let mut plan = CleanupPlan::empty(
        PlanRequest::for_platform(Platform::Windows, DeleteMode::RecoverableDelete)
            .with_workflow(CleanupWorkflow::AppLeftovers),
    );
    plan.targets.push(CleanupTarget::allowed(
        "windows.app-leftover-local-cache",
        PathBuf::from("C:/Users/Alice/AppData/Local/Example App/Local Storage"),
        10,
        DeleteMode::RecoverableDelete,
    ));
    plan.recompute_summary();

    let backend = FakeBackend::success();
    execute_cleanup_plan_with_policy(&mut plan, &backend, ProtectionPolicy::new()).unwrap();

    assert_eq!(backend.calls.get(), 0);
    assert_eq!(plan.targets[0].status, TargetStatus::Blocked);
    assert_eq!(
        plan.targets[0].reason_code,
        Some(CleanupTargetIssueReason::SafetyPolicyBlocked)
    );
    assert!(
        plan.targets[0]
            .reason
            .as_deref()
            .unwrap()
            .contains("application durable data")
    );
    assert_eq!(plan.summary.blocked_targets, 1);
}

struct FakeBackend {
    calls: Cell<usize>,
    failure_message: Option<&'static str>,
}

impl FakeBackend {
    fn success() -> Self {
        Self {
            calls: Cell::new(0),
            failure_message: None,
        }
    }

    fn failure(message: &'static str) -> Self {
        Self {
            calls: Cell::new(0),
            failure_message: Some(message),
        }
    }
}

impl CleanupBackend for FakeBackend {
    fn delete(&self, target: &CleanupTarget) -> Result<ExecutionOutcome> {
        self.calls.set(self.calls.get() + 1);
        if let Some(message) = self.failure_message {
            return Err(rebecca_core::RebeccaError::ExecutionFailed(
                message.to_string(),
            ));
        }

        Ok(ExecutionOutcome {
            freed_bytes: 0,
            pending_reclaim_bytes: target.estimated_bytes,
            note: Some("fake delete".to_string()),
        })
    }
}

struct BlockingBackend {
    started: AtomicUsize,
    max_active: AtomicUsize,
    gate: Arc<Barrier>,
}

#[derive(Debug, Clone, Copy)]
enum BatchBehavior {
    Success,
    FailSecond,
    MismatchedOutcomeCount,
}

struct BatchBackend {
    batches: Mutex<Vec<Vec<PathBuf>>>,
    single_deletes: AtomicUsize,
    behavior: BatchBehavior,
}

impl BatchBackend {
    fn new(behavior: BatchBehavior) -> Self {
        Self {
            batches: Mutex::new(Vec::new()),
            single_deletes: AtomicUsize::new(0),
            behavior,
        }
    }
}

impl CleanupBackend for BatchBackend {
    fn delete(&self, _target: &CleanupTarget) -> Result<ExecutionOutcome> {
        self.single_deletes.fetch_add(1, Ordering::SeqCst);
        Err(rebecca_core::RebeccaError::ExecutionFailed(
            "single delete should not be called".to_string(),
        ))
    }

    fn supports_batch_delete(&self) -> bool {
        true
    }

    fn delete_batch(&self, targets: &[&CleanupTarget]) -> Vec<Result<ExecutionOutcome>> {
        self.batches.lock().unwrap().push(
            targets
                .iter()
                .map(|target| target.path.clone())
                .collect::<Vec<_>>(),
        );

        match self.behavior {
            BatchBehavior::Success => targets.iter().map(|target| batch_success(target)).collect(),
            BatchBehavior::FailSecond => targets
                .iter()
                .enumerate()
                .map(|(index, target)| {
                    if index == 1 {
                        Err(rebecca_core::RebeccaError::ExecutionFailed(
                            "permission denied".to_string(),
                        ))
                    } else {
                        batch_success(target)
                    }
                })
                .collect(),
            BatchBehavior::MismatchedOutcomeCount => vec![batch_success(targets[0])],
        }
    }
}

fn batch_success(target: &CleanupTarget) -> Result<ExecutionOutcome> {
    Ok(ExecutionOutcome {
        freed_bytes: 0,
        pending_reclaim_bytes: target.estimated_bytes,
        note: Some("batch delete".to_string()),
    })
}

impl BlockingBackend {
    fn new() -> Self {
        Self {
            started: AtomicUsize::new(0),
            max_active: AtomicUsize::new(0),
            gate: Arc::new(Barrier::new(3)),
        }
    }
}

impl CleanupBackend for BlockingBackend {
    fn delete(&self, target: &CleanupTarget) -> Result<ExecutionOutcome> {
        let active = self.started.fetch_add(1, Ordering::SeqCst) + 1;
        loop {
            let current = self.max_active.load(Ordering::SeqCst);
            if active <= current {
                break;
            }
            if self
                .max_active
                .compare_exchange(current, active, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                break;
            }
        }

        self.gate.wait();
        self.started.fetch_sub(1, Ordering::SeqCst);

        Ok(ExecutionOutcome {
            freed_bytes: 0,
            pending_reclaim_bytes: target.estimated_bytes,
            note: Some("blocking delete".to_string()),
        })
    }
}

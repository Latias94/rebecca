use std::cell::Cell;
use std::fs;
use std::path::PathBuf;

use rebecca_core::config::AppPaths;
use rebecca_core::executor::{
    CleanupBackend, ExecutionOutcome, execute_cleanup_plan, execute_cleanup_plan_with_policy,
};
use rebecca_core::plan::{CleanupPlan, CleanupTarget, CleanupTargetIssueReason};
use rebecca_core::protection::ProtectionPolicy;
use rebecca_core::{DeleteMode, PlanRequest, Platform, Result, TargetStatus};

#[test]
fn executor_marks_allowed_targets_completed_and_keeps_blocked_targets() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("file.tmp");
    fs::write(&file, b"trash").unwrap();
    let mut plan = CleanupPlan::empty(PlanRequest::for_platform(
        Platform::Windows,
        DeleteMode::RecycleBin,
    ));
    plan.targets.push(CleanupTarget::allowed(
        "windows.user-temp",
        file,
        10,
        DeleteMode::RecycleBin,
    ));
    plan.targets.push(CleanupTarget::blocked_with_reason_code(
        "windows.user-temp",
        PathBuf::from("C:/Windows"),
        DeleteMode::RecycleBin,
        CleanupTargetIssueReason::SafetyPolicyBlocked,
        "protected",
    ));
    plan.recompute_summary();

    let backend = FakeBackend::success();
    execute_cleanup_plan(&mut plan, &backend).unwrap();

    assert_eq!(backend.calls.get(), 1);
    assert_eq!(plan.targets[0].status, TargetStatus::Completed);
    assert_eq!(plan.targets[0].pending_reclaim_bytes, 10);
    assert_eq!(plan.targets[1].status, TargetStatus::Blocked);
}

#[test]
fn executor_records_failure_without_aborting_plan() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("file.tmp");
    fs::write(&file, b"trash").unwrap();
    let mut plan = CleanupPlan::empty(PlanRequest::for_platform(
        Platform::Windows,
        DeleteMode::RecycleBin,
    ));
    plan.targets.push(CleanupTarget::allowed(
        "windows.user-temp",
        file,
        10,
        DeleteMode::RecycleBin,
    ));
    plan.recompute_summary();

    let backend = FakeBackend::failure();
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
fn executor_revalidates_protected_category_targets_before_backend_calls() {
    let mut plan = CleanupPlan::empty(PlanRequest::for_platform(
        Platform::Windows,
        DeleteMode::RecycleBin,
    ));
    plan.targets.push(CleanupTarget::allowed(
        "windows.custom-browser-history",
        PathBuf::from("C:/Users/Alice/AppData/Local/Google/Chrome/User Data/Default/History"),
        10,
        DeleteMode::RecycleBin,
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
        DeleteMode::RecycleBin,
    ));
    plan.targets.push(CleanupTarget::allowed(
        "windows.custom-rebecca-cache",
        app_paths.cache_dir.join("scan"),
        10,
        DeleteMode::RecycleBin,
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
fn executor_skips_missing_targets_before_backend_calls() {
    let temp = tempfile::tempdir().unwrap();
    let missing_file = temp.path().join("definitely-missing.tmp");
    let mut plan = CleanupPlan::empty(PlanRequest::for_platform(
        Platform::Windows,
        DeleteMode::RecycleBin,
    ));
    plan.targets.push(CleanupTarget::allowed(
        "windows.user-temp",
        missing_file,
        10,
        DeleteMode::RecycleBin,
    ));
    plan.recompute_summary();

    let backend = FakeBackend::success();
    execute_cleanup_plan_with_policy(&mut plan, &backend, ProtectionPolicy::new()).unwrap();

    assert_eq!(backend.calls.get(), 0);
    assert_eq!(plan.targets[0].status, TargetStatus::Skipped);
    assert_eq!(
        plan.targets[0].reason_code,
        Some(CleanupTargetIssueReason::SafetyPolicySkipped)
    );
    assert!(
        plan.targets[0]
            .reason
            .as_deref()
            .unwrap()
            .contains("path does not exist")
    );
    assert_eq!(plan.summary.skipped_targets, 1);
}

struct FakeBackend {
    calls: Cell<usize>,
    fail: bool,
}

impl FakeBackend {
    fn success() -> Self {
        Self {
            calls: Cell::new(0),
            fail: false,
        }
    }

    fn failure() -> Self {
        Self {
            calls: Cell::new(0),
            fail: true,
        }
    }
}

impl CleanupBackend for FakeBackend {
    fn delete(&self, target: &CleanupTarget) -> Result<ExecutionOutcome> {
        self.calls.set(self.calls.get() + 1);
        if self.fail {
            return Err(rebecca_core::RebeccaError::ExecutionFailed(
                "permission denied".to_string(),
            ));
        }

        Ok(ExecutionOutcome {
            freed_bytes: 0,
            pending_reclaim_bytes: target.estimated_bytes,
            note: Some("fake delete".to_string()),
        })
    }
}

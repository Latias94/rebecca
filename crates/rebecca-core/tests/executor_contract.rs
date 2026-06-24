use std::cell::Cell;
use std::path::PathBuf;

use rebecca_core::executor::{CleanupBackend, ExecutionOutcome, execute_cleanup_plan};
use rebecca_core::plan::{CleanupPlan, CleanupTarget, CleanupTargetIssueReason};
use rebecca_core::{DeleteMode, PlanRequest, Platform, Result, TargetStatus};

#[test]
fn executor_marks_allowed_targets_completed_and_keeps_blocked_targets() {
    let mut plan = CleanupPlan::empty(PlanRequest::for_platform(
        Platform::Windows,
        DeleteMode::RecycleBin,
    ));
    plan.targets.push(CleanupTarget::allowed(
        "windows.user-temp",
        PathBuf::from("C:/Temp/file.tmp"),
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
    let mut plan = CleanupPlan::empty(PlanRequest::for_platform(
        Platform::Windows,
        DeleteMode::RecycleBin,
    ));
    plan.targets.push(CleanupTarget::allowed(
        "windows.user-temp",
        PathBuf::from("C:/Temp/file.tmp"),
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

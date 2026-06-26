use crate::error::Result;
use crate::model::CleanupWorkflow;
use crate::plan::{CleanupPlan, CleanupTarget, CleanupTargetIssueReason};
use crate::protection::{AppLeftoverPathDisposition, ProtectionPolicy};
use crate::safety::{PathDisposition, assess_existing_path_with_policy};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionOutcome {
    pub freed_bytes: u64,
    pub pending_reclaim_bytes: u64,
    pub note: Option<String>,
}

pub trait CleanupBackend {
    fn delete(&self, target: &CleanupTarget) -> Result<ExecutionOutcome>;
}

pub fn execute_cleanup_plan<B: CleanupBackend>(plan: &mut CleanupPlan, backend: &B) -> Result<()> {
    execute_cleanup_plan_with_policy(plan, backend, ProtectionPolicy::new())
}

pub fn execute_cleanup_plan_with_policy<B: CleanupBackend>(
    plan: &mut CleanupPlan,
    backend: &B,
    policy: ProtectionPolicy<'_>,
) -> Result<()> {
    if plan.request.mode.is_dry_run() {
        plan.recompute_summary();
        return Ok(());
    }

    for target in &mut plan.targets {
        if !target.status.is_executable() {
            continue;
        }

        if !execution_target_is_still_allowed(plan.request.workflow, target, policy) {
            continue;
        }

        match backend.delete(target) {
            Ok(outcome) => {
                target.status = crate::TargetStatus::Completed;
                target.freed_bytes = outcome.freed_bytes;
                target.pending_reclaim_bytes = outcome.pending_reclaim_bytes;
                target.reason = outcome.note;
                target.reason_code = None;
            }
            Err(err) => {
                target.status = crate::TargetStatus::Failed;
                target.reason = Some(err.to_string());
                target.reason_code = Some(CleanupTargetIssueReason::ExecutionFailed);
                target.freed_bytes = 0;
                target.pending_reclaim_bytes = 0;
            }
        }
    }

    plan.recompute_summary();
    Ok(())
}

fn execution_target_is_still_allowed(
    workflow: CleanupWorkflow,
    target: &mut CleanupTarget,
    policy: ProtectionPolicy<'_>,
) -> bool {
    match workflow {
        CleanupWorkflow::Rules => match assess_existing_path_with_policy(&target.path, policy) {
            PathDisposition::Allowed => true,
            PathDisposition::Skipped(reason) => {
                mark_target_skipped_by_policy(target, reason);
                false
            }
            PathDisposition::Blocked(reason) => {
                mark_target_blocked_by_policy(target, reason);
                false
            }
        },
        CleanupWorkflow::AppLeftovers => {
            match policy.assess_existing_app_leftover_path(&target.path) {
                AppLeftoverPathDisposition::Allowed => true,
                AppLeftoverPathDisposition::Missing => {
                    mark_target_skipped_by_policy(target, "path does not exist".to_string());
                    false
                }
                AppLeftoverPathDisposition::Blocked(reason) => {
                    mark_target_blocked_by_policy(target, reason);
                    false
                }
            }
        }
    }
}

fn mark_target_skipped_by_policy(target: &mut CleanupTarget, reason: String) {
    target.status = crate::TargetStatus::Skipped;
    target.reason = Some(reason);
    target.reason_code = Some(CleanupTargetIssueReason::SafetyPolicySkipped);
    target.freed_bytes = 0;
    target.pending_reclaim_bytes = 0;
}

fn mark_target_blocked_by_policy(target: &mut CleanupTarget, reason: String) {
    target.status = crate::TargetStatus::Blocked;
    target.reason = Some(reason);
    target.reason_code = Some(CleanupTargetIssueReason::SafetyPolicyBlocked);
    target.freed_bytes = 0;
    target.pending_reclaim_bytes = 0;
}

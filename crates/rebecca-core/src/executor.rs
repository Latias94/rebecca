use crate::error::Result;
use crate::plan::{CleanupPlan, CleanupTarget, CleanupTargetIssueReason};

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
    if plan.request.mode.is_dry_run() {
        plan.recompute_summary();
        return Ok(());
    }

    for target in &mut plan.targets {
        if !target.status.is_executable() {
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

pub fn recycle_bin_outcome(estimated_bytes: u64) -> ExecutionOutcome {
    ExecutionOutcome {
        freed_bytes: 0,
        pending_reclaim_bytes: estimated_bytes,
        note: Some("moved to Recycle Bin".to_string()),
    }
}

pub fn permanent_delete_outcome(estimated_bytes: u64) -> ExecutionOutcome {
    ExecutionOutcome {
        freed_bytes: estimated_bytes,
        pending_reclaim_bytes: 0,
        note: Some("permanently deleted".to_string()),
    }
}

use std::sync::OnceLock;

use rayon::prelude::*;
use rayon::{ThreadPool, ThreadPoolBuilder};

use crate::error::Result;
use crate::model::CleanupWorkflow;
use crate::parallelism::bounded_parallelism_budget;
use crate::path_overlap::paths_overlap;
use crate::plan::{CleanupPlan, CleanupTarget, CleanupTargetIssueReason};
use crate::protection::{AppLeftoverPathDisposition, ProtectionPolicy};
use crate::safety::{PathDisposition, assess_existing_path_with_policy};

static CLEANUP_THREAD_POOL: OnceLock<ThreadPool> = OnceLock::new();

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
    execute_cleanup_plan_serially_with_policy(plan, backend, policy)
}

pub fn execute_cleanup_plan_parallel<B: CleanupBackend + Sync>(
    plan: &mut CleanupPlan,
    backend: &B,
) -> Result<()> {
    execute_cleanup_plan_parallel_with_policy(plan, backend, ProtectionPolicy::new())
}

pub fn execute_cleanup_plan_parallel_with_policy<B: CleanupBackend + Sync>(
    plan: &mut CleanupPlan,
    backend: &B,
    policy: ProtectionPolicy<'_>,
) -> Result<()> {
    if plan.request.mode.is_dry_run() {
        plan.recompute_summary();
        return Ok(());
    }

    if !revalidate_executable_targets(plan, policy) {
        plan.recompute_summary();
        return Ok(());
    }

    let batches = batch_executable_targets(&plan.targets);
    if batches.is_empty() {
        plan.recompute_summary();
        return Ok(());
    }

    for batch in batches {
        let outcomes = run_scoped_cleanup(|| {
            batch
                .into_par_iter()
                .map(|index| (index, backend.delete(&plan.targets[index])))
                .collect::<Vec<_>>()
        });

        for (index, outcome) in outcomes {
            apply_delete_result(&mut plan.targets[index], outcome);
        }
    }

    plan.recompute_summary();
    Ok(())
}

fn revalidate_executable_targets(plan: &mut CleanupPlan, policy: ProtectionPolicy<'_>) -> bool {
    let mut any_executable = false;

    for target in &mut plan.targets {
        if !target.status.is_executable() {
            continue;
        }

        if execution_target_is_still_allowed(plan.request.workflow, target, policy) {
            any_executable = true;
        }
    }

    any_executable
}

fn execute_cleanup_plan_serially_with_policy<B: CleanupBackend>(
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

        let outcome = backend.delete(target);
        apply_delete_result(target, outcome);
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
        CleanupWorkflow::Rules | CleanupWorkflow::ProjectArtifacts => {
            match assess_existing_path_with_policy(&target.path, policy) {
                PathDisposition::Allowed => true,
                PathDisposition::Skipped(reason) => {
                    mark_target_skipped_by_policy(target, reason);
                    false
                }
                PathDisposition::Blocked(reason) => {
                    mark_target_blocked_by_policy(target, reason);
                    false
                }
            }
        }
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

fn apply_delete_result(target: &mut CleanupTarget, outcome: Result<ExecutionOutcome>) {
    match outcome {
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

fn batch_executable_targets(targets: &[CleanupTarget]) -> Vec<Vec<usize>> {
    let mut executable_indices: Vec<_> = targets
        .iter()
        .enumerate()
        .filter_map(|(index, target)| target.status.is_executable().then_some(index))
        .collect();

    executable_indices.sort_by(|left, right| {
        path_depth(&targets[*right].path)
            .cmp(&path_depth(&targets[*left].path))
            .then_with(|| targets[*left].path.cmp(&targets[*right].path))
            .then_with(|| left.cmp(right))
    });

    let mut batches: Vec<Vec<usize>> = Vec::new();
    'candidate: for index in executable_indices {
        let path = targets[index].path.as_path();
        for batch in &mut batches {
            if batch
                .iter()
                .all(|&existing| !paths_overlap(path, targets[existing].path.as_path()))
            {
                batch.push(index);
                continue 'candidate;
            }
        }

        batches.push(vec![index]);
    }

    batches
}

fn path_depth(path: &std::path::Path) -> usize {
    path.components().count()
}

pub fn cleanup_parallelism_budget() -> usize {
    bounded_parallelism_budget()
}

pub(crate) fn run_scoped_cleanup<R, F>(work: F) -> R
where
    F: FnOnce() -> R + Send,
    R: Send,
{
    cleanup_thread_pool().install(work)
}

fn cleanup_thread_pool() -> &'static ThreadPool {
    CLEANUP_THREAD_POOL.get_or_init(|| {
        ThreadPoolBuilder::new()
            .num_threads(cleanup_parallelism_budget())
            .build()
            .expect("failed to build Rebecca cleanup thread pool")
    })
}

#[cfg(test)]
mod tests {
    use super::{batch_executable_targets, cleanup_parallelism_budget, run_scoped_cleanup};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crate::plan::{CleanupTarget, CleanupTargetDeletionStyle};
    use crate::scan::scan_parallelism_budget;
    use crate::{DeleteMode, TargetStatus};

    #[test]
    fn cleanup_parallelism_budget_stays_bounded() {
        let budget = cleanup_parallelism_budget();

        assert!((2..=8).contains(&budget));
    }

    #[test]
    fn run_scoped_cleanup_executes_work() {
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_ref = Arc::clone(&counter);

        run_scoped_cleanup(move || {
            counter_ref.fetch_add(1, Ordering::SeqCst);
        });

        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn cleanup_and_scan_parallelism_budgets_match() {
        assert_eq!(cleanup_parallelism_budget(), scan_parallelism_budget());
    }

    #[test]
    fn batch_executable_targets_keeps_overlapping_paths_separate() {
        let targets = vec![
            CleanupTarget::allowed(
                "windows.a",
                std::path::PathBuf::from(r"C:\Temp\Rebecca\a"),
                1,
                DeleteMode::RecycleBin,
            )
            .with_deletion_style(CleanupTargetDeletionStyle::DeleteWholePath),
            CleanupTarget::allowed(
                "windows.b",
                std::path::PathBuf::from(r"C:\Temp\Rebecca\a\child"),
                1,
                DeleteMode::RecycleBin,
            )
            .with_deletion_style(CleanupTargetDeletionStyle::DeleteWholePath),
            CleanupTarget::allowed(
                "windows.c",
                std::path::PathBuf::from(r"C:\Temp\Rebecca\other"),
                1,
                DeleteMode::RecycleBin,
            )
            .with_deletion_style(CleanupTargetDeletionStyle::DeleteWholePath),
        ];

        let batches = batch_executable_targets(&targets);
        let mut seen = batches.iter().flatten().copied().collect::<Vec<_>>();
        seen.sort_unstable();

        assert_eq!(seen, vec![0, 1, 2]);
        assert!(batches.iter().all(|batch| {
            batch.iter().enumerate().all(|(index, left)| {
                batch[index + 1..].iter().all(|right| {
                    !crate::path_overlap::paths_overlap(
                        targets[*left].path.as_path(),
                        targets[*right].path.as_path(),
                    )
                })
            })
        }));
        assert!(
            targets
                .iter()
                .all(|target| target.status == TargetStatus::Allowed)
        );
    }

    #[test]
    fn batch_executable_targets_prefers_descendants_before_ancestors() {
        let targets = vec![
            CleanupTarget::allowed(
                "windows.parent",
                std::path::PathBuf::from(r"C:\Temp\Rebecca"),
                1,
                DeleteMode::RecycleBin,
            )
            .with_deletion_style(CleanupTargetDeletionStyle::DeleteWholePath),
            CleanupTarget::allowed(
                "windows.child",
                std::path::PathBuf::from(r"C:\Temp\Rebecca\child"),
                1,
                DeleteMode::RecycleBin,
            )
            .with_deletion_style(CleanupTargetDeletionStyle::DeleteWholePath),
        ];

        let batches = batch_executable_targets(&targets);

        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0], vec![1]);
        assert_eq!(batches[1], vec![0]);
    }
}

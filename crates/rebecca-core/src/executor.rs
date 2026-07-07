use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use rayon::ThreadPool;
use rayon::prelude::*;

use crate::cache::{CachePurgeBackend, CachePurgeEntryKind, CachePurgeOutcome};
use crate::error::{RebeccaError, Result};
use crate::execution::ExecutionReport;
use crate::model::CleanupWorkflow;
use crate::parallelism::{bounded_parallelism_budget, run_scoped_parallel_work};
use crate::path_overlap::{PathRelation, path_relation, paths_overlap};
use crate::plan::{
    CleanupPlan, CleanupTarget, CleanupTargetDeletionStyle, CleanupTargetIssueReason,
};
use crate::protection::{AppLeftoverPathDisposition, ProtectionPolicy};
use crate::safety::{
    PATH_DOES_NOT_EXIST_REASON, PathDisposition, assess_existing_path_with_policy, is_reparse_like,
};

static CLEANUP_THREAD_POOL: OnceLock<ThreadPool> = OnceLock::new();

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionOutcome {
    pub freed_bytes: u64,
    pub pending_reclaim_bytes: u64,
    pub note: Option<String>,
}

pub trait CleanupBackend {
    fn delete(&self, target: &CleanupTarget) -> Result<ExecutionOutcome>;

    fn supports_batch_delete(&self) -> bool {
        false
    }

    fn delete_batch(&self, targets: &[&CleanupTarget]) -> Vec<Result<ExecutionOutcome>> {
        targets.iter().map(|target| self.delete(target)).collect()
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct RecoverableTrashBackend;

impl RecoverableTrashBackend {
    pub fn new() -> Self {
        Self
    }
}

impl CleanupBackend for RecoverableTrashBackend {
    fn delete(&self, target: &CleanupTarget) -> Result<ExecutionOutcome> {
        delete_to_recoverable_trash(&target.path, target.estimated_bytes, target.deletion_style)
    }

    fn supports_batch_delete(&self) -> bool {
        true
    }

    fn delete_batch(&self, targets: &[&CleanupTarget]) -> Vec<Result<ExecutionOutcome>> {
        delete_batch_to_recoverable_trash(targets)
    }
}

impl CachePurgeBackend for RecoverableTrashBackend {
    fn purge(
        &self,
        path: &Path,
        kind: CachePurgeEntryKind,
        estimated_bytes: u64,
    ) -> Result<CachePurgeOutcome> {
        match kind {
            CachePurgeEntryKind::File | CachePurgeEntryKind::Directory => {
                delete_to_recoverable_trash(
                    path,
                    estimated_bytes,
                    CleanupTargetDeletionStyle::DeleteWholePath,
                )
                .map(|outcome| CachePurgeOutcome {
                    reclaimed_bytes: outcome.freed_bytes,
                    pending_reclaim_bytes: outcome.pending_reclaim_bytes,
                    note: outcome.note,
                })
            }
            CachePurgeEntryKind::Symlink | CachePurgeEntryKind::Other => {
                Err(RebeccaError::ExecutionFailed(format!(
                    "cache purge backend does not support {} entries",
                    kind.label()
                )))
            }
        }
    }
}

struct BatchTrashTarget {
    target_index: usize,
    delete_paths: Vec<PathBuf>,
}

fn delete_to_recoverable_trash(
    path: &Path,
    estimated_bytes: u64,
    deletion_style: CleanupTargetDeletionStyle,
) -> Result<ExecutionOutcome> {
    match deletion_style {
        CleanupTargetDeletionStyle::DeleteWholePath => {
            trash::delete(path).map_err(|err| RebeccaError::ExecutionFailed(err.to_string()))?;
        }
        CleanupTargetDeletionStyle::PreserveRootContents => {
            if path.is_dir() {
                let entries = preserve_root_delete_paths(path)?;
                if !entries.is_empty() {
                    trash::delete_all(entries)
                        .map_err(|err| RebeccaError::ExecutionFailed(err.to_string()))?;
                }
            } else {
                trash::delete(path)
                    .map_err(|err| RebeccaError::ExecutionFailed(err.to_string()))?;
            }
        }
    }

    Ok(recoverable_trash_outcome(estimated_bytes))
}

fn delete_batch_to_recoverable_trash(targets: &[&CleanupTarget]) -> Vec<Result<ExecutionOutcome>> {
    let mut outcomes = (0..targets.len()).map(|_| None).collect::<Vec<_>>();
    let mut batch_targets = Vec::new();
    let mut batch_paths = Vec::new();

    for (target_index, target) in targets.iter().enumerate() {
        match recoverable_trash_paths_for_target(target) {
            Ok(delete_paths) if delete_paths.is_empty() => {
                outcomes[target_index] =
                    Some(Ok(recoverable_trash_outcome(target.estimated_bytes)));
            }
            Ok(delete_paths) => {
                batch_paths.extend(delete_paths.iter().cloned());
                batch_targets.push(BatchTrashTarget {
                    target_index,
                    delete_paths,
                });
            }
            Err(err) => {
                outcomes[target_index] = Some(Err(err));
            }
        }
    }

    if !batch_paths.is_empty() {
        match trash::delete_all(batch_paths.iter()) {
            Ok(()) => {
                for batch_target in batch_targets {
                    let target = targets[batch_target.target_index];
                    outcomes[batch_target.target_index] =
                        Some(Ok(recoverable_trash_outcome(target.estimated_bytes)));
                }
            }
            Err(_) => {
                for batch_target in batch_targets {
                    let target = targets[batch_target.target_index];
                    outcomes[batch_target.target_index] =
                        Some(reconstruct_or_fallback_after_batch_failure(
                            target,
                            &batch_target.delete_paths,
                        ));
                }
            }
        }
    }

    outcomes
        .into_iter()
        .map(|outcome| {
            outcome.unwrap_or_else(|| {
                Err(RebeccaError::ExecutionFailed(
                    "batch recoverable trash backend did not produce a target outcome".to_string(),
                ))
            })
        })
        .collect()
}

fn recoverable_trash_paths_for_target(target: &CleanupTarget) -> Result<Vec<PathBuf>> {
    let metadata = fs::symlink_metadata(&target.path)?;
    match target.deletion_style {
        CleanupTargetDeletionStyle::DeleteWholePath => Ok(vec![target.path.clone()]),
        CleanupTargetDeletionStyle::PreserveRootContents => {
            if metadata.is_dir() {
                preserve_root_delete_paths(&target.path)
            } else {
                Ok(vec![target.path.clone()])
            }
        }
    }
}

fn preserve_root_delete_paths(path: &Path) -> Result<Vec<PathBuf>> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let child = entry.path();
        let metadata = fs::symlink_metadata(&child)?;
        if is_reparse_like(&metadata) {
            return Err(RebeccaError::ExecutionFailed(format!(
                "preserve-root cleanup refused reparse child {}",
                child.display()
            )));
        }
        entries.push(child);
    }
    Ok(entries)
}

fn reconstruct_or_fallback_after_batch_failure(
    target: &CleanupTarget,
    delete_paths: &[PathBuf],
) -> Result<ExecutionOutcome> {
    if delete_paths
        .iter()
        .all(|path| matches!(path.try_exists(), Ok(false)))
    {
        return Ok(recoverable_trash_outcome(target.estimated_bytes));
    }

    delete_to_recoverable_trash(&target.path, target.estimated_bytes, target.deletion_style)
}

fn recoverable_trash_outcome(estimated_bytes: u64) -> ExecutionOutcome {
    ExecutionOutcome {
        freed_bytes: 0,
        pending_reclaim_bytes: estimated_bytes,
        note: Some("moved to recoverable trash".to_string()),
    }
}

pub fn execute_cleanup_plan<B: CleanupBackend>(
    plan: &mut CleanupPlan,
    backend: &B,
) -> Result<ExecutionReport> {
    execute_cleanup_plan_with_policy(plan, backend, ProtectionPolicy::new())
}

pub fn execute_cleanup_plan_with_policy<B: CleanupBackend>(
    plan: &mut CleanupPlan,
    backend: &B,
    policy: ProtectionPolicy<'_>,
) -> Result<ExecutionReport> {
    execute_cleanup_plan_serially_with_policy(plan, backend, policy)
}

pub fn execute_cleanup_plan_parallel<B: CleanupBackend + Sync>(
    plan: &mut CleanupPlan,
    backend: &B,
) -> Result<ExecutionReport> {
    execute_cleanup_plan_parallel_with_policy(plan, backend, ProtectionPolicy::new())
}

pub fn execute_cleanup_plan_parallel_with_policy<B: CleanupBackend + Sync>(
    plan: &mut CleanupPlan,
    backend: &B,
    policy: ProtectionPolicy<'_>,
) -> Result<ExecutionReport> {
    if plan.request.mode.is_dry_run() {
        plan.recompute_summary();
        let report = ExecutionReport::dry_run();
        plan.execution_report = Some(report.clone());
        return Ok(report);
    }

    if !revalidate_executable_targets(plan, policy) {
        plan.recompute_summary();
        let report = ExecutionReport::from_targets(&plan.targets);
        plan.execution_report = Some(report.clone());
        return Ok(report);
    }
    normalize_overlapping_executable_targets(plan);

    let batches = batch_executable_targets(&plan.targets);
    if batches.is_empty() {
        plan.recompute_summary();
        let report = ExecutionReport::from_targets(&plan.targets);
        plan.execution_report = Some(report.clone());
        return Ok(report);
    }

    let batch_delete_supported = backend.supports_batch_delete();
    for mut batch in batches {
        batch.retain(|&index| {
            execution_target_is_still_allowed(
                plan.request.workflow,
                &mut plan.targets[index],
                policy,
            )
        });
        if batch.is_empty() {
            continue;
        }

        if batch_delete_supported {
            let outcomes = {
                let targets = batch
                    .iter()
                    .map(|&index| &plan.targets[index])
                    .collect::<Vec<_>>();
                backend.delete_batch(&targets)
            };
            apply_batch_delete_results(&mut plan.targets, &batch, outcomes);
        } else {
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
    }

    plan.recompute_summary();
    let report = ExecutionReport::from_targets(&plan.targets);
    plan.execution_report = Some(report.clone());
    Ok(report)
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
) -> Result<ExecutionReport> {
    if plan.request.mode.is_dry_run() {
        plan.recompute_summary();
        let report = ExecutionReport::dry_run();
        plan.execution_report = Some(report.clone());
        return Ok(report);
    }

    if revalidate_executable_targets(plan, policy) {
        normalize_overlapping_executable_targets(plan);
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
    let report = ExecutionReport::from_targets(&plan.targets);
    plan.execution_report = Some(report.clone());
    Ok(report)
}

fn normalize_overlapping_executable_targets(plan: &mut CleanupPlan) {
    let executable_indices = plan
        .targets
        .iter()
        .enumerate()
        .filter_map(|(index, target)| target.status.is_executable().then_some(index))
        .collect::<Vec<_>>();

    let mut shadowed_by = vec![None; plan.targets.len()];
    for &covered_index in &executable_indices {
        for &candidate_index in &executable_indices {
            if candidate_index == covered_index {
                continue;
            }

            let candidate_covers_target = match path_relation(
                plan.targets[candidate_index].path.as_path(),
                plan.targets[covered_index].path.as_path(),
            ) {
                PathRelation::Same => candidate_index < covered_index,
                PathRelation::Ancestor => true,
                PathRelation::Descendant | PathRelation::Unrelated => false,
            };
            if candidate_covers_target
                && shadow_parent_is_preferred(plan, shadowed_by[covered_index], candidate_index)
            {
                shadowed_by[covered_index] = Some(candidate_index);
            }
        }
    }

    for covered_index in executable_indices {
        let Some(parent_index) = shadowed_by[covered_index] else {
            continue;
        };
        let parent_rule_id = plan.targets[parent_index].rule_id.clone();
        mark_target_shadowed_before_execution(&mut plan.targets[covered_index], parent_rule_id);
    }
}

fn shadow_parent_is_preferred(
    plan: &CleanupPlan,
    current_parent: Option<usize>,
    candidate_parent: usize,
) -> bool {
    let Some(current_parent) = current_parent else {
        return true;
    };

    let candidate_depth = plan.targets[candidate_parent].path.components().count();
    let current_depth = plan.targets[current_parent].path.components().count();
    candidate_depth < current_depth
        || (candidate_depth == current_depth && candidate_parent < current_parent)
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
                PathDisposition::Missing => {
                    mark_target_missing_before_execution(target);
                    false
                }
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
                    mark_target_missing_before_execution(target);
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

fn mark_target_missing_before_execution(target: &mut CleanupTarget) {
    target.status = crate::TargetStatus::Skipped;
    target.reason = Some(PATH_DOES_NOT_EXIST_REASON.to_string());
    target.reason_code = Some(CleanupTargetIssueReason::ExecutionTargetMissing);
    target.freed_bytes = 0;
    target.pending_reclaim_bytes = 0;
}

fn mark_target_shadowed_before_execution(target: &mut CleanupTarget, parent_rule_id: String) {
    target.status = crate::TargetStatus::Skipped;
    target.reason = Some(format!(
        "covered by overlapping cleanup target from {parent_rule_id}"
    ));
    target.reason_code = Some(CleanupTargetIssueReason::ExecutionTargetShadowed);
    target.freed_bytes = 0;
    target.pending_reclaim_bytes = 0;
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

fn apply_batch_delete_results(
    targets: &mut [CleanupTarget],
    batch: &[usize],
    outcomes: Vec<Result<ExecutionOutcome>>,
) {
    if outcomes.len() != batch.len() {
        let message = format!(
            "batch cleanup backend returned {} outcome(s) for {} target(s)",
            outcomes.len(),
            batch.len()
        );
        for &index in batch {
            apply_delete_result(
                &mut targets[index],
                Err(RebeccaError::ExecutionFailed(message.clone())),
            );
        }
        return;
    }

    for (&index, outcome) in batch.iter().zip(outcomes) {
        apply_delete_result(&mut targets[index], outcome);
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
    path.as_os_str()
        .to_string_lossy()
        .replace('\\', "/")
        .split('/')
        .filter(|component| !component.is_empty() && *component != ".")
        .count()
}

pub fn cleanup_parallelism_budget() -> usize {
    bounded_parallelism_budget()
}

pub(crate) fn run_scoped_cleanup<R, F>(work: F) -> R
where
    F: FnOnce() -> R + Send,
    R: Send,
{
    run_scoped_parallel_work(&CLEANUP_THREAD_POOL, "cleanup", work)
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
                DeleteMode::RecoverableDelete,
            )
            .with_deletion_style(CleanupTargetDeletionStyle::DeleteWholePath),
            CleanupTarget::allowed(
                "windows.b",
                std::path::PathBuf::from(r"C:\Temp\Rebecca\a\child"),
                1,
                DeleteMode::RecoverableDelete,
            )
            .with_deletion_style(CleanupTargetDeletionStyle::DeleteWholePath),
            CleanupTarget::allowed(
                "windows.c",
                std::path::PathBuf::from(r"C:\Temp\Rebecca\other"),
                1,
                DeleteMode::RecoverableDelete,
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
                DeleteMode::RecoverableDelete,
            )
            .with_deletion_style(CleanupTargetDeletionStyle::DeleteWholePath),
            CleanupTarget::allowed(
                "windows.child",
                std::path::PathBuf::from(r"C:\Temp\Rebecca\child"),
                1,
                DeleteMode::RecoverableDelete,
            )
            .with_deletion_style(CleanupTargetDeletionStyle::DeleteWholePath),
        ];

        let batches = batch_executable_targets(&targets);

        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0], vec![1]);
        assert_eq!(batches[1], vec![0]);
    }
}

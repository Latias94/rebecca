use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use rayon::ThreadPool;
use rayon::prelude::*;

use crate::cache::{CachePurgeBackend, CachePurgeEntryKind, CachePurgeOutcome};
use crate::error::{RebeccaError, Result};
use crate::execution::{ExecutionProgressEvent, ExecutionProgressTarget, ExecutionReport};
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
use crate::safety_catalog::default_safety_knowledge_for_platform;
use crate::scan::ScanCancellationToken;

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

pub trait RecoverableTrashAdapter: Send + Sync {
    fn delete_paths(&self, paths: &[PathBuf]) -> Result<()>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SystemRecoverableTrashAdapter;

impl RecoverableTrashAdapter for SystemRecoverableTrashAdapter {
    fn delete_paths(&self, paths: &[PathBuf]) -> Result<()> {
        match paths {
            [] => Ok(()),
            [path] => {
                trash::delete(path).map_err(|err| RebeccaError::ExecutionFailed(err.to_string()))
            }
            _ => trash::delete_all(paths.iter())
                .map_err(|err| RebeccaError::ExecutionFailed(err.to_string())),
        }
    }
}

#[derive(Clone)]
pub struct RecoverableTrashBackend {
    adapter: Arc<dyn RecoverableTrashAdapter>,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct PermanentDeleteBackend;

impl std::fmt::Debug for RecoverableTrashBackend {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RecoverableTrashBackend")
            .finish_non_exhaustive()
    }
}

impl Default for RecoverableTrashBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl RecoverableTrashBackend {
    pub fn new() -> Self {
        Self::with_adapter(SystemRecoverableTrashAdapter)
    }

    pub fn with_adapter(adapter: impl RecoverableTrashAdapter + 'static) -> Self {
        Self {
            adapter: Arc::new(adapter),
        }
    }
}

impl CleanupBackend for RecoverableTrashBackend {
    fn delete(&self, target: &CleanupTarget) -> Result<ExecutionOutcome> {
        delete_to_recoverable_trash(
            self.adapter.as_ref(),
            &target.path,
            target.estimated_bytes,
            target.deletion_style,
        )
    }

    fn supports_batch_delete(&self) -> bool {
        true
    }

    fn delete_batch(&self, targets: &[&CleanupTarget]) -> Vec<Result<ExecutionOutcome>> {
        delete_batch_to_recoverable_trash(self.adapter.as_ref(), targets)
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
                    self.adapter.as_ref(),
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

impl CleanupBackend for PermanentDeleteBackend {
    fn delete(&self, target: &CleanupTarget) -> Result<ExecutionOutcome> {
        delete_permanently(&target.path, target.estimated_bytes, target.deletion_style)
    }
}

struct BatchTrashTarget {
    target_index: usize,
    delete_paths: Vec<PathBuf>,
}

fn delete_to_recoverable_trash(
    adapter: &dyn RecoverableTrashAdapter,
    path: &Path,
    estimated_bytes: u64,
    deletion_style: CleanupTargetDeletionStyle,
) -> Result<ExecutionOutcome> {
    let delete_paths = recoverable_trash_paths(path, deletion_style)?;
    adapter.delete_paths(&delete_paths)?;
    Ok(recoverable_trash_outcome(estimated_bytes))
}

fn delete_batch_to_recoverable_trash(
    adapter: &dyn RecoverableTrashAdapter,
    targets: &[&CleanupTarget],
) -> Vec<Result<ExecutionOutcome>> {
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
        match adapter.delete_paths(&batch_paths) {
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
                            adapter,
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
    recoverable_trash_paths(&target.path, target.deletion_style)
}

fn recoverable_trash_paths(
    path: &Path,
    deletion_style: CleanupTargetDeletionStyle,
) -> Result<Vec<PathBuf>> {
    let metadata = recoverable_delete_candidate_metadata(path, "target")?;
    match deletion_style {
        CleanupTargetDeletionStyle::DeleteWholePath => Ok(vec![path.to_path_buf()]),
        CleanupTargetDeletionStyle::PreserveRootContents => {
            if metadata.is_dir() {
                preserve_root_delete_paths(path)
            } else {
                Ok(vec![path.to_path_buf()])
            }
        }
    }
}

fn preserve_root_delete_paths(path: &Path) -> Result<Vec<PathBuf>> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let child = entry.path();
        recoverable_delete_candidate_metadata(&child, "child")?;
        entries.push(child);
    }
    Ok(entries)
}

fn recoverable_delete_candidate_metadata(path: &Path, role: &str) -> Result<fs::Metadata> {
    let metadata = fs::symlink_metadata(path)?;
    if is_reparse_like(&metadata) {
        return Err(RebeccaError::SafetyBlocked(format!(
            "recoverable trash refused reparse {role} {}",
            path.display()
        )));
    }

    Ok(metadata)
}

fn delete_permanently(
    path: &Path,
    estimated_bytes: u64,
    deletion_style: CleanupTargetDeletionStyle,
) -> Result<ExecutionOutcome> {
    let delete_paths = permanent_delete_paths(path, deletion_style)?;
    for path in delete_paths {
        permanently_delete_path(&path)?;
    }
    Ok(permanent_delete_outcome(estimated_bytes))
}

fn permanent_delete_paths(
    path: &Path,
    deletion_style: CleanupTargetDeletionStyle,
) -> Result<Vec<PathBuf>> {
    let metadata = recoverable_delete_candidate_metadata(path, "target")?;
    match deletion_style {
        CleanupTargetDeletionStyle::DeleteWholePath => Ok(vec![path.to_path_buf()]),
        CleanupTargetDeletionStyle::PreserveRootContents => {
            if metadata.is_dir() {
                preserve_root_delete_paths(path)
            } else {
                Ok(vec![path.to_path_buf()])
            }
        }
    }
}

fn permanently_delete_path(path: &Path) -> Result<()> {
    let metadata = recoverable_delete_candidate_metadata(path, "target")?;
    if metadata.is_dir() {
        fs::remove_dir_all(path)?;
    } else if metadata.is_file() {
        fs::remove_file(path)?;
    } else {
        return Err(RebeccaError::ExecutionFailed(format!(
            "permanent delete does not support {}",
            path.display()
        )));
    }
    Ok(())
}

fn reconstruct_or_fallback_after_batch_failure(
    adapter: &dyn RecoverableTrashAdapter,
    target: &CleanupTarget,
    delete_paths: &[PathBuf],
) -> Result<ExecutionOutcome> {
    if delete_paths
        .iter()
        .all(|path| matches!(path.try_exists(), Ok(false)))
    {
        return Ok(recoverable_trash_outcome(target.estimated_bytes));
    }

    delete_to_recoverable_trash(
        adapter,
        &target.path,
        target.estimated_bytes,
        target.deletion_style,
    )
}

fn recoverable_trash_outcome(estimated_bytes: u64) -> ExecutionOutcome {
    ExecutionOutcome {
        freed_bytes: 0,
        pending_reclaim_bytes: estimated_bytes,
        note: Some("moved to system trash".to_string()),
    }
}

fn permanent_delete_outcome(estimated_bytes: u64) -> ExecutionOutcome {
    ExecutionOutcome {
        freed_bytes: estimated_bytes,
        pending_reclaim_bytes: 0,
        note: Some("deleted permanently".to_string()),
    }
}

pub fn execute_cleanup_plan<B: CleanupBackend>(
    plan: &mut CleanupPlan,
    backend: &B,
) -> Result<ExecutionReport> {
    let policy = default_execution_policy(plan);
    execute_cleanup_plan_with_policy(plan, backend, policy)
}

pub fn execute_cleanup_plan_with_policy<B: CleanupBackend>(
    plan: &mut CleanupPlan,
    backend: &B,
    policy: ProtectionPolicy<'_>,
) -> Result<ExecutionReport> {
    execute_cleanup_plan_serially_with_policy_and_cancellation(
        plan,
        backend,
        policy,
        &ScanCancellationToken::new(),
    )
}

pub fn execute_cleanup_plan_parallel<B: CleanupBackend + Sync>(
    plan: &mut CleanupPlan,
    backend: &B,
) -> Result<ExecutionReport> {
    let policy = default_execution_policy(plan);
    execute_cleanup_plan_parallel_with_policy(plan, backend, policy)
}

fn default_execution_policy(plan: &CleanupPlan) -> ProtectionPolicy<'static> {
    match default_safety_knowledge_for_platform(plan.request.platform) {
        Some(safety_knowledge) => ProtectionPolicy::new().with_safety_knowledge(safety_knowledge),
        None => ProtectionPolicy::new(),
    }
}

pub fn execute_cleanup_plan_parallel_with_policy<B: CleanupBackend + Sync>(
    plan: &mut CleanupPlan,
    backend: &B,
    policy: ProtectionPolicy<'_>,
) -> Result<ExecutionReport> {
    execute_cleanup_plan_parallel_with_policy_and_cancellation(
        plan,
        backend,
        policy,
        &ScanCancellationToken::new(),
    )
}

pub fn execute_cleanup_plan_parallel_with_policy_and_cancellation<B: CleanupBackend + Sync>(
    plan: &mut CleanupPlan,
    backend: &B,
    policy: ProtectionPolicy<'_>,
    cancellation: &ScanCancellationToken,
) -> Result<ExecutionReport> {
    execute_cleanup_plan_parallel_with_policy_and_cancellation_and_progress(
        plan,
        backend,
        policy,
        cancellation,
        |_| {},
    )
}

pub fn execute_cleanup_plan_parallel_with_policy_and_cancellation_and_progress<
    B: CleanupBackend + Sync,
    F,
>(
    plan: &mut CleanupPlan,
    backend: &B,
    policy: ProtectionPolicy<'_>,
    cancellation: &ScanCancellationToken,
    mut progress: F,
) -> Result<ExecutionReport>
where
    F: for<'event> FnMut(ExecutionProgressEvent<'event>),
{
    if plan.request.mode.is_dry_run() {
        plan.recompute_summary();
        let report = ExecutionReport::dry_run();
        plan.execution_report = Some(report.clone());
        return Ok(report);
    }

    abort_if_execution_cancelled(plan, cancellation)?;
    emit_execution_started(plan, &mut progress);

    if !revalidate_executable_targets(plan, policy) {
        plan.recompute_summary();
        let report = ExecutionReport::from_targets(&plan.targets);
        progress(ExecutionProgressEvent::Completed {
            summary: &report.summary,
        });
        plan.execution_report = Some(report.clone());
        return Ok(report);
    }
    normalize_overlapping_executable_targets(plan);

    let batches = batch_executable_targets(&plan.targets);
    if batches.is_empty() {
        plan.recompute_summary();
        let report = ExecutionReport::from_targets(&plan.targets);
        progress(ExecutionProgressEvent::Completed {
            summary: &report.summary,
        });
        plan.execution_report = Some(report.clone());
        return Ok(report);
    }

    let batch_delete_supported = backend.supports_batch_delete();
    for mut batch in batches {
        abort_if_execution_cancelled(plan, cancellation)?;
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

        for &index in &batch {
            progress(ExecutionProgressEvent::TargetStarted {
                target_index: index,
                target: ExecutionProgressTarget::from_target(&plan.targets[index]),
            });
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
            for &index in &batch {
                progress(ExecutionProgressEvent::TargetFinished {
                    target_index: index,
                    target: ExecutionProgressTarget::from_target(&plan.targets[index]),
                });
            }
        } else {
            let outcomes = run_scoped_cleanup(|| {
                batch
                    .par_iter()
                    .map(|&index| (index, backend.delete(&plan.targets[index])))
                    .collect::<Vec<_>>()
            });

            for (index, outcome) in outcomes {
                apply_delete_result(&mut plan.targets[index], outcome);
            }
            for &index in &batch {
                progress(ExecutionProgressEvent::TargetFinished {
                    target_index: index,
                    target: ExecutionProgressTarget::from_target(&plan.targets[index]),
                });
            }
        }
    }

    abort_if_execution_cancelled(plan, cancellation)?;
    plan.recompute_summary();
    let report = ExecutionReport::from_targets(&plan.targets);
    progress(ExecutionProgressEvent::Completed {
        summary: &report.summary,
    });
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

fn execute_cleanup_plan_serially_with_policy_and_cancellation<B: CleanupBackend>(
    plan: &mut CleanupPlan,
    backend: &B,
    policy: ProtectionPolicy<'_>,
    cancellation: &ScanCancellationToken,
) -> Result<ExecutionReport> {
    execute_cleanup_plan_serially_with_policy_and_cancellation_and_progress(
        plan,
        backend,
        policy,
        cancellation,
        |_| {},
    )
}

fn execute_cleanup_plan_serially_with_policy_and_cancellation_and_progress<B: CleanupBackend, F>(
    plan: &mut CleanupPlan,
    backend: &B,
    policy: ProtectionPolicy<'_>,
    cancellation: &ScanCancellationToken,
    mut progress: F,
) -> Result<ExecutionReport>
where
    F: for<'event> FnMut(ExecutionProgressEvent<'event>),
{
    if plan.request.mode.is_dry_run() {
        plan.recompute_summary();
        let report = ExecutionReport::dry_run();
        plan.execution_report = Some(report.clone());
        return Ok(report);
    }

    abort_if_execution_cancelled(plan, cancellation)?;
    emit_execution_started(plan, &mut progress);

    if revalidate_executable_targets(plan, policy) {
        normalize_overlapping_executable_targets(plan);
    }

    for index in 0..plan.targets.len() {
        abort_if_execution_cancelled(plan, cancellation)?;
        let target = &mut plan.targets[index];
        if !target.status.is_executable() {
            continue;
        }

        if !execution_target_is_still_allowed(plan.request.workflow, target, policy) {
            continue;
        }

        progress(ExecutionProgressEvent::TargetStarted {
            target_index: index,
            target: ExecutionProgressTarget::from_target(target),
        });
        let outcome = backend.delete(target);
        apply_delete_result(target, outcome);
        progress(ExecutionProgressEvent::TargetFinished {
            target_index: index,
            target: ExecutionProgressTarget::from_target(target),
        });
    }

    abort_if_execution_cancelled(plan, cancellation)?;
    plan.recompute_summary();
    let report = ExecutionReport::from_targets(&plan.targets);
    progress(ExecutionProgressEvent::Completed {
        summary: &report.summary,
    });
    plan.execution_report = Some(report.clone());
    Ok(report)
}

fn emit_execution_started(
    plan: &CleanupPlan,
    progress: &mut impl for<'event> FnMut(ExecutionProgressEvent<'event>),
) {
    let executable_targets = plan
        .targets
        .iter()
        .filter(|target| target.status.is_executable())
        .count();
    let estimated_bytes = plan
        .targets
        .iter()
        .filter(|target| target.status.is_executable())
        .map(|target| target.estimated_bytes)
        .sum();

    progress(ExecutionProgressEvent::Started {
        total_targets: plan.targets.len(),
        executable_targets,
        estimated_bytes,
        mode: plan.request.mode,
    });
}

fn abort_if_execution_cancelled(
    plan: &mut CleanupPlan,
    cancellation: &ScanCancellationToken,
) -> Result<()> {
    if cancellation.is_cancelled() {
        plan.recompute_summary();
        return Err(RebeccaError::OperationCancelled(
            "cleanup execution was cancelled".to_string(),
        ));
    }
    Ok(())
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
    target.mark_skipped_with_reason_code(
        CleanupTargetIssueReason::ExecutionTargetMissing,
        PATH_DOES_NOT_EXIST_REASON,
    );
}

fn mark_target_shadowed_before_execution(target: &mut CleanupTarget, parent_rule_id: String) {
    target.mark_skipped_with_reason_code(
        CleanupTargetIssueReason::ExecutionTargetShadowed,
        format!("covered by overlapping cleanup target from {parent_rule_id}"),
    );
}

fn mark_target_skipped_by_policy(target: &mut CleanupTarget, reason: String) {
    target.mark_skipped_with_reason_code(CleanupTargetIssueReason::SafetyPolicySkipped, reason);
}

fn mark_target_blocked_by_policy(target: &mut CleanupTarget, reason: String) {
    target.mark_blocked_with_reason_code(CleanupTargetIssueReason::SafetyPolicyBlocked, reason);
}

fn apply_delete_result(target: &mut CleanupTarget, outcome: Result<ExecutionOutcome>) {
    match outcome {
        Ok(outcome) => {
            target.mark_completed(
                outcome.freed_bytes,
                outcome.pending_reclaim_bytes,
                outcome.note,
            );
        }
        Err(err) => match &err {
            RebeccaError::SafetyBlocked(_) => {
                target.mark_blocked_with_reason_code(
                    CleanupTargetIssueReason::SafetyPolicyBlocked,
                    err.to_string(),
                );
            }
            _ => {
                target.mark_failed_with_reason_code(execution_issue_reason(&err), err.to_string());
            }
        },
    }
}

fn execution_issue_reason(err: &RebeccaError) -> CleanupTargetIssueReason {
    if matches!(err, RebeccaError::SafetyBlocked(_)) {
        CleanupTargetIssueReason::SafetyPolicyBlocked
    } else if is_permission_denied_execution_error(err) {
        CleanupTargetIssueReason::ExecutionPermissionDenied
    } else {
        CleanupTargetIssueReason::ExecutionFailed
    }
}

fn is_permission_denied_execution_error(err: &RebeccaError) -> bool {
    match err {
        RebeccaError::Io(err) if err.kind() == std::io::ErrorKind::PermissionDenied => true,
        RebeccaError::ExecutionFailed(message) => {
            let normalized = message.to_ascii_lowercase();
            normalized.contains("permission denied")
                || normalized.contains("access is denied")
                || normalized.contains("operation not permitted")
        }
        _ => false,
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
    use super::{
        CleanupBackend, ExecutionOutcome, batch_executable_targets, cleanup_parallelism_budget,
        run_scoped_cleanup,
    };
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crate::plan::{CleanupPlan, CleanupTarget, CleanupTargetDeletionStyle};
    use crate::scan::ScanCancellationToken;
    use crate::scan::scan_parallelism_budget;
    use crate::{DeleteMode, PlanRequest, Platform, RebeccaError, TargetStatus};

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
    fn cancelled_token_before_execution_prevents_backend_mutation() {
        let cancellation = ScanCancellationToken::new();
        cancellation.cancel();
        let backend = CountingBackend::default();
        let mut plan = executable_plan(2);

        let err = super::execute_cleanup_plan_serially_with_policy_and_cancellation(
            &mut plan,
            &backend,
            crate::protection::ProtectionPolicy::new(),
            &cancellation,
        )
        .unwrap_err();

        assert!(matches!(err, RebeccaError::OperationCancelled(_)));
        assert_eq!(backend.calls.load(Ordering::SeqCst), 0);
        assert!(
            plan.targets
                .iter()
                .all(|target| target.status == TargetStatus::Allowed)
        );
    }

    #[test]
    fn serial_execution_stops_before_next_target_after_cancellation() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = (0..3)
            .map(|index| {
                let path = temp.path().join(format!("target-{index}"));
                std::fs::write(&path, b"data").expect("write target");
                path
            })
            .collect::<Vec<_>>();
        let cancellation = ScanCancellationToken::new();
        let backend = CountingBackend {
            calls: Arc::new(AtomicUsize::new(0)),
            cancel_after_calls: Some((1, cancellation.clone())),
        };
        let mut plan = executable_plan_with_paths(paths);

        let err = super::execute_cleanup_plan_serially_with_policy_and_cancellation(
            &mut plan,
            &backend,
            crate::protection::ProtectionPolicy::new(),
            &cancellation,
        )
        .unwrap_err();

        assert!(matches!(err, RebeccaError::OperationCancelled(_)));
        assert_eq!(backend.calls.load(Ordering::SeqCst), 1);
        assert_eq!(plan.targets[0].status, TargetStatus::Completed);
        assert_eq!(plan.targets[1].status, TargetStatus::Allowed);
        assert_eq!(plan.targets[2].status, TargetStatus::Allowed);
        assert_eq!(plan.summary.completed_targets, 1);
        assert_eq!(plan.summary.allowed_targets, 2);
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

    #[derive(Clone, Default)]
    struct CountingBackend {
        calls: Arc<AtomicUsize>,
        cancel_after_calls: Option<(usize, ScanCancellationToken)>,
    }

    impl CleanupBackend for CountingBackend {
        fn delete(&self, _target: &CleanupTarget) -> crate::Result<ExecutionOutcome> {
            let calls = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            if let Some((limit, cancellation)) = &self.cancel_after_calls
                && calls >= *limit
            {
                cancellation.cancel();
            }
            Ok(ExecutionOutcome {
                freed_bytes: 1,
                pending_reclaim_bytes: 0,
                note: None,
            })
        }
    }

    fn executable_plan(target_count: usize) -> CleanupPlan {
        executable_plan_with_paths(
            (0..target_count).map(|index| std::path::PathBuf::from(format!("target-{index}"))),
        )
    }

    fn executable_plan_with_paths(
        paths: impl IntoIterator<Item = std::path::PathBuf>,
    ) -> CleanupPlan {
        let mut plan = CleanupPlan::empty(PlanRequest::for_platform(
            Platform::current(),
            DeleteMode::RecoverableDelete,
        ));
        plan.targets = paths
            .into_iter()
            .enumerate()
            .map(|(index, path)| {
                CleanupTarget::allowed(
                    format!("test.rule.{index}"),
                    path,
                    1,
                    DeleteMode::RecoverableDelete,
                )
            })
            .collect();
        plan.recompute_summary();
        plan
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

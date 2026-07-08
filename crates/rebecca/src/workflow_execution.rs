use std::path::PathBuf;

use rebecca::core::DeleteMode;
use rebecca::core::RebeccaError;
use rebecca::core::execution::{ExecutionReport, ExecutionWarning};
use rebecca::core::executor::{
    PermanentDeleteBackend, execute_cleanup_plan_parallel_with_policy_and_cancellation,
};
use rebecca::core::history::HistoryStore;
use rebecca::core::plan::CleanupPlan;
use rebecca::core::protection::ProtectionPolicy;
use rebecca::core::scan::ScanCancellationToken;

use crate::trash_backend::recoverable_trash_backend;

pub(crate) fn execute_plan(
    plan: &mut CleanupPlan,
    execution_policy: ProtectionPolicy<'_>,
    cancellation: &ScanCancellationToken,
    mode: DeleteMode,
) -> std::result::Result<ExecutionReport, RebeccaError> {
    match mode {
        DeleteMode::RecoverableDelete => {
            let backend = recoverable_trash_backend();
            execute_cleanup_plan_parallel_with_policy_and_cancellation(
                plan,
                &backend,
                execution_policy,
                cancellation,
            )
        }
        DeleteMode::PermanentDelete => {
            let backend = PermanentDeleteBackend;
            execute_cleanup_plan_parallel_with_policy_and_cancellation(
                plan,
                &backend,
                execution_policy,
                cancellation,
            )
        }
        DeleteMode::DryRun => unreachable!("dry-run returns before execution"),
    }
}

pub(crate) fn record_execution_report(
    plan: &mut CleanupPlan,
    execution_report: &mut ExecutionReport,
    history_file: PathBuf,
) -> Option<ExecutionWarning> {
    let history_append = HistoryStore::new(history_file).append_plan_report(plan);
    let warning = history_append.warning;
    if let Some(warning) = warning.clone() {
        execution_report.push_warning(warning);
    }
    plan.execution_report = Some(std::mem::take(execution_report));
    warning
}

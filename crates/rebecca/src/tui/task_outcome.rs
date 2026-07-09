use anyhow::Result;
use rebecca_core::RebeccaError;
use rebecca_core::config::AppRuntimeConfig;
use rebecca_core::disk_session::DiskMapSession;

use crate::tui::app::TuiApp;
use crate::tui::effect::TuiEffect;

pub(super) enum TaskOutcome {
    Scan(Result<DiskMapSession, TaskFailure>),
    Refresh(Result<TuiRefreshResult, TaskFailure>),
    Preview(Result<rebecca_core::CleanupPlan, TaskFailure>),
    Execute(Result<rebecca_core::CleanupPlan, TaskFailure>),
}

pub(super) struct TuiRefreshResult {
    pub(super) anchor: std::path::PathBuf,
    pub(super) session: DiskMapSession,
}

pub(super) struct TaskFailure {
    message: String,
    cancelled: bool,
}

pub(super) fn apply_outcome(
    app: &mut TuiApp,
    outcome: TaskOutcome,
    runtime_config: &AppRuntimeConfig,
    retry_effect: TuiEffect,
) -> Result<()> {
    match outcome {
        TaskOutcome::Scan(result) => match result {
            Ok(session) => app.apply_scan_result(session),
            Err(err) => apply_failure(app, err, retry_effect),
        },
        TaskOutcome::Refresh(result) => match result {
            Ok(result) => app.apply_refresh_result(result.anchor, result.session),
            Err(err) => apply_failure(app, err, retry_effect),
        },
        TaskOutcome::Preview(result) => match result {
            Ok(plan) => app.apply_preview(plan),
            Err(err) => apply_failure(app, err, retry_effect),
        },
        TaskOutcome::Execute(result) => match result {
            Ok(plan) => {
                app.apply_execution(plan);
                app.set_history(super::load_recent_history(runtime_config)?);
            }
            Err(err) => apply_failure(app, err, retry_effect),
        },
    }
    Ok(())
}

pub(super) fn task_failure(err: anyhow::Error) -> TaskFailure {
    let cancelled = err
        .downcast_ref::<RebeccaError>()
        .is_some_and(|err| matches!(err, RebeccaError::OperationCancelled(_)));
    TaskFailure {
        message: err.to_string(),
        cancelled,
    }
}

fn apply_failure(app: &mut TuiApp, failure: TaskFailure, retry_effect: TuiEffect) {
    if failure.cancelled {
        app.apply_task_cancelled();
    } else {
        app.apply_task_error(failure.message, retry_effect);
    }
}

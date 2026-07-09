use std::path::PathBuf;

use anyhow::Error;
use indicatif::ProgressBar;
use rebecca_core::DeleteMode;
use rebecca_core::RebeccaError;
use rebecca_core::execution::{ExecutionProgressEvent, ExecutionReport, ExecutionWarning};
use rebecca_core::executor::{
    PermanentDeleteBackend, execute_cleanup_plan_parallel_with_policy_and_cancellation_and_progress,
};
use rebecca_core::history::HistoryStore;
use rebecca_core::plan::CleanupPlan;
use rebecca_core::protection::ProtectionPolicy;
use rebecca_core::scan::ScanCancellationToken;

use crate::output::{NdjsonEventWriter, format_bytes};
use crate::progress::{PROGRESS_PATH_MAX_CHARS, compact_progress_path, stderr_spinner};
use crate::trash_backend::recoverable_trash_backend;

pub(crate) fn execute_plan_with_progress<F>(
    plan: &mut CleanupPlan,
    execution_policy: ProtectionPolicy<'_>,
    cancellation: &ScanCancellationToken,
    mode: DeleteMode,
    progress: F,
) -> std::result::Result<ExecutionReport, RebeccaError>
where
    F: for<'event> FnMut(ExecutionProgressEvent<'event>),
{
    match mode {
        DeleteMode::RecoverableDelete => {
            let backend = recoverable_trash_backend();
            execute_cleanup_plan_parallel_with_policy_and_cancellation_and_progress(
                plan,
                &backend,
                execution_policy,
                cancellation,
                progress,
            )
        }
        DeleteMode::PermanentDelete => {
            let backend = PermanentDeleteBackend;
            execute_cleanup_plan_parallel_with_policy_and_cancellation_and_progress(
                plan,
                &backend,
                execution_policy,
                cancellation,
                progress,
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

pub(crate) struct ExecutionProgressReporter {
    bar: Option<ProgressBar>,
    event_writer: Option<NdjsonEventWriter>,
    event_error: Option<Error>,
    targets_started: u64,
    targets_finished: u64,
    executable_targets: usize,
}

impl ExecutionProgressReporter {
    pub(crate) fn new(human_enabled: bool, event_writer: Option<NdjsonEventWriter>) -> Self {
        Self {
            bar: stderr_spinner(
                human_enabled,
                "execute | preparing cleanup | Ctrl+C cancels",
            ),
            event_writer,
            event_error: None,
            targets_started: 0,
            targets_finished: 0,
            executable_targets: 0,
        }
    }

    pub(crate) fn on_event(&mut self, event: ExecutionProgressEvent<'_>) {
        if self.event_error.is_none()
            && let Some(writer) = &mut self.event_writer
            && let Err(err) = writer.emit_execution_progress(event.clone())
        {
            self.event_error = Some(err);
        }

        let Some(bar) = &self.bar else {
            return;
        };

        match event {
            ExecutionProgressEvent::Started {
                executable_targets,
                estimated_bytes,
                mode,
                ..
            } => {
                self.executable_targets = executable_targets;
                bar.set_message(format!(
                    "execute | {} | {} selected | {} | Ctrl+C cancels",
                    execution_target_count(executable_targets),
                    format_bytes(estimated_bytes),
                    delete_mode_label(mode)
                ));
            }
            ExecutionProgressEvent::TargetStarted { target, .. } => {
                self.targets_started = self.targets_started.saturating_add(1);
                bar.set_message(format!(
                    "execute | target {}/{} | {} | {} | Ctrl+C cancels",
                    self.targets_started,
                    self.executable_targets,
                    target.rule_id,
                    compact_progress_path(&target.path, PROGRESS_PATH_MAX_CHARS)
                ));
            }
            ExecutionProgressEvent::TargetFinished { target, .. } => {
                self.targets_finished = self.targets_finished.saturating_add(1);
                bar.set_message(format!(
                    "execute | {}/{} finished | {} | {} | now {} pending {}",
                    self.targets_finished,
                    self.executable_targets,
                    target.status.label(),
                    target.rule_id,
                    format_bytes(target.freed_bytes),
                    format_bytes(target.pending_reclaim_bytes)
                ));
                bar.tick();
            }
            ExecutionProgressEvent::Completed { summary } => {
                bar.set_message(format!(
                    "execute | complete | {} completed | freed {} | pending {}",
                    summary.completed_actions,
                    format_bytes(summary.confirmed_reclaimed_bytes),
                    format_bytes(summary.pending_reclaim_bytes)
                ));
                bar.tick();
            }
        }
    }

    pub(crate) fn finish(&self) {
        if let Some(bar) = &self.bar {
            bar.finish_and_clear();
        }
    }

    pub(crate) fn take_event_error(&mut self) -> Option<Error> {
        self.event_error.take()
    }

    pub(crate) fn into_event_writer(self) -> Option<NdjsonEventWriter> {
        self.event_writer
    }
}

fn execution_target_count(count: usize) -> String {
    match count {
        1 => "1 target".to_string(),
        _ => format!("{count} targets"),
    }
}

fn delete_mode_label(mode: DeleteMode) -> &'static str {
    match mode {
        DeleteMode::DryRun => "dry-run",
        DeleteMode::RecoverableDelete => "recoverable-delete",
        DeleteMode::PermanentDelete => "permanent-delete",
    }
}

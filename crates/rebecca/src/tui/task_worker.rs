use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::thread::{self, JoinHandle};

use anyhow::{Context, Result, bail};
use rebecca::core::config::AppRuntimeConfig;
use rebecca::core::disk_map::{
    DiskMapGroupKind, DiskMapRequest, DiskMapSortField,
    inspect_map_with_progress as inspect_map_core,
};
use rebecca::core::disk_session::DiskMapSession;
use rebecca::core::scan::{ScanBackendKind, ScanCancellationToken};

use crate::runtime::CliRuntime;
use crate::tui::app::TuiApp;
use crate::tui::effect::TuiEffect;
use crate::tui::progress::{TuiTaskId, TuiTaskProgressEvent};
use crate::tui::task_outcome::{TaskOutcome, TuiRefreshResult, task_failure};
use crate::tui::task_progress::{inspect_progress_event, plan_progress_sender, progress_sender};

pub(super) const TASK_CHANNEL_CAPACITY: usize = 256;

pub(super) struct ActiveTask {
    pub(super) id: TuiTaskId,
    pub(super) label: &'static str,
    pub(super) effect: TuiEffect,
    pub(super) cancellation: ScanCancellationToken,
    pub(super) receiver: Receiver<TaskMessage>,
    pub(super) handle: JoinHandle<()>,
}

impl ActiveTask {
    pub(super) fn cancel(&self) {
        self.cancellation.cancel();
    }

    pub(super) fn cancel_and_join(self) {
        self.cancel();
        let _ = self.handle.join();
    }
}

pub(super) enum TaskMessage {
    Progress {
        task_id: TuiTaskId,
        event: TuiTaskProgressEvent,
    },
    Finished {
        task_id: TuiTaskId,
        outcome: Box<TaskOutcome>,
    },
}

impl TaskMessage {
    pub(super) fn is_coalescible_progress(&self) -> bool {
        matches!(self, Self::Progress { event, .. } if event.is_coalescible())
    }
}

pub(super) fn spawn_task(
    app: &mut TuiApp,
    effect: TuiEffect,
    runtime_config: &AppRuntimeConfig,
    runtime: &CliRuntime,
) -> Result<ActiveTask> {
    let runtime_config = runtime_config.clone();
    let task_runtime = runtime.child_task();
    let cancellation = task_runtime.cancellation().clone();
    let (sender, receiver) = mpsc::sync_channel(TASK_CHANNEL_CAPACITY);
    let active_effect = effect.clone();
    let task_id = app.allocate_task_id();

    let (label, handle) = match effect {
        TuiEffect::None | TuiEffect::CancelTask | TuiEffect::Quit => {
            bail!("non-background tui effect cannot start a task")
        }
        TuiEffect::Scan(roots) => {
            let entry_limit = app.entry_limit;
            let scan_backend = app.scan_backend;
            app.apply_task_started(format!("Scanning {} root(s)...", roots.len()));
            (
                "scan",
                thread::spawn(move || {
                    let result = scan_session_with_progress(
                        roots,
                        entry_limit,
                        scan_backend,
                        &runtime_config,
                        &task_runtime,
                        progress_sender(task_id, sender.clone()),
                    )
                    .map_err(task_failure);
                    let _ = sender.send(TaskMessage::Finished {
                        task_id,
                        outcome: Box::new(TaskOutcome::Scan(result)),
                    });
                }),
            )
        }
        TuiEffect::Refresh { anchor } => {
            let entry_limit = app.entry_limit;
            let scan_backend = app.scan_backend;
            app.apply_task_started(format!("Refreshing {}...", anchor.display()));
            (
                "refresh",
                thread::spawn(move || {
                    let roots = vec![anchor.clone()];
                    let result = scan_session_with_progress(
                        roots,
                        entry_limit,
                        scan_backend,
                        &runtime_config,
                        &task_runtime,
                        progress_sender(task_id, sender.clone()),
                    )
                    .map(|session| TuiRefreshResult { anchor, session })
                    .map_err(task_failure);
                    let _ = sender.send(TaskMessage::Finished {
                        task_id,
                        outcome: Box::new(TaskOutcome::Refresh(result)),
                    });
                }),
            )
        }
        TuiEffect::Preview(request) => {
            app.apply_task_started("Building cleanup preview...");
            (
                "preview",
                thread::spawn(move || {
                    let result = crate::workbench::preview_cleanup_plan_with_progress(
                        &request,
                        &runtime_config,
                        &task_runtime,
                        plan_progress_sender(task_id, sender.clone()),
                    )
                    .map_err(task_failure);
                    let _ = sender.send(TaskMessage::Finished {
                        task_id,
                        outcome: Box::new(TaskOutcome::Preview(result)),
                    });
                }),
            )
        }
        TuiEffect::Execute(request) => {
            app.apply_task_started("Moving allowed targets to the system trash...");
            (
                "execute",
                thread::spawn(move || {
                    let result = crate::workbench::execute_recoverable_cleanup_with_progress(
                        &request,
                        &runtime_config,
                        &task_runtime,
                        plan_progress_sender(task_id, sender.clone()),
                    )
                    .map_err(task_failure);
                    if let Ok(plan) = &result {
                        let _ = sender.send(TaskMessage::Progress {
                            task_id,
                            event: TuiTaskProgressEvent::ExecutionFinished {
                                completed_targets: plan.summary.completed_targets as u64,
                                freed_bytes: plan.summary.freed_bytes,
                                pending_reclaim_bytes: plan.summary.pending_reclaim_bytes,
                            },
                        });
                    }
                    let _ = sender.send(TaskMessage::Finished {
                        task_id,
                        outcome: Box::new(TaskOutcome::Execute(result)),
                    });
                }),
            )
        }
    };

    Ok(ActiveTask {
        id: task_id,
        label,
        effect: active_effect,
        cancellation,
        receiver,
        handle,
    })
}

pub(super) fn scan_session(
    roots: Vec<PathBuf>,
    entry_limit: usize,
    scan_backend: ScanBackendKind,
    runtime_config: &AppRuntimeConfig,
    runtime: &CliRuntime,
) -> Result<DiskMapSession> {
    scan_session_with_progress(
        roots,
        entry_limit,
        scan_backend,
        runtime_config,
        runtime,
        |_| Ok(()),
    )
}

fn scan_session_with_progress<F>(
    roots: Vec<PathBuf>,
    entry_limit: usize,
    scan_backend: ScanBackendKind,
    runtime_config: &AppRuntimeConfig,
    runtime: &CliRuntime,
    mut progress: F,
) -> Result<DiskMapSession>
where
    F: FnMut(TuiTaskProgressEvent) -> rebecca::core::Result<()>,
{
    let roots = resolve_roots(roots)?;
    let request = DiskMapRequest::new(roots)
        .with_top_limit(entry_limit.max(1))
        .with_top_sort(DiskMapSortField::Logical)
        .with_group_kinds(vec![DiskMapGroupKind::Type, DiskMapGroupKind::Extension])
        .with_group_limit(entry_limit.max(25))
        .with_group_sort(DiskMapSortField::Logical)
        .with_diagnostic_limit(100)
        .with_scan_backend(scan_backend);
    let mut report = inspect_map_core(&request, runtime.cancellation(), |event| {
        progress(inspect_progress_event(event))
    })?;
    crate::inspect::annotate_map_report_with_cleanup_advice(
        &mut report,
        runtime_config,
        None,
        runtime.cancellation(),
    )?;
    Ok(DiskMapSession::from_report(report))
}

fn resolve_roots(roots: Vec<PathBuf>) -> Result<Vec<PathBuf>> {
    roots
        .into_iter()
        .map(|root| {
            if root.as_os_str().is_empty() {
                bail!("tui root cannot be empty");
            }
            if root.is_absolute() {
                Ok(root)
            } else {
                Ok(std::env::current_dir()
                    .context("failed to resolve current directory")?
                    .join(root))
            }
        })
        .collect()
}

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread::{self, JoinHandle};

use anyhow::{Context, Result, bail};
use rebecca::core::config::AppRuntimeConfig;
use rebecca::core::disk_map::{
    DiskMapGroupKind, DiskMapRequest, DiskMapSortField,
    inspect_map_with_progress as inspect_map_core,
};
use rebecca::core::disk_session::DiskMapSession;
use rebecca::core::planner::PlanProgressEvent;
use rebecca::core::progress::InspectProgressEvent;
use rebecca::core::scan::{ScanBackendKind, ScanCancellationToken};
use rebecca::core::{CleanupPlan, RebeccaError};

use crate::runtime::CliRuntime;
use crate::tui::app::{TuiApp, TuiEffect, TuiTaskId, TuiTaskProgressEvent};

pub(super) struct ActiveTask {
    id: TuiTaskId,
    label: &'static str,
    effect: TuiEffect,
    cancellation: ScanCancellationToken,
    receiver: Receiver<TaskMessage>,
    handle: JoinHandle<()>,
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

enum TaskMessage {
    Progress {
        task_id: TuiTaskId,
        event: TuiTaskProgressEvent,
    },
    Finished {
        task_id: TuiTaskId,
        outcome: Box<TaskOutcome>,
    },
}

enum TaskOutcome {
    Scan(Result<DiskMapSession, TaskFailure>),
    Refresh(Result<DiskMapSession, TaskFailure>),
    Preview(Result<CleanupPlan, TaskFailure>),
    Execute(Result<CleanupPlan, TaskFailure>),
}

struct TaskFailure {
    message: String,
    cancelled: bool,
}

pub(super) fn start(
    app: &mut TuiApp,
    effect: TuiEffect,
    runtime_config: &AppRuntimeConfig,
    runtime: &CliRuntime,
) -> Result<Option<ActiveTask>> {
    let runtime_config = runtime_config.clone();
    let task_runtime = runtime.child_task();
    let cancellation = task_runtime.cancellation().clone();
    let (sender, receiver) = mpsc::channel();
    let active_effect = effect.clone();
    let task_id = app.allocate_task_id();

    let (label, handle) = match effect {
        TuiEffect::None | TuiEffect::CancelTask | TuiEffect::Quit => return Ok(None),
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
        TuiEffect::Refresh(roots) => {
            let entry_limit = app.entry_limit;
            let scan_backend = app.scan_backend;
            app.prepare_refresh();
            app.apply_task_started(format!("Refreshing {} root(s)...", roots.len()));
            (
                "refresh",
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
            app.apply_task_started("Moving allowed targets to recoverable trash...");
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

    Ok(Some(ActiveTask {
        id: task_id,
        label,
        effect: active_effect,
        cancellation,
        receiver,
        handle,
    }))
}

pub(super) fn poll(
    app: &mut TuiApp,
    active_task: &mut Option<ActiveTask>,
    runtime_config: &AppRuntimeConfig,
) -> Result<()> {
    let mut outcome = None;
    let mut disconnected = false;

    if let Some(task) = active_task.as_ref() {
        loop {
            match task.receiver.try_recv() {
                Ok(TaskMessage::Progress { task_id, event }) => {
                    if task_id == task.id {
                        app.apply_task_progress(event);
                    }
                }
                Ok(TaskMessage::Finished {
                    task_id,
                    outcome: result,
                }) => {
                    if task_id == task.id {
                        outcome = Some(*result);
                        break;
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }
    }

    if let Some(outcome) = outcome {
        if let Some(task) = active_task.take() {
            let effect = task.effect.clone();
            let _ = task.handle.join();
            apply_outcome(app, outcome, runtime_config, effect)?;
        }
    } else if disconnected && let Some(task) = active_task.take() {
        let label = task.label;
        let _ = task.handle.join();
        app.apply_error(format!("{label} worker stopped before reporting a result"));
    }

    Ok(())
}

fn apply_outcome(
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
            Ok(session) => app.apply_refresh_result(session),
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

fn apply_failure(app: &mut TuiApp, failure: TaskFailure, retry_effect: TuiEffect) {
    if failure.cancelled {
        app.apply_task_cancelled();
    } else {
        app.apply_task_error(failure.message, retry_effect);
    }
}

fn task_failure(err: anyhow::Error) -> TaskFailure {
    let cancelled = err
        .downcast_ref::<RebeccaError>()
        .is_some_and(|err| matches!(err, RebeccaError::OperationCancelled(_)));
    TaskFailure {
        message: err.to_string(),
        cancelled,
    }
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

fn progress_sender(
    task_id: TuiTaskId,
    sender: Sender<TaskMessage>,
) -> impl FnMut(TuiTaskProgressEvent) -> rebecca::core::Result<()> {
    move |event| {
        sender
            .send(TaskMessage::Progress { task_id, event })
            .map_err(|_| {
                RebeccaError::OperationCancelled("tui task receiver was closed".to_string())
            })
    }
}

fn plan_progress_sender(
    task_id: TuiTaskId,
    sender: Sender<TaskMessage>,
) -> impl for<'a> FnMut(PlanProgressEvent<'a>) {
    move |event| {
        let _ = sender.send(TaskMessage::Progress {
            task_id,
            event: plan_progress_event(event),
        });
    }
}

fn inspect_progress_event(event: InspectProgressEvent<'_>) -> TuiTaskProgressEvent {
    match event {
        InspectProgressEvent::RootStarted {
            root_index,
            root_count,
            root,
            backend,
        } => TuiTaskProgressEvent::RootStarted {
            root_index,
            root_count,
            root: root.to_path_buf(),
            backend: backend.label().to_string(),
        },
        InspectProgressEvent::RootFinished {
            root_index,
            root_count,
            root,
            status,
            logical_bytes,
            files,
            directories,
        } => TuiTaskProgressEvent::RootFinished {
            root_index,
            root_count,
            root: root.to_path_buf(),
            status: status.label().to_string(),
            logical_bytes,
            files,
            directories,
        },
        InspectProgressEvent::EntryStarted {
            root, entry_index, ..
        } => TuiTaskProgressEvent::Traversal {
            root: root.to_path_buf(),
            counter: "entries".to_string(),
            value: entry_index,
            logical_bytes: 0,
            files: 0,
            directories: 0,
        },
        InspectProgressEvent::EntryMeasured {
            root,
            logical_bytes,
            files,
            directories,
            entry_index,
            ..
        } => TuiTaskProgressEvent::Traversal {
            root: root.to_path_buf(),
            counter: "entries".to_string(),
            value: entry_index,
            logical_bytes,
            files,
            directories,
        },
        InspectProgressEvent::FileMeasured {
            target_path,
            path,
            file_size,
            files_scanned,
            bytes_scanned,
            ..
        } => TuiTaskProgressEvent::FileMeasured {
            target_path: target_path.to_path_buf(),
            path: path.to_path_buf(),
            file_size,
            files_scanned,
            bytes_scanned,
        },
        InspectProgressEvent::TraversalProgress {
            root,
            counter,
            value,
            logical_bytes,
            files,
            directories,
        } => TuiTaskProgressEvent::Traversal {
            root: root.to_path_buf(),
            counter: counter.label().to_string(),
            value,
            logical_bytes,
            files,
            directories,
        },
        InspectProgressEvent::BackendFallback {
            root,
            backend,
            reason,
        } => TuiTaskProgressEvent::BackendFallback {
            root: root.to_path_buf(),
            backend: backend.label().to_string(),
            reason: reason.to_string(),
        },
        InspectProgressEvent::BackendStageStarted {
            root,
            backend,
            stage,
        } => TuiTaskProgressEvent::BackendStage {
            root: root.to_path_buf(),
            backend: backend.label().to_string(),
            stage,
            finished: false,
        },
        InspectProgressEvent::BackendStageFinished {
            root,
            backend,
            stage,
        } => TuiTaskProgressEvent::BackendStage {
            root: root.to_path_buf(),
            backend: backend.label().to_string(),
            stage,
            finished: true,
        },
        InspectProgressEvent::BackendMetric { metric, value, .. } => {
            TuiTaskProgressEvent::BackendMetric { metric, value }
        }
        InspectProgressEvent::CacheEvent {
            path,
            event,
            reason,
            estimated_bytes,
        } => match event {
            rebecca::core::progress::InspectProgressCacheEvent::Hit => {
                TuiTaskProgressEvent::CleanupCacheHit {
                    rule_id: "inspect-map".to_string(),
                    path: path.to_path_buf(),
                    estimated_bytes: estimated_bytes.unwrap_or(0),
                }
            }
            rebecca::core::progress::InspectProgressCacheEvent::Miss => {
                TuiTaskProgressEvent::CleanupCacheMiss {
                    rule_id: "inspect-map".to_string(),
                    path: path.to_path_buf(),
                    reason: reason.unwrap_or("unknown").to_string(),
                    pruned: false,
                }
            }
            rebecca::core::progress::InspectProgressCacheEvent::WriteSkipped => {
                TuiTaskProgressEvent::CleanupCacheWriteSkipped {
                    rule_id: "inspect-map".to_string(),
                    path: path.to_path_buf(),
                }
            }
        },
        InspectProgressEvent::Finalizing {
            roots,
            logical_bytes,
            files,
            directories,
        } => TuiTaskProgressEvent::Finalizing {
            roots,
            logical_bytes,
            files,
            directories,
        },
    }
}

fn plan_progress_event(event: PlanProgressEvent<'_>) -> TuiTaskProgressEvent {
    match event {
        PlanProgressEvent::TargetScanning { rule_id, path } => {
            TuiTaskProgressEvent::CleanupTargetScanning {
                rule_id: rule_id.to_string(),
                path: path.to_path_buf(),
            }
        }
        PlanProgressEvent::TargetFinished {
            rule_id,
            path,
            status,
            estimated_bytes,
        } => TuiTaskProgressEvent::CleanupTargetFinished {
            rule_id: rule_id.to_string(),
            path: path.to_path_buf(),
            status: status.label().to_string(),
            estimated_bytes,
        },
        PlanProgressEvent::FileMeasured {
            rule_id,
            target_path,
            path,
            file_size,
            files_scanned,
            bytes_scanned,
        } => TuiTaskProgressEvent::CleanupFileMeasured {
            rule_id: rule_id.to_string(),
            target_path: target_path.to_path_buf(),
            path: path.to_path_buf(),
            file_size,
            files_scanned,
            bytes_scanned,
        },
        PlanProgressEvent::ScanCacheHit {
            rule_id,
            path,
            estimated_bytes,
        } => TuiTaskProgressEvent::CleanupCacheHit {
            rule_id: rule_id.to_string(),
            path: path.to_path_buf(),
            estimated_bytes,
        },
        PlanProgressEvent::ScanCacheMiss {
            rule_id,
            path,
            reason,
            pruned,
        } => TuiTaskProgressEvent::CleanupCacheMiss {
            rule_id: rule_id.to_string(),
            path: path.to_path_buf(),
            reason: reason.label().to_string(),
            pruned,
        },
        PlanProgressEvent::ScanCacheWriteSkipped { rule_id, path } => {
            TuiTaskProgressEvent::CleanupCacheWriteSkipped {
                rule_id: rule_id.to_string(),
                path: path.to_path_buf(),
            }
        }
        PlanProgressEvent::ScanCachePruned { report } => TuiTaskProgressEvent::CleanupCachePruned {
            inspected: report.inspected,
            pruned: report.pruned,
            retained: report.retained,
        },
    }
}

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError};
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
use crate::tui::app::TuiApp;
use crate::tui::effect::TuiEffect;
use crate::tui::progress::{TuiTaskId, TuiTaskProgressEvent};

const TASK_CHANNEL_CAPACITY: usize = 256;

pub(super) struct TuiTaskManager {
    active: Option<ActiveTask>,
}

impl TuiTaskManager {
    pub(super) fn new() -> Self {
        Self { active: None }
    }

    pub(super) fn handle_effect(
        &mut self,
        app: &mut TuiApp,
        effect: TuiEffect,
        runtime_config: &AppRuntimeConfig,
        runtime: &CliRuntime,
    ) -> Result<()> {
        match effect {
            TuiEffect::None | TuiEffect::Quit => Ok(()),
            TuiEffect::CancelTask => {
                self.cancel(app);
                Ok(())
            }
            TuiEffect::Scan(_)
            | TuiEffect::Refresh { .. }
            | TuiEffect::Preview(_)
            | TuiEffect::Execute(_) => self.start(app, effect, runtime_config, runtime),
        }
    }

    pub(super) fn poll(
        &mut self,
        app: &mut TuiApp,
        runtime_config: &AppRuntimeConfig,
    ) -> Result<()> {
        let mut outcome = None;
        let mut disconnected = false;
        let mut pending_progress = None;

        if let Some(task) = self.active.as_ref() {
            loop {
                match task.receiver.try_recv() {
                    Ok(TaskMessage::Progress { task_id, event }) => {
                        if task_id == task.id {
                            if event.is_coalescible() {
                                pending_progress = Some(event);
                            } else {
                                apply_pending_progress(app, &mut pending_progress);
                                app.apply_task_progress(event);
                            }
                        }
                    }
                    Ok(TaskMessage::Finished {
                        task_id,
                        outcome: result,
                    }) => {
                        if task_id == task.id {
                            apply_pending_progress(app, &mut pending_progress);
                            outcome = Some(*result);
                            break;
                        }
                    }
                    Err(TryRecvError::Empty) => {
                        apply_pending_progress(app, &mut pending_progress);
                        break;
                    }
                    Err(TryRecvError::Disconnected) => {
                        apply_pending_progress(app, &mut pending_progress);
                        disconnected = true;
                        break;
                    }
                }
            }
        }

        if let Some(outcome) = outcome {
            if let Some(task) = self.active.take() {
                let effect = task.effect.clone();
                let _ = task.handle.join();
                apply_outcome(app, outcome, runtime_config, effect)?;
            }
        } else if disconnected && let Some(task) = self.active.take() {
            let label = task.label;
            let _ = task.handle.join();
            app.apply_error(format!("{label} worker stopped before reporting a result"));
        }

        Ok(())
    }

    pub(super) fn shutdown(&mut self) {
        if let Some(task) = self.active.take() {
            task.cancel_and_join();
        }
    }

    fn start(
        &mut self,
        app: &mut TuiApp,
        effect: TuiEffect,
        runtime_config: &AppRuntimeConfig,
        runtime: &CliRuntime,
    ) -> Result<()> {
        if self.active.is_some() {
            app.apply_task_already_running();
            return Ok(());
        }

        self.active = Some(spawn_task(app, effect, runtime_config, runtime)?);
        Ok(())
    }

    fn cancel(&mut self, app: &mut TuiApp) {
        if let Some(task) = self.active.as_ref() {
            task.cancel();
            app.apply_cancel_requested();
        } else {
            app.apply_error("no active background task to cancel");
        }
    }
}

impl Drop for TuiTaskManager {
    fn drop(&mut self) {
        self.shutdown();
    }
}

struct ActiveTask {
    id: TuiTaskId,
    label: &'static str,
    effect: TuiEffect,
    cancellation: ScanCancellationToken,
    receiver: Receiver<TaskMessage>,
    handle: JoinHandle<()>,
}

impl ActiveTask {
    fn cancel(&self) {
        self.cancellation.cancel();
    }

    fn cancel_and_join(self) {
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

impl TaskMessage {
    fn is_coalescible_progress(&self) -> bool {
        matches!(self, Self::Progress { event, .. } if event.is_coalescible())
    }
}

enum TaskOutcome {
    Scan(Result<DiskMapSession, TaskFailure>),
    Refresh(Result<TuiRefreshResult, TaskFailure>),
    Preview(Result<CleanupPlan, TaskFailure>),
    Execute(Result<CleanupPlan, TaskFailure>),
}

struct TuiRefreshResult {
    anchor: PathBuf,
    session: DiskMapSession,
}

struct TaskFailure {
    message: String,
    cancelled: bool,
}

fn spawn_task(
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

    Ok(ActiveTask {
        id: task_id,
        label,
        effect: active_effect,
        cancellation,
        receiver,
        handle,
    })
}

fn apply_pending_progress(app: &mut TuiApp, pending_progress: &mut Option<TuiTaskProgressEvent>) {
    if let Some(event) = pending_progress.take() {
        app.apply_task_progress(event);
    }
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
    sender: SyncSender<TaskMessage>,
) -> impl FnMut(TuiTaskProgressEvent) -> rebecca::core::Result<()> {
    move |event| {
        send_progress_message(&sender, TaskMessage::Progress { task_id, event })
            .map_err(|_| tui_receiver_closed())
    }
}

fn plan_progress_sender(
    task_id: TuiTaskId,
    sender: SyncSender<TaskMessage>,
) -> impl for<'a> FnMut(PlanProgressEvent<'a>) {
    move |event| {
        let _ = send_progress_message(
            &sender,
            TaskMessage::Progress {
                task_id,
                event: plan_progress_event(event),
            },
        );
    }
}

fn send_progress_message(
    sender: &SyncSender<TaskMessage>,
    message: TaskMessage,
) -> std::result::Result<(), TaskSendError> {
    match sender.try_send(message) {
        Ok(()) => Ok(()),
        Err(TrySendError::Full(message)) if message.is_coalescible_progress() => Ok(()),
        Err(TrySendError::Full(message)) => sender.send(message).map_err(|_| TaskSendError),
        Err(TrySendError::Disconnected(_)) => Err(TaskSendError),
    }
}

fn tui_receiver_closed() -> RebeccaError {
    RebeccaError::OperationCancelled("tui task receiver was closed".to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TaskSendError;

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

#[cfg(test)]
mod tests {
    use super::*;
    use rebecca::core::config::{AppPaths, PurgeRuntimeConfig};
    use rebecca::core::scan_cache::ScanCachePolicy;

    #[test]
    fn task_manager_rejects_second_background_task() {
        let temp = tempfile::tempdir().unwrap();
        let mut app = TuiApp::root_picker(Vec::new(), ScanBackendKind::PortableRecursive, 10);
        let (active_task, _sender, _cancellation) = active_task_fixture(TuiTaskId(7));
        let mut manager = TuiTaskManager {
            active: Some(active_task),
        };
        let runtime_config = runtime_config_fixture(temp.path().to_path_buf());
        let runtime = CliRuntime::new(ScanCancellationToken::new());

        manager
            .start(
                &mut app,
                TuiEffect::Scan(vec![temp.path().to_path_buf()]),
                &runtime_config,
                &runtime,
            )
            .unwrap();

        assert_eq!(app.message, "A background task is already running.");
        assert_eq!(
            manager.active.as_ref().map(|task| task.id),
            Some(TuiTaskId(7))
        );
        manager.shutdown();
    }

    #[test]
    fn task_manager_cancel_marks_task_and_ui() {
        let mut app = TuiApp::root_picker(Vec::new(), ScanBackendKind::PortableRecursive, 10);
        app.apply_task_started("Scanning fixture...");
        let (active_task, _sender, cancellation) = active_task_fixture(TuiTaskId(7));
        let mut manager = TuiTaskManager {
            active: Some(active_task),
        };

        manager.cancel(&mut app);

        assert!(cancellation.is_cancelled());
        let status = app.task_status.as_ref().unwrap();
        assert!(status.cancel_requested);
        assert_eq!(status.phase, "Cancel requested");
        manager.shutdown();
    }

    #[test]
    fn task_manager_ignores_stale_progress() {
        let temp = tempfile::tempdir().unwrap();
        let mut app = TuiApp::root_picker(Vec::new(), ScanBackendKind::PortableRecursive, 10);
        let (active_task, sender, _cancellation) = active_task_fixture(TuiTaskId(7));
        sender
            .send(TaskMessage::Progress {
                task_id: TuiTaskId(6),
                event: important_progress(),
            })
            .unwrap();
        let mut manager = TuiTaskManager {
            active: Some(active_task),
        };
        let runtime_config = runtime_config_fixture(temp.path().to_path_buf());

        manager.poll(&mut app, &runtime_config).unwrap();

        assert!(app.task_status.is_none());
        assert_eq!(
            manager.active.as_ref().map(|task| task.id),
            Some(TuiTaskId(7))
        );
        drop(sender);
        manager.shutdown();
    }

    #[test]
    fn full_task_channel_drops_coalescible_progress() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let task_id = TuiTaskId(7);
        sender
            .send(TaskMessage::Progress {
                task_id,
                event: important_progress(),
            })
            .unwrap();

        send_progress_message(
            &sender,
            TaskMessage::Progress {
                task_id,
                event: coalescible_progress(2),
            },
        )
        .unwrap();

        drop(sender);
        let messages = receiver.try_iter().collect::<Vec<_>>();
        assert_eq!(messages.len(), 1);
        assert!(matches!(
            &messages[0],
            TaskMessage::Progress {
                event: TuiTaskProgressEvent::BackendMetric { .. },
                ..
            }
        ));
    }

    #[test]
    fn disconnected_task_channel_reports_send_error() {
        let (sender, receiver) = mpsc::sync_channel(1);
        drop(receiver);

        assert_eq!(
            send_progress_message(
                &sender,
                TaskMessage::Progress {
                    task_id: TuiTaskId(7),
                    event: important_progress(),
                },
            ),
            Err(TaskSendError)
        );
    }

    #[test]
    fn task_message_classifies_only_coalescible_progress() {
        assert!(
            TaskMessage::Progress {
                task_id: TuiTaskId(7),
                event: coalescible_progress(1),
            }
            .is_coalescible_progress()
        );
        assert!(
            !TaskMessage::Progress {
                task_id: TuiTaskId(7),
                event: important_progress(),
            }
            .is_coalescible_progress()
        );
    }

    fn coalescible_progress(value: u64) -> TuiTaskProgressEvent {
        TuiTaskProgressEvent::Traversal {
            root: PathBuf::from("/tmp"),
            counter: "files".to_string(),
            value,
            logical_bytes: value,
            files: value,
            directories: 0,
        }
    }

    fn important_progress() -> TuiTaskProgressEvent {
        TuiTaskProgressEvent::BackendMetric {
            metric: "records",
            value: 42,
        }
    }

    fn active_task_fixture(
        id: TuiTaskId,
    ) -> (ActiveTask, SyncSender<TaskMessage>, ScanCancellationToken) {
        let (sender, receiver) = mpsc::sync_channel(TASK_CHANNEL_CAPACITY);
        let cancellation = ScanCancellationToken::new();
        let handle = thread::spawn(|| {});
        (
            ActiveTask {
                id,
                label: "test",
                effect: TuiEffect::Scan(Vec::new()),
                cancellation: cancellation.clone(),
                receiver,
                handle,
            },
            sender,
            cancellation,
        )
    }

    fn runtime_config_fixture(root: PathBuf) -> AppRuntimeConfig {
        AppRuntimeConfig {
            app_paths: AppPaths {
                config_dir: root.join("config"),
                config_file: root.join("config").join("config.toml"),
                state_dir: root.join("state"),
                cache_dir: root.join("cache"),
                history_file: root.join("state").join("history.jsonl"),
            },
            scan_cache_policy: ScanCachePolicy::default(),
            protected_paths: Vec::new(),
            purge: PurgeRuntimeConfig {
                roots: Vec::new(),
                max_depth: 1,
                min_age_days: 0,
            },
        }
    }
}

use std::sync::mpsc::TryRecvError;

use anyhow::Result;
use rebecca_core::config::AppRuntimeConfig;

use crate::runtime::CliRuntime;
use crate::tui::app::TuiApp;
use crate::tui::effect::TuiEffect;
use crate::tui::progress::TuiTaskProgressEvent;
use crate::tui::task_outcome::apply_outcome;
use crate::tui::task_worker::{ActiveTask, TaskMessage, spawn_task};

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

fn apply_pending_progress(app: &mut TuiApp, pending_progress: &mut Option<TuiTaskProgressEvent>) {
    if let Some(event) = pending_progress.take() {
        app.apply_task_progress(event);
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::mpsc::{self, SyncSender};
    use std::thread;

    use super::*;
    use crate::tui::progress::TuiTaskId;
    use crate::tui::task_progress::{TaskSendError, send_progress_message};
    use crate::tui::task_worker::{ActiveTask, TASK_CHANNEL_CAPACITY, TaskMessage};
    use rebecca_core::config::{AppPaths, PurgeRuntimeConfig};
    use rebecca_core::scan::{ScanBackendKind, ScanCancellationToken};
    use rebecca_core::scan_cache::ScanCachePolicy;

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

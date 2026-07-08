use std::sync::mpsc::{SyncSender, TrySendError};

use rebecca::core::RebeccaError;
use rebecca::core::planner::PlanProgressEvent;
use rebecca::core::progress::InspectProgressEvent;

use crate::tui::progress::{TuiTaskId, TuiTaskProgressEvent};
use crate::tui::task_worker::TaskMessage;

pub(super) fn progress_sender(
    task_id: TuiTaskId,
    sender: SyncSender<TaskMessage>,
) -> impl FnMut(TuiTaskProgressEvent) -> rebecca::core::Result<()> {
    move |event| {
        send_progress_message(&sender, TaskMessage::Progress { task_id, event })
            .map_err(|_| tui_receiver_closed())
    }
}

pub(super) fn plan_progress_sender(
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

pub(super) fn send_progress_message(
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

pub(super) fn inspect_progress_event(event: InspectProgressEvent<'_>) -> TuiTaskProgressEvent {
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

fn tui_receiver_closed() -> RebeccaError {
    RebeccaError::OperationCancelled("tui task receiver was closed".to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct TaskSendError;

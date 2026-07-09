use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::plan::{CleanupTarget, CleanupTargetDeletionStyle};
use crate::{DeleteMode, TargetStatus};

const EXECUTION_TARGET_SHADOWED_REASON: &str = "execution-target-shadowed";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionReport {
    #[serde(default)]
    pub dry_run: bool,
    pub summary: ExecutionSummary,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<ExecutionActionReport>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<ExecutionWarning>,
}

impl ExecutionReport {
    pub fn dry_run() -> Self {
        Self {
            dry_run: true,
            ..Self::default()
        }
    }

    pub fn from_targets(targets: &[CleanupTarget]) -> Self {
        let actions = targets
            .iter()
            .enumerate()
            .filter(|(_, target)| target.status != TargetStatus::Allowed)
            .map(|(index, target)| ExecutionActionReport::from_target(index, target))
            .collect::<Vec<_>>();

        Self::from_actions(actions)
    }

    pub fn from_actions(actions: Vec<ExecutionActionReport>) -> Self {
        Self::from_actions_with_dry_run(actions, false)
    }

    pub fn from_actions_with_dry_run(actions: Vec<ExecutionActionReport>, dry_run: bool) -> Self {
        let summary = ExecutionSummary::from_actions(&actions);
        Self {
            dry_run,
            summary,
            actions,
            warnings: Vec::new(),
        }
    }

    pub fn push_warning(&mut self, warning: ExecutionWarning) {
        self.warnings.push(warning);
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionSummary {
    pub total_actions: usize,
    pub completed_actions: usize,
    pub skipped_actions: usize,
    pub blocked_actions: usize,
    pub failed_actions: usize,
    pub estimated_bytes: u64,
    pub confirmed_reclaimed_bytes: u64,
    pub pending_reclaim_bytes: u64,
    pub skipped_bytes: u64,
    pub failed_bytes: u64,
    pub shadowed_bytes: u64,
}

impl ExecutionSummary {
    pub fn from_actions(actions: &[ExecutionActionReport]) -> Self {
        let mut summary = Self {
            total_actions: actions.len(),
            ..Self::default()
        };

        for action in actions {
            summary.estimated_bytes = summary
                .estimated_bytes
                .saturating_add(action.estimated_bytes);
            summary.confirmed_reclaimed_bytes = summary
                .confirmed_reclaimed_bytes
                .saturating_add(action.confirmed_reclaimed_bytes);
            summary.pending_reclaim_bytes = summary
                .pending_reclaim_bytes
                .saturating_add(action.pending_reclaim_bytes);

            match action.status {
                TargetStatus::Completed => summary.completed_actions += 1,
                TargetStatus::Skipped => {
                    summary.skipped_actions += 1;
                    summary.skipped_bytes =
                        summary.skipped_bytes.saturating_add(action.estimated_bytes);
                    if action.reason_code.as_deref() == Some(EXECUTION_TARGET_SHADOWED_REASON) {
                        summary.shadowed_bytes = summary
                            .shadowed_bytes
                            .saturating_add(action.estimated_bytes);
                    }
                }
                TargetStatus::Blocked => {
                    summary.blocked_actions += 1;
                    summary.skipped_bytes =
                        summary.skipped_bytes.saturating_add(action.estimated_bytes);
                }
                TargetStatus::Failed => {
                    summary.failed_actions += 1;
                    summary.failed_bytes =
                        summary.failed_bytes.saturating_add(action.estimated_bytes);
                }
                TargetStatus::Allowed => {}
            }
        }

        summary
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionActionReport {
    pub target_index: usize,
    pub rule_id: String,
    pub path: PathBuf,
    pub deletion_style: CleanupTargetDeletionStyle,
    pub estimated_bytes: u64,
    pub status: TargetStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attempted_paths: Vec<PathBuf>,
    pub confirmed_reclaimed_bytes: u64,
    pub pending_reclaim_bytes: u64,
}

impl ExecutionActionReport {
    pub fn from_target(target_index: usize, target: &CleanupTarget) -> Self {
        let attempted_paths = match target.status {
            TargetStatus::Completed | TargetStatus::Failed => vec![target.path.clone()],
            TargetStatus::Allowed | TargetStatus::Skipped | TargetStatus::Blocked => Vec::new(),
        };

        Self {
            target_index,
            rule_id: target.rule_id.clone(),
            path: target.path.clone(),
            deletion_style: target.deletion_style,
            estimated_bytes: target.estimated_bytes,
            status: target.status,
            reason: target.reason.clone(),
            reason_code: target.reason_code.map(|reason| reason.label().to_string()),
            attempted_paths,
            confirmed_reclaimed_bytes: target.freed_bytes,
            pending_reclaim_bytes: target.pending_reclaim_bytes,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionWarning {
    pub kind: ExecutionWarningKind,
    pub message: String,
}

impl ExecutionWarning {
    pub fn history_write_failed(message: impl Into<String>) -> Self {
        Self {
            kind: ExecutionWarningKind::HistoryWriteFailed,
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExecutionWarningKind {
    HistoryWriteFailed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionProgressTarget {
    pub rule_id: String,
    pub path: PathBuf,
    pub deletion_style: CleanupTargetDeletionStyle,
    pub estimated_bytes: u64,
    pub status: TargetStatus,
    pub freed_bytes: u64,
    pub pending_reclaim_bytes: u64,
    pub reason_code: Option<String>,
    pub reason: Option<String>,
}

impl ExecutionProgressTarget {
    pub fn from_target(target: &CleanupTarget) -> Self {
        Self {
            rule_id: target.rule_id.clone(),
            path: target.path.clone(),
            deletion_style: target.deletion_style,
            estimated_bytes: target.estimated_bytes,
            status: target.status,
            freed_bytes: target.freed_bytes,
            pending_reclaim_bytes: target.pending_reclaim_bytes,
            reason_code: target.reason_code.map(|reason| reason.label().to_string()),
            reason: target.reason.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum ExecutionProgressEvent<'a> {
    Started {
        total_targets: usize,
        executable_targets: usize,
        estimated_bytes: u64,
        mode: DeleteMode,
    },
    TargetStarted {
        target_index: usize,
        target: ExecutionProgressTarget,
    },
    TargetFinished {
        target_index: usize,
        target: ExecutionProgressTarget,
    },
    Completed {
        summary: &'a ExecutionSummary,
    },
}

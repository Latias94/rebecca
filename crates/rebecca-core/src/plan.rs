use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{DeleteMode, PlanRequest, TargetStatus};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanupSummary {
    pub total_targets: usize,
    pub allowed_targets: usize,
    pub skipped_targets: usize,
    pub blocked_targets: usize,
    pub failed_targets: usize,
    pub completed_targets: usize,
    pub estimated_bytes: u64,
    pub freed_bytes: u64,
    pub pending_reclaim_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanupTarget {
    pub rule_id: String,
    pub path: PathBuf,
    pub estimated_bytes: u64,
    pub mode: DeleteMode,
    pub status: TargetStatus,
    pub reason: Option<String>,
    pub restore_hint: Option<String>,
    pub freed_bytes: u64,
    pub pending_reclaim_bytes: u64,
}

impl CleanupTarget {
    pub fn allowed(
        rule_id: impl Into<String>,
        path: PathBuf,
        estimated_bytes: u64,
        mode: DeleteMode,
    ) -> Self {
        Self {
            rule_id: rule_id.into(),
            path,
            estimated_bytes,
            mode,
            status: TargetStatus::Allowed,
            reason: None,
            restore_hint: None,
            freed_bytes: 0,
            pending_reclaim_bytes: 0,
        }
    }

    pub fn skipped(
        rule_id: impl Into<String>,
        path: PathBuf,
        mode: DeleteMode,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            rule_id: rule_id.into(),
            path,
            estimated_bytes: 0,
            mode,
            status: TargetStatus::Skipped,
            reason: Some(reason.into()),
            restore_hint: None,
            freed_bytes: 0,
            pending_reclaim_bytes: 0,
        }
    }

    pub fn blocked(
        rule_id: impl Into<String>,
        path: PathBuf,
        mode: DeleteMode,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            rule_id: rule_id.into(),
            path,
            estimated_bytes: 0,
            mode,
            status: TargetStatus::Blocked,
            reason: Some(reason.into()),
            restore_hint: None,
            freed_bytes: 0,
            pending_reclaim_bytes: 0,
        }
    }

    pub fn failed(
        rule_id: impl Into<String>,
        path: PathBuf,
        mode: DeleteMode,
        estimated_bytes: u64,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            rule_id: rule_id.into(),
            path,
            estimated_bytes,
            mode,
            status: TargetStatus::Failed,
            reason: Some(reason.into()),
            restore_hint: None,
            freed_bytes: 0,
            pending_reclaim_bytes: 0,
        }
    }

    pub fn with_restore_hint(mut self, restore_hint: Option<String>) -> Self {
        self.restore_hint = restore_hint;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanupPlan {
    pub request: PlanRequest,
    pub summary: CleanupSummary,
    pub targets: Vec<CleanupTarget>,
}

impl CleanupPlan {
    pub fn new(mode: DeleteMode) -> Self {
        Self::empty(PlanRequest::new(mode))
    }

    pub fn empty(request: PlanRequest) -> Self {
        Self {
            request,
            summary: CleanupSummary::default(),
            targets: Vec::new(),
        }
    }

    pub fn recompute_summary(&mut self) {
        let mut summary = CleanupSummary::default();

        for target in &self.targets {
            summary.total_targets += 1;
            summary.estimated_bytes = summary
                .estimated_bytes
                .saturating_add(target.estimated_bytes);
            summary.freed_bytes = summary.freed_bytes.saturating_add(target.freed_bytes);
            summary.pending_reclaim_bytes = summary
                .pending_reclaim_bytes
                .saturating_add(target.pending_reclaim_bytes);

            match target.status {
                TargetStatus::Allowed => summary.allowed_targets += 1,
                TargetStatus::Skipped => summary.skipped_targets += 1,
                TargetStatus::Blocked => summary.blocked_targets += 1,
                TargetStatus::Failed => summary.failed_targets += 1,
                TargetStatus::Completed => summary.completed_targets += 1,
            }
        }

        self.summary = summary;
    }
}

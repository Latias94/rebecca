use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::project_artifacts::ProjectArtifactContextMatch;
use crate::{DeleteMode, PlanRequest, TargetStatus};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CleanupTargetDeletionStyle {
    #[default]
    PreserveRootContents,
    DeleteWholePath,
}

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
    #[serde(default)]
    pub issue_matrix: Vec<CleanupIssueSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanupIssueSummary {
    pub status: TargetStatus,
    pub reason_code: CleanupTargetIssueReason,
    pub targets: usize,
    pub estimated_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanupTarget {
    pub rule_id: String,
    pub path: PathBuf,
    pub estimated_bytes: u64,
    pub mode: DeleteMode,
    pub status: TargetStatus,
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<CleanupTargetIssueReason>,
    pub restore_hint: Option<String>,
    #[serde(default)]
    pub deletion_style: CleanupTargetDeletionStyle,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified_at_unix_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_artifact: Option<ProjectArtifactContextMatch>,
    pub freed_bytes: u64,
    pub pending_reclaim_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CleanupTargetIssueReason {
    SafetyOptInRequired,
    TargetDiscoverySkipped,
    TargetDiscoveryFailed,
    DuplicateTargetPath,
    SafetyPolicySkipped,
    ExecutionTargetMissing,
    SafetyPolicyBlocked,
    ProjectArtifactRecentlyModified,
    ScanFailed,
    ExecutionFailed,
    Unclassified,
}

impl CleanupTargetIssueReason {
    pub fn label(self) -> &'static str {
        match self {
            Self::SafetyOptInRequired => "safety-opt-in-required",
            Self::TargetDiscoverySkipped => "target-discovery-skipped",
            Self::TargetDiscoveryFailed => "target-discovery-failed",
            Self::DuplicateTargetPath => "duplicate-target-path",
            Self::SafetyPolicySkipped => "safety-policy-skipped",
            Self::ExecutionTargetMissing => "execution-target-missing",
            Self::SafetyPolicyBlocked => "safety-policy-blocked",
            Self::ProjectArtifactRecentlyModified => "project-artifact-recently-modified",
            Self::ScanFailed => "scan-failed",
            Self::ExecutionFailed => "execution-failed",
            Self::Unclassified => "unclassified",
        }
    }
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
            reason_code: None,
            restore_hint: None,
            deletion_style: CleanupTargetDeletionStyle::default(),
            modified_at_unix_seconds: None,
            project_artifact: None,
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
        Self::skipped_with_reason_code(
            rule_id,
            path,
            mode,
            CleanupTargetIssueReason::Unclassified,
            reason,
        )
    }

    pub fn skipped_with_reason_code(
        rule_id: impl Into<String>,
        path: PathBuf,
        mode: DeleteMode,
        reason_code: CleanupTargetIssueReason,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            rule_id: rule_id.into(),
            path,
            estimated_bytes: 0,
            mode,
            status: TargetStatus::Skipped,
            reason: Some(reason.into()),
            reason_code: Some(reason_code),
            restore_hint: None,
            deletion_style: CleanupTargetDeletionStyle::default(),
            modified_at_unix_seconds: None,
            project_artifact: None,
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
        Self::blocked_with_reason_code(
            rule_id,
            path,
            mode,
            CleanupTargetIssueReason::Unclassified,
            reason,
        )
    }

    pub fn blocked_with_reason_code(
        rule_id: impl Into<String>,
        path: PathBuf,
        mode: DeleteMode,
        reason_code: CleanupTargetIssueReason,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            rule_id: rule_id.into(),
            path,
            estimated_bytes: 0,
            mode,
            status: TargetStatus::Blocked,
            reason: Some(reason.into()),
            reason_code: Some(reason_code),
            restore_hint: None,
            deletion_style: CleanupTargetDeletionStyle::default(),
            modified_at_unix_seconds: None,
            project_artifact: None,
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
        Self::failed_with_reason_code(
            rule_id,
            path,
            mode,
            estimated_bytes,
            CleanupTargetIssueReason::Unclassified,
            reason,
        )
    }

    pub fn failed_with_reason_code(
        rule_id: impl Into<String>,
        path: PathBuf,
        mode: DeleteMode,
        estimated_bytes: u64,
        reason_code: CleanupTargetIssueReason,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            rule_id: rule_id.into(),
            path,
            estimated_bytes,
            mode,
            status: TargetStatus::Failed,
            reason: Some(reason.into()),
            reason_code: Some(reason_code),
            restore_hint: None,
            deletion_style: CleanupTargetDeletionStyle::default(),
            modified_at_unix_seconds: None,
            project_artifact: None,
            freed_bytes: 0,
            pending_reclaim_bytes: 0,
        }
    }

    pub fn with_deletion_style(mut self, deletion_style: CleanupTargetDeletionStyle) -> Self {
        self.deletion_style = deletion_style;
        self
    }

    pub fn with_modified_at_unix_seconds(mut self, modified_at_unix_seconds: Option<u64>) -> Self {
        self.modified_at_unix_seconds = modified_at_unix_seconds;
        self
    }

    pub fn with_project_artifact_context(
        mut self,
        context: Option<ProjectArtifactContextMatch>,
    ) -> Self {
        self.project_artifact = context;
        self
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
    pub fn empty(request: PlanRequest) -> Self {
        Self {
            request,
            summary: CleanupSummary::default(),
            targets: Vec::new(),
        }
    }

    pub fn recompute_summary(&mut self) {
        let mut summary = CleanupSummary::default();
        let mut issue_matrix = BTreeMap::new();

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

            if target.status.is_issue() {
                if let Some(reason_code) = target.reason_code {
                    let bucket = issue_matrix
                        .entry((target.status, reason_code))
                        .or_insert_with(|| CleanupIssueSummary {
                            status: target.status,
                            reason_code,
                            targets: 0,
                            estimated_bytes: 0,
                        });
                    bucket.targets = bucket.targets.saturating_add(1);
                    bucket.estimated_bytes = bucket
                        .estimated_bytes
                        .saturating_add(target.estimated_bytes);
                }
            }
        }

        summary.issue_matrix = issue_matrix.into_values().collect();
        self.summary = summary;
    }
}

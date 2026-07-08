use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::project_artifacts::{ProjectArtifactContextMatch, ProjectArtifactDiscoveryDiagnostic};
use crate::scan::{
    MeasuredScan, ScanBackendEvidence, ScanBackendKind, ScanEstimateCaveat, ScanEstimateConfidence,
};
use crate::warnings::WarningSummary;
use crate::{DeleteMode, PlanRequest, TargetStatus};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CleanupTargetDeletionStyle {
    #[default]
    PreserveRootContents,
    DeleteWholePath,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EstimateSource {
    #[default]
    Unknown,
    FreshScan,
    ScanCache,
    NotMeasured,
}

impl EstimateSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::FreshScan => "fresh-scan",
            Self::ScanCache => "scan-cache",
            Self::NotMeasured => "not-measured",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EstimateProvenance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimate_backend: Option<ScanBackendKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimate_backend_source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimate_confidence: Option<ScanEstimateConfidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimate_fallback_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub estimate_caveats: Vec<ScanEstimateCaveat>,
    #[serde(default, skip_serializing_if = "ScanBackendEvidence::is_empty")]
    pub estimate_backend_evidence: ScanBackendEvidence,
}

impl EstimateProvenance {
    pub fn from_measured_scan(scan: &MeasuredScan) -> Self {
        Self {
            estimate_backend: Some(scan.backend),
            estimate_backend_source: scan.backend_source.clone(),
            estimate_confidence: Some(scan.confidence),
            estimate_fallback_reason: scan.fallback_reason.clone(),
            estimate_caveats: scan.caveats.clone(),
            estimate_backend_evidence: scan.backend_evidence.clone(),
        }
    }

    pub fn from_backend_confidence(
        backend: ScanBackendKind,
        confidence: ScanEstimateConfidence,
    ) -> Self {
        Self::from_backend_confidence_and_source(backend, confidence, None)
    }

    pub fn from_backend_confidence_and_source(
        backend: ScanBackendKind,
        confidence: ScanEstimateConfidence,
        source: Option<String>,
    ) -> Self {
        Self {
            estimate_backend: Some(backend),
            estimate_backend_source: source,
            estimate_confidence: Some(confidence),
            estimate_fallback_reason: None,
            estimate_caveats: Vec::new(),
            estimate_backend_evidence: ScanBackendEvidence::default(),
        }
    }

    pub fn with_backend_evidence(mut self, evidence: ScanBackendEvidence) -> Self {
        self.estimate_backend_evidence.merge(evidence);
        self
    }

    pub fn is_empty(&self) -> bool {
        self.estimate_backend.is_none()
            && self.estimate_backend_source.is_none()
            && self.estimate_confidence.is_none()
            && self.estimate_fallback_reason.is_none()
            && self.estimate_caveats.is_empty()
            && self.estimate_backend_evidence.is_empty()
    }

    pub fn has_human_visible_detail(&self, estimate_source: EstimateSource) -> bool {
        matches!(
            estimate_source,
            EstimateSource::Unknown | EstimateSource::ScanCache
        ) || self.estimate_backend.is_some_and(|backend| {
            backend != ScanBackendKind::PortableRecursive
                || !self.estimate_caveats.is_empty()
                || self.estimate_fallback_reason.is_some()
                || self.estimate_backend_source.is_some()
                || !self.estimate_backend_evidence.is_empty()
        }) || self.estimate_fallback_reason.is_some()
            || self.estimate_backend_source.is_some()
            || !self.estimate_caveats.is_empty()
            || !self.estimate_backend_evidence.is_empty()
    }
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
    #[serde(default)]
    pub warning_matrix: Vec<WarningSummary>,
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
    #[serde(default)]
    pub estimate_source: EstimateSource,
    #[serde(default, flatten)]
    pub estimate_provenance: EstimateProvenance,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<CleanupTargetEvidence>,
    pub freed_bytes: u64,
    pub pending_reclaim_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanupTargetEvidence {
    pub kind: CleanupTargetEvidenceKind,
    pub status: TargetStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<CleanupTargetIssueReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl CleanupTargetEvidence {
    pub fn issue(
        status: TargetStatus,
        reason_code: CleanupTargetIssueReason,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            kind: CleanupTargetEvidenceKind::Issue,
            status,
            reason_code: Some(reason_code),
            warning: None,
            reason: Some(reason.into()),
        }
    }

    pub fn warning(status: TargetStatus, warning: impl Into<String>) -> Self {
        Self {
            kind: CleanupTargetEvidenceKind::Warning,
            status,
            reason_code: None,
            warning: Some(warning.into()),
            reason: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CleanupTargetEvidenceKind {
    Issue,
    Warning,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CleanupTargetIssueReason {
    SafetyOptInRequired,
    WarningGateRequired,
    TargetDiscoverySkipped,
    TargetDiscoveryFailed,
    DuplicateTargetPath,
    SafetyPolicySkipped,
    ExecutionTargetMissing,
    ExecutionTargetShadowed,
    SafetyPolicyBlocked,
    ProjectArtifactRecentlyModified,
    ReclaimLimitSatisfied,
    ScanFailed,
    ScanPermissionDenied,
    ExecutionFailed,
    ExecutionPermissionDenied,
    Unclassified,
}

impl CleanupTargetIssueReason {
    pub fn label(self) -> &'static str {
        match self {
            Self::SafetyOptInRequired => "safety-opt-in-required",
            Self::WarningGateRequired => "warning-gate-required",
            Self::TargetDiscoverySkipped => "target-discovery-skipped",
            Self::TargetDiscoveryFailed => "target-discovery-failed",
            Self::DuplicateTargetPath => "duplicate-target-path",
            Self::SafetyPolicySkipped => "safety-policy-skipped",
            Self::ExecutionTargetMissing => "execution-target-missing",
            Self::ExecutionTargetShadowed => "execution-target-shadowed",
            Self::SafetyPolicyBlocked => "safety-policy-blocked",
            Self::ProjectArtifactRecentlyModified => "project-artifact-recently-modified",
            Self::ReclaimLimitSatisfied => "reclaim-limit-satisfied",
            Self::ScanFailed => "scan-failed",
            Self::ScanPermissionDenied => "scan-permission-denied",
            Self::ExecutionFailed => "execution-failed",
            Self::ExecutionPermissionDenied => "execution-permission-denied",
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
            estimate_source: EstimateSource::FreshScan,
            estimate_provenance: EstimateProvenance::default(),
            mode,
            status: TargetStatus::Allowed,
            reason: None,
            reason_code: None,
            restore_hint: None,
            deletion_style: CleanupTargetDeletionStyle::default(),
            modified_at_unix_seconds: None,
            project_artifact: None,
            warnings: Vec::new(),
            evidence: Vec::new(),
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
        let (reason, issue_evidence) =
            Self::issue_reason_and_evidence(TargetStatus::Skipped, reason_code, reason);
        Self {
            rule_id: rule_id.into(),
            path,
            estimated_bytes: 0,
            estimate_source: EstimateSource::NotMeasured,
            estimate_provenance: EstimateProvenance::default(),
            mode,
            status: TargetStatus::Skipped,
            reason: Some(reason.clone()),
            reason_code: Some(reason_code),
            restore_hint: None,
            deletion_style: CleanupTargetDeletionStyle::default(),
            modified_at_unix_seconds: None,
            project_artifact: None,
            warnings: Vec::new(),
            evidence: vec![issue_evidence],
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
        let (reason, issue_evidence) =
            Self::issue_reason_and_evidence(TargetStatus::Blocked, reason_code, reason);
        Self {
            rule_id: rule_id.into(),
            path,
            estimated_bytes: 0,
            estimate_source: EstimateSource::NotMeasured,
            estimate_provenance: EstimateProvenance::default(),
            mode,
            status: TargetStatus::Blocked,
            reason: Some(reason.clone()),
            reason_code: Some(reason_code),
            restore_hint: None,
            deletion_style: CleanupTargetDeletionStyle::default(),
            modified_at_unix_seconds: None,
            project_artifact: None,
            warnings: Vec::new(),
            evidence: vec![issue_evidence],
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
        let (reason, issue_evidence) =
            Self::issue_reason_and_evidence(TargetStatus::Failed, reason_code, reason);
        Self {
            rule_id: rule_id.into(),
            path,
            estimated_bytes,
            estimate_source: if estimated_bytes == 0 {
                EstimateSource::NotMeasured
            } else {
                EstimateSource::FreshScan
            },
            estimate_provenance: EstimateProvenance::default(),
            mode,
            status: TargetStatus::Failed,
            reason: Some(reason.clone()),
            reason_code: Some(reason_code),
            restore_hint: None,
            deletion_style: CleanupTargetDeletionStyle::default(),
            modified_at_unix_seconds: None,
            project_artifact: None,
            warnings: Vec::new(),
            evidence: vec![issue_evidence],
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

    pub fn with_estimate_source(mut self, estimate_source: EstimateSource) -> Self {
        self.estimate_source = estimate_source;
        self
    }

    pub fn with_estimate_provenance(mut self, estimate_provenance: EstimateProvenance) -> Self {
        self.estimate_provenance = estimate_provenance;
        self
    }

    pub fn with_restore_hint(mut self, restore_hint: Option<String>) -> Self {
        self.restore_hint = restore_hint;
        self
    }

    pub fn with_warnings(mut self, warnings: Vec<String>) -> Self {
        self.evidence
            .retain(|evidence| evidence.kind != CleanupTargetEvidenceKind::Warning);
        self.evidence.extend(
            warnings
                .iter()
                .cloned()
                .map(|warning| CleanupTargetEvidence::warning(self.status, warning)),
        );
        self.warnings = warnings;
        self
    }

    pub fn mark_completed(
        &mut self,
        freed_bytes: u64,
        pending_reclaim_bytes: u64,
        reason: Option<String>,
    ) {
        self.status = TargetStatus::Completed;
        self.reason = reason;
        self.reason_code = None;
        self.freed_bytes = freed_bytes;
        self.pending_reclaim_bytes = pending_reclaim_bytes;
        self.remove_issue_evidence_and_sync_warnings();
    }

    pub fn mark_skipped_with_reason_code(
        &mut self,
        reason_code: CleanupTargetIssueReason,
        reason: impl Into<String>,
    ) {
        self.mark_issue_state(TargetStatus::Skipped, reason_code, reason);
    }

    pub fn mark_blocked_with_reason_code(
        &mut self,
        reason_code: CleanupTargetIssueReason,
        reason: impl Into<String>,
    ) {
        self.mark_issue_state(TargetStatus::Blocked, reason_code, reason);
    }

    pub fn mark_failed_with_reason_code(
        &mut self,
        reason_code: CleanupTargetIssueReason,
        reason: impl Into<String>,
    ) {
        self.mark_issue_state(TargetStatus::Failed, reason_code, reason);
    }

    fn mark_issue_state(
        &mut self,
        status: TargetStatus,
        reason_code: CleanupTargetIssueReason,
        reason: impl Into<String>,
    ) {
        let (reason, issue_evidence) = Self::issue_reason_and_evidence(status, reason_code, reason);
        self.status = status;
        self.reason = Some(reason);
        self.reason_code = Some(reason_code);
        self.freed_bytes = 0;
        self.pending_reclaim_bytes = 0;
        self.remove_issue_evidence_and_sync_warnings();
        self.evidence.push(issue_evidence);
    }

    fn issue_reason_and_evidence(
        status: TargetStatus,
        reason_code: CleanupTargetIssueReason,
        reason: impl Into<String>,
    ) -> (String, CleanupTargetEvidence) {
        debug_assert!(status.is_issue());
        let reason = reason.into();
        let issue_evidence = CleanupTargetEvidence::issue(status, reason_code, reason.clone());
        (reason, issue_evidence)
    }

    fn remove_issue_evidence_and_sync_warnings(&mut self) {
        let status = self.status;
        self.evidence.retain_mut(|evidence| match evidence.kind {
            CleanupTargetEvidenceKind::Issue => false,
            CleanupTargetEvidenceKind::Warning => {
                evidence.status = status;
                true
            }
        });
    }

    fn for_each_issue_evidence(
        &self,
        mut visit: impl FnMut(TargetStatus, CleanupTargetIssueReason),
    ) {
        let mut emitted = false;
        for evidence in &self.evidence {
            if evidence.kind == CleanupTargetEvidenceKind::Issue
                && evidence.status.is_issue()
                && let Some(reason_code) = evidence.reason_code
            {
                visit(evidence.status, reason_code);
                emitted = true;
            }
        }

        if !emitted
            && self.status.is_issue()
            && let Some(reason_code) = self.reason_code
        {
            visit(self.status, reason_code);
        }
    }

    fn for_each_warning_evidence(&self, mut visit: impl FnMut(&str)) {
        let mut emitted = false;
        for evidence in &self.evidence {
            if evidence.kind == CleanupTargetEvidenceKind::Warning
                && let Some(warning) = evidence.warning.as_deref()
            {
                visit(warning);
                emitted = true;
            }
        }

        if !emitted {
            for warning in &self.warnings {
                visit(warning);
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanupPlan {
    pub request: PlanRequest,
    pub summary: CleanupSummary,
    pub targets: Vec<CleanupTarget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_report: Option<crate::execution::ExecutionReport>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub discovery_diagnostics: Vec<ProjectArtifactDiscoveryDiagnostic>,
}

impl CleanupPlan {
    pub fn empty(request: PlanRequest) -> Self {
        Self {
            request,
            summary: CleanupSummary::default(),
            targets: Vec::new(),
            execution_report: None,
            discovery_diagnostics: Vec::new(),
        }
    }

    pub fn recompute_summary(&mut self) {
        let mut summary = CleanupSummary::default();
        let mut issue_matrix = BTreeMap::new();
        let mut warning_matrix = BTreeMap::new();

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

            target.for_each_issue_evidence(|status, reason_code| {
                let bucket = issue_matrix
                    .entry((status, reason_code))
                    .or_insert_with(|| CleanupIssueSummary {
                        status,
                        reason_code,
                        targets: 0,
                        estimated_bytes: 0,
                    });
                bucket.targets = bucket.targets.saturating_add(1);
                bucket.estimated_bytes = bucket
                    .estimated_bytes
                    .saturating_add(target.estimated_bytes);
            });

            target.for_each_warning_evidence(|warning| {
                let bucket =
                    warning_matrix
                        .entry(warning.to_owned())
                        .or_insert_with(|| WarningSummary {
                            warning: warning.to_owned(),
                            targets: 0,
                            estimated_bytes: 0,
                        });
                bucket.targets = bucket.targets.saturating_add(1);
                bucket.estimated_bytes = bucket
                    .estimated_bytes
                    .saturating_add(target.estimated_bytes);
            });
        }

        summary.issue_matrix = issue_matrix.into_values().collect();
        summary.warning_matrix = warning_matrix.into_values().collect();
        self.summary = summary;
    }
}

use std::path::Path;

use rebecca::core::plan::{CleanupPlan, CleanupSummary, CleanupTarget};
use rebecca::core::{DeleteMode, EstimateSource, TargetStatus};

use crate::text::format_count;

const LARGEST_TARGET_LIMIT: usize = 5;

#[derive(Debug, Clone)]
pub(crate) struct CleanPlanProjection<'a> {
    pub(crate) workflow_title: &'static str,
    pub(crate) mode_label: &'static str,
    pub(crate) summary: CleanPlanSummary,
    issue_matrix: Vec<CleanIssueRow>,
    scan_cache_summary: Option<ScanCacheSummaryRow>,
    largest_targets: Vec<CleanTargetRow<'a>>,
    target_groups: Vec<CleanTargetGroup<'a>>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct CleanPlanSummary {
    pub(crate) total_targets: usize,
    pub(crate) allowed_targets: usize,
    pub(crate) skipped_targets: usize,
    pub(crate) blocked_targets: usize,
    pub(crate) failed_targets: usize,
    pub(crate) completed_targets: usize,
    pub(crate) estimated_bytes: u64,
    pub(crate) freed_bytes: u64,
    pub(crate) pending_reclaim_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CleanIssueRow {
    pub(crate) status_label: &'static str,
    pub(crate) reason_label: &'static str,
    pub(crate) targets_label: String,
    pub(crate) estimated_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScanCacheSummaryRow {
    pub(crate) hits_label: String,
    pub(crate) misses_label: String,
    pub(crate) write_skipped_label: String,
    pub(crate) pruned_label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CleanTargetGroup<'a> {
    pub(crate) status_label: &'static str,
    pub(crate) targets: Vec<CleanTargetRow<'a>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CleanTargetRow<'a> {
    pub(crate) rule_id: &'a str,
    pub(crate) path: &'a Path,
    pub(crate) estimated_bytes: u64,
    pub(crate) estimate_source: EstimateSource,
    pub(crate) reason: Option<&'a str>,
    pub(crate) restore_hint: Option<&'a str>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ScanCacheProgressSummary {
    pub(crate) hits: u64,
    pub(crate) misses: u64,
    pub(crate) write_skipped: u64,
    pub(crate) pruned: u64,
}

impl<'a> CleanPlanProjection<'a> {
    pub(crate) fn new(
        plan: &'a CleanupPlan,
        scan_cache_summary: Option<ScanCacheProgressSummary>,
    ) -> Self {
        Self {
            workflow_title: plan.request.workflow.title(),
            mode_label: cleanup_mode_label(plan.request.mode),
            summary: CleanPlanSummary::from(&plan.summary),
            issue_matrix: issue_matrix_rows(plan),
            scan_cache_summary: scan_cache_summary
                .filter(|summary| summary.has_activity())
                .map(ScanCacheSummaryRow::from),
            largest_targets: largest_target_rows(plan),
            target_groups: target_groups(plan),
        }
    }

    pub(crate) fn issue_matrix(&self) -> &[CleanIssueRow] {
        &self.issue_matrix
    }

    pub(crate) fn scan_cache_summary(&self) -> Option<&ScanCacheSummaryRow> {
        self.scan_cache_summary.as_ref()
    }

    pub(crate) fn largest_targets(&self) -> &[CleanTargetRow<'a>] {
        &self.largest_targets
    }

    pub(crate) fn target_groups(&self) -> &[CleanTargetGroup<'a>] {
        &self.target_groups
    }
}

impl From<&CleanupSummary> for CleanPlanSummary {
    fn from(summary: &CleanupSummary) -> Self {
        Self {
            total_targets: summary.total_targets,
            allowed_targets: summary.allowed_targets,
            skipped_targets: summary.skipped_targets,
            blocked_targets: summary.blocked_targets,
            failed_targets: summary.failed_targets,
            completed_targets: summary.completed_targets,
            estimated_bytes: summary.estimated_bytes,
            freed_bytes: summary.freed_bytes,
            pending_reclaim_bytes: summary.pending_reclaim_bytes,
        }
    }
}

impl ScanCacheProgressSummary {
    fn has_activity(self) -> bool {
        self.hits > 0 || self.misses > 0 || self.write_skipped > 0 || self.pruned > 0
    }
}

impl From<ScanCacheProgressSummary> for ScanCacheSummaryRow {
    fn from(summary: ScanCacheProgressSummary) -> Self {
        Self {
            hits_label: format_count(summary.hits, "hit", "hits"),
            misses_label: format_count(summary.misses, "miss", "misses"),
            write_skipped_label: format_count(
                summary.write_skipped,
                "skipped write",
                "skipped writes",
            ),
            pruned_label: format_count(summary.pruned, "pruned record", "pruned records"),
        }
    }
}

impl<'a> From<&'a CleanupTarget> for CleanTargetRow<'a> {
    fn from(target: &'a CleanupTarget) -> Self {
        Self {
            rule_id: &target.rule_id,
            path: target.path.as_path(),
            estimated_bytes: target.estimated_bytes,
            estimate_source: target.estimate_source,
            reason: target.reason.as_deref(),
            restore_hint: target.restore_hint.as_deref(),
        }
    }
}

fn issue_matrix_rows(plan: &CleanupPlan) -> Vec<CleanIssueRow> {
    plan.summary
        .issue_matrix
        .iter()
        .map(|issue| CleanIssueRow {
            status_label: issue.status.label(),
            reason_label: issue.reason_code.label(),
            targets_label: format_count(issue.targets as u64, "target", "targets"),
            estimated_bytes: issue.estimated_bytes,
        })
        .collect()
}

fn largest_target_rows(plan: &CleanupPlan) -> Vec<CleanTargetRow<'_>> {
    let mut targets = plan
        .targets
        .iter()
        .filter(|target| target.estimated_bytes > 0)
        .collect::<Vec<_>>();

    targets.sort_by(|left, right| {
        right
            .estimated_bytes
            .cmp(&left.estimated_bytes)
            .then_with(|| left.rule_id.cmp(&right.rule_id))
            .then_with(|| left.path.cmp(&right.path))
    });

    targets
        .into_iter()
        .take(LARGEST_TARGET_LIMIT)
        .map(CleanTargetRow::from)
        .collect()
}

fn target_groups(plan: &CleanupPlan) -> Vec<CleanTargetGroup<'_>> {
    [
        TargetStatus::Allowed,
        TargetStatus::Completed,
        TargetStatus::Failed,
        TargetStatus::Blocked,
        TargetStatus::Skipped,
    ]
    .into_iter()
    .filter_map(|status| {
        let targets = plan
            .targets
            .iter()
            .filter(|target| target.status == status)
            .map(CleanTargetRow::from)
            .collect::<Vec<_>>();

        (!targets.is_empty()).then_some(CleanTargetGroup {
            status_label: status.label(),
            targets,
        })
    })
    .collect()
}

fn cleanup_mode_label(mode: DeleteMode) -> &'static str {
    match mode {
        DeleteMode::DryRun => "dry-run",
        DeleteMode::RecycleBin => "recycle-bin",
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use rebecca::core::plan::{CleanupTarget, CleanupTargetIssueReason};
    use rebecca::core::{PlanRequest, Platform};

    use super::*;

    fn plan_with_targets(targets: Vec<CleanupTarget>) -> CleanupPlan {
        let mut plan = CleanupPlan {
            request: PlanRequest::for_platform(Platform::Windows, DeleteMode::DryRun),
            summary: CleanupSummary::default(),
            targets,
            discovery_diagnostics: Vec::new(),
        };
        plan.recompute_summary();
        plan
    }

    fn target(rule_id: &str, estimated_bytes: u64, status: TargetStatus) -> CleanupTarget {
        let path = PathBuf::from(format!("cache/{rule_id}-{estimated_bytes}"));
        let mut target = match status {
            TargetStatus::Allowed => {
                CleanupTarget::allowed(rule_id, path, estimated_bytes, DeleteMode::DryRun)
            }
            TargetStatus::Skipped => CleanupTarget::skipped_with_reason_code(
                rule_id,
                path,
                DeleteMode::DryRun,
                CleanupTargetIssueReason::DuplicateTargetPath,
                "duplicate target path already covered",
            ),
            TargetStatus::Blocked => CleanupTarget::blocked_with_reason_code(
                rule_id,
                path,
                DeleteMode::DryRun,
                CleanupTargetIssueReason::SafetyPolicyBlocked,
                "blocked by safety policy",
            ),
            TargetStatus::Failed => CleanupTarget::failed_with_reason_code(
                rule_id,
                path,
                DeleteMode::DryRun,
                estimated_bytes,
                CleanupTargetIssueReason::ScanFailed,
                "scan failed",
            ),
            TargetStatus::Completed => {
                CleanupTarget::allowed(rule_id, path, estimated_bytes, DeleteMode::DryRun)
            }
        };

        if status == TargetStatus::Completed {
            target.status = TargetStatus::Completed;
            target.freed_bytes = estimated_bytes;
        }

        target
    }

    #[test]
    fn projection_orders_and_limits_largest_targets() {
        let plan = plan_with_targets(vec![
            target("rule-f", 1, TargetStatus::Allowed),
            target("rule-a", 100, TargetStatus::Allowed),
            target("rule-c", 30, TargetStatus::Allowed),
            target("rule-b", 100, TargetStatus::Allowed),
            target("rule-e", 10, TargetStatus::Allowed),
            target("rule-d", 20, TargetStatus::Allowed),
        ]);

        let projection = CleanPlanProjection::new(&plan, None);
        let rule_ids = projection
            .largest_targets()
            .iter()
            .map(|target| target.rule_id)
            .collect::<Vec<_>>();

        assert_eq!(rule_ids, ["rule-a", "rule-b", "rule-c", "rule-d", "rule-e"]);
    }

    #[test]
    fn projection_groups_targets_in_display_order() {
        let plan = plan_with_targets(vec![
            target("skipped", 0, TargetStatus::Skipped),
            target("allowed", 1, TargetStatus::Allowed),
            target("blocked", 0, TargetStatus::Blocked),
            target("failed", 2, TargetStatus::Failed),
            target("completed", 3, TargetStatus::Completed),
        ]);

        let projection = CleanPlanProjection::new(&plan, None);
        let groups = projection
            .target_groups()
            .iter()
            .map(|group| (group.status_label, group.targets[0].rule_id))
            .collect::<Vec<_>>();

        assert_eq!(
            groups,
            [
                ("allowed", "allowed"),
                ("completed", "completed"),
                ("failed", "failed"),
                ("blocked", "blocked"),
                ("skipped", "skipped"),
            ]
        );
    }

    #[test]
    fn projection_prepares_issue_matrix_rows() {
        let plan = plan_with_targets(vec![target("skipped", 0, TargetStatus::Skipped)]);

        let projection = CleanPlanProjection::new(&plan, None);
        let issue = projection
            .issue_matrix()
            .first()
            .expect("skipped target should produce an issue row");

        assert_eq!(issue.status_label, "skipped");
        assert_eq!(issue.reason_label, "duplicate-target-path");
        assert_eq!(issue.targets_label, "1 target");
        assert_eq!(issue.estimated_bytes, 0);
    }

    #[test]
    fn projection_formats_active_scan_cache_summary() {
        let plan = plan_with_targets(Vec::new());
        let projection = CleanPlanProjection::new(
            &plan,
            Some(ScanCacheProgressSummary {
                hits: 1,
                misses: 2,
                write_skipped: 1,
                pruned: 3,
            }),
        );

        let summary = projection
            .scan_cache_summary()
            .expect("active summary should be visible");
        assert_eq!(summary.hits_label, "1 hit");
        assert_eq!(summary.misses_label, "2 misses");
        assert_eq!(summary.write_skipped_label, "1 skipped write");
        assert_eq!(summary.pruned_label, "3 pruned records");
    }

    #[test]
    fn projection_omits_inactive_scan_cache_summary() {
        let plan = plan_with_targets(Vec::new());
        let projection = CleanPlanProjection::new(&plan, Some(ScanCacheProgressSummary::default()));

        assert!(projection.scan_cache_summary().is_none());
    }
}

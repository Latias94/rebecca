use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rebecca::core::plan::{CleanupPlan, CleanupSummary, CleanupTarget, CleanupTargetIssueReason};
use rebecca::core::project_artifacts::{
    ProjectArtifactDiscoveryDiagnostic, ProjectArtifactPolicy, all_project_artifact_policies,
};
use rebecca::core::{EstimateSource, TargetStatus};
use serde::Serialize;

use crate::text::format_count;

const LARGEST_ARTIFACT_LIMIT: usize = 5;
const INSIGHT_TOP_TARGET_LIMIT: usize = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProjectArtifactPlanProjection<'a> {
    artifact_summaries: Vec<ProjectArtifactSummaryRow>,
    largest_targets: Vec<ProjectArtifactRow<'a>>,
    project_groups: Vec<ProjectArtifactGroup<'a>>,
    recently_modified: Vec<ProjectArtifactRow<'a>>,
    discovery_diagnostics: Vec<ProjectArtifactDiscoveryDiagnosticRow<'a>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProjectArtifactSummaryRow {
    pub(crate) artifact_type: String,
    pub(crate) targets_label: String,
    pub(crate) estimated_bytes: u64,
    pub(crate) status_summary_label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProjectArtifactGroup<'a> {
    pub(crate) project_path: PathBuf,
    pub(crate) targets_label: String,
    pub(crate) estimated_bytes: u64,
    pub(crate) targets: Vec<ProjectArtifactRow<'a>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProjectArtifactRow<'a> {
    pub(crate) artifact_type: String,
    pub(crate) status: TargetStatus,
    pub(crate) status_label: &'static str,
    pub(crate) path: &'a Path,
    pub(crate) estimated_bytes: u64,
    pub(crate) estimate_source: EstimateSource,
    pub(crate) modified_at_unix_seconds: Option<u64>,
    pub(crate) reason: Option<&'a str>,
    pub(crate) restore_hint: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProjectArtifactDiscoveryDiagnosticRow<'a> {
    pub(crate) kind_label: &'static str,
    pub(crate) path: &'a Path,
    pub(crate) detail: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ProjectArtifactCatalogEntry {
    pub(crate) artifact: &'static str,
    pub(crate) aliases: Vec<&'static str>,
    pub(crate) rule_id: &'static str,
    pub(crate) rule_suffix: &'static str,
    pub(crate) restore_hint: &'static str,
    pub(crate) default_min_age_days: u64,
    pub(crate) trim_eligible: bool,
    pub(crate) deletion_style: &'static str,
    pub(crate) ranking: &'static str,
    #[serde(skip)]
    pub(crate) selectors_label: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct ProjectArtifactInsightReport {
    pub(crate) roots: Vec<PathBuf>,
    pub(crate) summary: CleanupSummary,
    pub(crate) totals_by_root: Vec<ProjectArtifactInsightTotal>,
    pub(crate) totals_by_project: Vec<ProjectArtifactInsightTotal>,
    pub(crate) totals_by_artifact: Vec<ProjectArtifactInsightTotal>,
    pub(crate) top_targets: Vec<ProjectArtifactInsightTarget>,
    pub(crate) discovery_diagnostics: Vec<ProjectArtifactDiscoveryDiagnostic>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ProjectArtifactInsightTotal {
    pub(crate) key: String,
    pub(crate) label: String,
    pub(crate) targets: usize,
    pub(crate) estimated_bytes: u64,
}

#[derive(Debug, Serialize)]
pub(crate) struct ProjectArtifactInsightTarget {
    pub(crate) rule_id: String,
    pub(crate) artifact: String,
    pub(crate) path: PathBuf,
    pub(crate) project_root: Option<PathBuf>,
    pub(crate) status: TargetStatus,
    pub(crate) estimated_bytes: u64,
    pub(crate) estimate_source: EstimateSource,
    pub(crate) reason: Option<String>,
}

impl<'a> ProjectArtifactPlanProjection<'a> {
    pub(crate) fn new(plan: &'a CleanupPlan) -> Self {
        let mut summaries = BTreeMap::<String, ProjectArtifactSummaryAccumulator>::new();
        let mut grouped = BTreeMap::<PathBuf, Vec<ProjectArtifactRow<'a>>>::new();
        let mut recently_modified = Vec::new();

        for target in &plan.targets {
            let row = ProjectArtifactRow::from(target);
            summaries
                .entry(row.artifact_type.clone())
                .or_default()
                .record(&row);

            if target.reason_code == Some(CleanupTargetIssueReason::ProjectArtifactRecentlyModified)
            {
                recently_modified.push(ProjectArtifactRow::from(target));
            }

            grouped
                .entry(project_path_for(target.path.as_path()))
                .or_default()
                .push(row);
        }

        recently_modified.sort_by(|left, right| {
            left.path
                .cmp(right.path)
                .then_with(|| left.artifact_type.cmp(&right.artifact_type))
        });

        let project_groups = grouped
            .into_iter()
            .map(|(project_path, mut targets)| {
                targets.sort_by(|left, right| {
                    status_order(left.status)
                        .cmp(&status_order(right.status))
                        .then_with(|| left.artifact_type.cmp(&right.artifact_type))
                        .then_with(|| left.path.cmp(right.path))
                });

                let estimated_bytes = targets
                    .iter()
                    .map(|target| target.estimated_bytes)
                    .sum::<u64>();
                let targets_label = format_count(targets.len() as u64, "target", "targets");

                ProjectArtifactGroup {
                    project_path,
                    targets_label,
                    estimated_bytes,
                    targets,
                }
            })
            .collect();

        Self {
            artifact_summaries: artifact_summary_rows(summaries),
            largest_targets: largest_project_artifact_rows(plan),
            project_groups,
            recently_modified,
            discovery_diagnostics: discovery_diagnostic_rows(plan),
        }
    }

    pub(crate) fn artifact_summaries(&self) -> &[ProjectArtifactSummaryRow] {
        &self.artifact_summaries
    }

    pub(crate) fn largest_targets(&self) -> &[ProjectArtifactRow<'a>] {
        &self.largest_targets
    }

    pub(crate) fn project_groups(&self) -> &[ProjectArtifactGroup<'a>] {
        &self.project_groups
    }

    pub(crate) fn recently_modified(&self) -> &[ProjectArtifactRow<'a>] {
        &self.recently_modified
    }

    pub(crate) fn discovery_diagnostics(&self) -> &[ProjectArtifactDiscoveryDiagnosticRow<'a>] {
        &self.discovery_diagnostics
    }
}

impl ProjectArtifactInsightReport {
    pub(crate) fn from_plan(plan: &CleanupPlan) -> Self {
        Self {
            roots: plan.request.project_artifact_roots.clone(),
            summary: plan.summary.clone(),
            totals_by_root: insight_totals(plan, RootGrouping::Root),
            totals_by_project: insight_totals(plan, RootGrouping::Project),
            totals_by_artifact: insight_totals(plan, RootGrouping::Artifact),
            top_targets: insight_top_targets(plan),
            discovery_diagnostics: plan.discovery_diagnostics.clone(),
        }
    }
}

impl ProjectArtifactCatalogEntry {
    pub(crate) fn from_policy(policy: &'static ProjectArtifactPolicy) -> Self {
        let definition = policy.definition;
        let rule_suffix = project_artifact_rule_suffix(definition.rule_id);
        Self {
            artifact: policy.artifact,
            aliases: policy.aliases.to_vec(),
            rule_id: definition.rule_id,
            rule_suffix,
            restore_hint: definition.restore_hint,
            default_min_age_days: policy.default_min_age_days,
            trim_eligible: policy.trim_eligible,
            deletion_style: policy.deletion_style_label(),
            ranking: policy.ranking.label(),
            selectors_label: project_artifact_selectors_label(policy, rule_suffix),
        }
    }
}

pub(crate) fn project_artifact_catalog_entries() -> Vec<ProjectArtifactCatalogEntry> {
    all_project_artifact_policies()
        .map(ProjectArtifactCatalogEntry::from_policy)
        .collect()
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ProjectArtifactSummaryAccumulator {
    targets: u64,
    estimated_bytes: u64,
    allowed_targets: u64,
    completed_targets: u64,
    failed_targets: u64,
    blocked_targets: u64,
    skipped_targets: u64,
}

#[derive(Debug, Clone, Copy)]
enum RootGrouping {
    Root,
    Project,
    Artifact,
}

#[derive(Debug, Default)]
struct InsightTotalAccumulator {
    targets: usize,
    estimated_bytes: u64,
}

impl ProjectArtifactSummaryAccumulator {
    fn record(&mut self, row: &ProjectArtifactRow<'_>) {
        self.targets = self.targets.saturating_add(1);
        self.estimated_bytes = self.estimated_bytes.saturating_add(row.estimated_bytes);

        match row.status {
            TargetStatus::Allowed => self.allowed_targets = self.allowed_targets.saturating_add(1),
            TargetStatus::Completed => {
                self.completed_targets = self.completed_targets.saturating_add(1)
            }
            TargetStatus::Failed => self.failed_targets = self.failed_targets.saturating_add(1),
            TargetStatus::Blocked => self.blocked_targets = self.blocked_targets.saturating_add(1),
            TargetStatus::Skipped => self.skipped_targets = self.skipped_targets.saturating_add(1),
        }
    }

    fn status_summary_label(&self) -> String {
        [
            (self.allowed_targets, "allowed"),
            (self.completed_targets, "completed"),
            (self.failed_targets, "failed"),
            (self.blocked_targets, "blocked"),
            (self.skipped_targets, "skipped"),
        ]
        .into_iter()
        .filter(|(count, _)| *count > 0)
        .map(|(count, label)| format!("{count} {label}"))
        .collect::<Vec<_>>()
        .join(", ")
    }
}

impl<'a> From<&'a CleanupTarget> for ProjectArtifactRow<'a> {
    fn from(target: &'a CleanupTarget) -> Self {
        Self {
            artifact_type: artifact_type_label(&target.rule_id, target.path.as_path()),
            status: target.status,
            status_label: target.status.label(),
            path: target.path.as_path(),
            estimated_bytes: target.estimated_bytes,
            estimate_source: target.estimate_source,
            modified_at_unix_seconds: target.modified_at_unix_seconds,
            reason: target.reason.as_deref(),
            restore_hint: target.restore_hint.as_deref(),
        }
    }
}

impl<'a> From<&'a ProjectArtifactDiscoveryDiagnostic>
    for ProjectArtifactDiscoveryDiagnosticRow<'a>
{
    fn from(diagnostic: &'a ProjectArtifactDiscoveryDiagnostic) -> Self {
        Self {
            kind_label: diagnostic.kind.label(),
            path: diagnostic.path.as_path(),
            detail: diagnostic.detail.as_str(),
        }
    }
}

fn discovery_diagnostic_rows(plan: &CleanupPlan) -> Vec<ProjectArtifactDiscoveryDiagnosticRow<'_>> {
    plan.discovery_diagnostics
        .iter()
        .map(ProjectArtifactDiscoveryDiagnosticRow::from)
        .collect()
}

fn artifact_summary_rows(
    summaries: BTreeMap<String, ProjectArtifactSummaryAccumulator>,
) -> Vec<ProjectArtifactSummaryRow> {
    let mut rows = summaries
        .into_iter()
        .map(|(artifact_type, summary)| ProjectArtifactSummaryRow {
            artifact_type,
            targets_label: format_count(summary.targets, "target", "targets"),
            estimated_bytes: summary.estimated_bytes,
            status_summary_label: summary.status_summary_label(),
        })
        .collect::<Vec<_>>();

    rows.sort_by(|left, right| {
        right
            .estimated_bytes
            .cmp(&left.estimated_bytes)
            .then_with(|| left.artifact_type.cmp(&right.artifact_type))
    });

    rows
}

fn largest_project_artifact_rows(plan: &CleanupPlan) -> Vec<ProjectArtifactRow<'_>> {
    let mut targets = plan
        .targets
        .iter()
        .filter(|target| target.estimated_bytes > 0)
        .map(ProjectArtifactRow::from)
        .collect::<Vec<_>>();

    targets.sort_by(|left, right| {
        right
            .estimated_bytes
            .cmp(&left.estimated_bytes)
            .then_with(|| left.artifact_type.cmp(&right.artifact_type))
            .then_with(|| left.path.cmp(right.path))
    });

    targets.truncate(LARGEST_ARTIFACT_LIMIT);
    targets
}

fn insight_totals(plan: &CleanupPlan, grouping: RootGrouping) -> Vec<ProjectArtifactInsightTotal> {
    let mut totals = BTreeMap::<String, InsightTotalAccumulator>::new();

    for target in &plan.targets {
        let key = match grouping {
            RootGrouping::Root => root_for_target(plan, target.path.as_path())
                .unwrap_or_else(|| PathBuf::from("(outside configured roots)"))
                .display()
                .to_string(),
            RootGrouping::Project => target
                .project_artifact
                .as_ref()
                .map(|context| context.project_root.display().to_string())
                .unwrap_or_else(|| {
                    project_path_for(target.path.as_path())
                        .display()
                        .to_string()
                }),
            RootGrouping::Artifact => artifact_type_label(&target.rule_id, target.path.as_path()),
        };
        let entry = totals.entry(key).or_default();
        entry.targets = entry.targets.saturating_add(1);
        entry.estimated_bytes = entry.estimated_bytes.saturating_add(target.estimated_bytes);
    }

    totals
        .into_iter()
        .map(|(key, total)| ProjectArtifactInsightTotal {
            label: key.clone(),
            key,
            targets: total.targets,
            estimated_bytes: total.estimated_bytes,
        })
        .collect()
}

fn insight_top_targets(plan: &CleanupPlan) -> Vec<ProjectArtifactInsightTarget> {
    let mut targets = plan
        .targets
        .iter()
        .map(|target| ProjectArtifactInsightTarget {
            rule_id: target.rule_id.clone(),
            artifact: artifact_type_label(&target.rule_id, target.path.as_path()),
            path: target.path.clone(),
            project_root: target
                .project_artifact
                .as_ref()
                .map(|context| context.project_root.clone()),
            status: target.status,
            estimated_bytes: target.estimated_bytes,
            estimate_source: target.estimate_source,
            reason: target.reason.clone(),
        })
        .collect::<Vec<_>>();

    targets.sort_by(|left, right| {
        right
            .estimated_bytes
            .cmp(&left.estimated_bytes)
            .then_with(|| left.artifact.cmp(&right.artifact))
            .then_with(|| left.path.cmp(&right.path))
    });
    targets.truncate(INSIGHT_TOP_TARGET_LIMIT);
    targets
}

fn root_for_target(plan: &CleanupPlan, path: &Path) -> Option<PathBuf> {
    plan.request
        .project_artifact_roots
        .iter()
        .filter(|root| path.starts_with(root))
        .max_by_key(|root| root.components().count())
        .cloned()
}

fn project_artifact_selectors_label(
    policy: &ProjectArtifactPolicy,
    rule_suffix: &'static str,
) -> String {
    let mut selectors = Vec::new();
    push_project_artifact_selector(&mut selectors, policy.definition.directory_name);
    for alias in policy.aliases {
        push_project_artifact_selector(&mut selectors, alias);
    }
    push_project_artifact_selector(&mut selectors, rule_suffix);
    push_project_artifact_selector(&mut selectors, policy.definition.rule_id);
    selectors.join(", ")
}

fn push_project_artifact_selector(selectors: &mut Vec<&'static str>, selector: &'static str) {
    if !selectors
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(selector))
    {
        selectors.push(selector);
    }
}

fn project_artifact_rule_suffix(rule_id: &'static str) -> &'static str {
    rule_id
        .strip_prefix("windows.project-artifact-")
        .unwrap_or(rule_id)
}

fn artifact_type_label(rule_id: &str, path: &Path) -> String {
    match rule_id {
        "windows.project-artifact-cachedir-tag" => "CACHEDIR.TAG".to_string(),
        "windows.project-artifact-composer-vendor" => "vendor (Composer)".to_string(),
        "windows.project-artifact-dotnet-bin" => "bin (.NET)".to_string(),
        "windows.project-artifact-dotnet-obj" => "obj (.NET)".to_string(),
        _ => path
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_string)
            .unwrap_or_else(|| {
                rule_id
                    .strip_prefix("windows.project-artifact-")
                    .unwrap_or(rule_id)
                    .replace('-', "_")
            }),
    }
}

fn project_path_for(path: &Path) -> PathBuf {
    path.parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| path.to_path_buf())
}

fn status_order(status: TargetStatus) -> usize {
    match status {
        TargetStatus::Allowed => 0,
        TargetStatus::Completed => 1,
        TargetStatus::Failed => 2,
        TargetStatus::Blocked => 3,
        TargetStatus::Skipped => 4,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use rebecca::core::plan::{CleanupPlan, CleanupSummary, CleanupTargetIssueReason};
    use rebecca::core::{CleanupWorkflow, DeleteMode, PlanRequest, Platform};

    use super::*;

    fn plan_with_targets(targets: Vec<CleanupTarget>) -> CleanupPlan {
        let mut plan = CleanupPlan {
            request: PlanRequest::for_platform(Platform::Windows, DeleteMode::DryRun)
                .with_workflow(CleanupWorkflow::ProjectArtifacts),
            summary: CleanupSummary::default(),
            targets,
            discovery_diagnostics: Vec::new(),
        };
        plan.recompute_summary();
        plan
    }

    #[test]
    fn projection_groups_artifacts_by_parent_project_path() {
        let plan = plan_with_targets(vec![
            CleanupTarget::allowed(
                "windows.project-artifact-target",
                PathBuf::from("workspace/app/target"),
                4,
                DeleteMode::DryRun,
            ),
            CleanupTarget::allowed(
                "windows.project-artifact-node-modules",
                PathBuf::from("workspace/app/node_modules"),
                3,
                DeleteMode::DryRun,
            ),
        ]);

        let projection = ProjectArtifactPlanProjection::new(&plan);

        assert_eq!(projection.project_groups().len(), 1);
        let group = &projection.project_groups()[0];
        assert_eq!(group.project_path, PathBuf::from("workspace/app"));
        assert_eq!(group.targets_label, "2 targets");
        assert_eq!(group.estimated_bytes, 7);
        assert_eq!(group.targets[0].artifact_type, "node_modules");
        assert_eq!(group.targets[1].artifact_type, "target");

        let summaries = projection.artifact_summaries();
        assert_eq!(summaries[0].artifact_type, "target");
        assert_eq!(summaries[0].targets_label, "1 target");
        assert_eq!(summaries[0].estimated_bytes, 4);
        assert_eq!(summaries[0].status_summary_label, "1 allowed");
        assert_eq!(summaries[1].artifact_type, "node_modules");
        assert_eq!(summaries[1].estimated_bytes, 3);

        let largest = projection
            .largest_targets()
            .iter()
            .map(|target| target.artifact_type.as_str())
            .collect::<Vec<_>>();
        assert_eq!(largest, ["target", "node_modules"]);
    }

    #[test]
    fn projection_extracts_recently_modified_artifacts() {
        let target = CleanupTarget::skipped_with_reason_code(
            "windows.project-artifact-node-modules",
            PathBuf::from("workspace/app/node_modules"),
            DeleteMode::DryRun,
            CleanupTargetIssueReason::ProjectArtifactRecentlyModified,
            "project artifact was modified within the last 7 days",
        );
        let plan = plan_with_targets(vec![target]);

        let projection = ProjectArtifactPlanProjection::new(&plan);

        assert_eq!(projection.recently_modified().len(), 1);
        assert_eq!(
            projection.recently_modified()[0].artifact_type,
            "node_modules"
        );
        assert_eq!(
            projection.recently_modified()[0].reason,
            Some("project artifact was modified within the last 7 days")
        );
    }

    #[test]
    fn projection_labels_context_sensitive_artifact_types() {
        let plan = plan_with_targets(vec![
            CleanupTarget::allowed(
                "windows.project-artifact-composer-vendor",
                PathBuf::from("workspace/php-app/vendor"),
                1,
                DeleteMode::DryRun,
            ),
            CleanupTarget::allowed(
                "windows.project-artifact-dotnet-bin",
                PathBuf::from("workspace/dotnet-app/bin"),
                1,
                DeleteMode::DryRun,
            ),
            CleanupTarget::allowed(
                "windows.project-artifact-cachedir-tag",
                PathBuf::from("workspace/app/tool-cache"),
                1,
                DeleteMode::DryRun,
            ),
        ]);

        let projection = ProjectArtifactPlanProjection::new(&plan);
        let artifact_types = projection
            .project_groups()
            .iter()
            .flat_map(|group| {
                group
                    .targets
                    .iter()
                    .map(|target| target.artifact_type.as_str())
            })
            .collect::<Vec<_>>();

        assert!(artifact_types.contains(&"vendor (Composer)"));
        assert!(artifact_types.contains(&"bin (.NET)"));
        assert!(artifact_types.contains(&"CACHEDIR.TAG"));
    }

    #[test]
    fn catalog_entry_exposes_policy_fields_and_human_selectors() {
        let policy = all_project_artifact_policies()
            .find(|policy| policy.definition.rule_id == "windows.project-artifact-node-modules")
            .unwrap();
        let entry = ProjectArtifactCatalogEntry::from_policy(policy);

        assert_eq!(entry.artifact, "node_modules");
        assert_eq!(entry.aliases, vec!["node-modules"]);
        assert_eq!(entry.rule_suffix, "node-modules");
        assert_eq!(
            entry.selectors_label,
            "node_modules, node-modules, windows.project-artifact-node-modules"
        );
        assert_eq!(entry.default_min_age_days, 7);
        assert!(entry.trim_eligible);
        assert_eq!(entry.deletion_style, "delete-whole-path");
        assert_eq!(entry.ranking, "heavy-dependency-tree");

        let value = serde_json::to_value(&entry).unwrap();
        assert_eq!(value["artifact"], "node_modules");
        assert_eq!(value["aliases"], serde_json::json!(["node-modules"]));
        assert_eq!(value["rule_id"], "windows.project-artifact-node-modules");
        assert_eq!(value["rule_suffix"], "node-modules");
        assert_eq!(
            value["restore_hint"],
            "Dependencies can be restored with the project's package manager."
        );
        assert_eq!(value["default_min_age_days"], 7);
        assert_eq!(value["trim_eligible"], true);
        assert_eq!(value["deletion_style"], "delete-whole-path");
        assert_eq!(value["ranking"], "heavy-dependency-tree");
        assert!(value.get("selectors_label").is_none());
    }
}

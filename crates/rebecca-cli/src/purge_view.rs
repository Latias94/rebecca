use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rebecca_core::plan::{CleanupPlan, CleanupTarget, CleanupTargetIssueReason};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProjectArtifactPlanProjection<'a> {
    project_groups: Vec<ProjectArtifactGroup<'a>>,
    recently_modified: Vec<ProjectArtifactRow<'a>>,
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
    pub(crate) status_label: &'static str,
    pub(crate) path: &'a Path,
    pub(crate) estimated_bytes: u64,
    pub(crate) modified_at_unix_seconds: Option<u64>,
    pub(crate) reason: Option<&'a str>,
    pub(crate) restore_hint: Option<&'a str>,
}

impl<'a> ProjectArtifactPlanProjection<'a> {
    pub(crate) fn new(plan: &'a CleanupPlan) -> Self {
        let mut grouped = BTreeMap::<PathBuf, Vec<ProjectArtifactRow<'a>>>::new();
        let mut recently_modified = Vec::new();

        for target in &plan.targets {
            let row = ProjectArtifactRow::from(target);
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
                    status_order(left.status_label)
                        .cmp(&status_order(right.status_label))
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
            project_groups,
            recently_modified,
        }
    }

    pub(crate) fn project_groups(&self) -> &[ProjectArtifactGroup<'a>] {
        &self.project_groups
    }

    pub(crate) fn recently_modified(&self) -> &[ProjectArtifactRow<'a>] {
        &self.recently_modified
    }
}

impl<'a> From<&'a CleanupTarget> for ProjectArtifactRow<'a> {
    fn from(target: &'a CleanupTarget) -> Self {
        Self {
            artifact_type: artifact_type_label(&target.rule_id, target.path.as_path()),
            status_label: target.status.label(),
            path: target.path.as_path(),
            estimated_bytes: target.estimated_bytes,
            modified_at_unix_seconds: target.modified_at_unix_seconds,
            reason: target.reason.as_deref(),
            restore_hint: target.restore_hint.as_deref(),
        }
    }
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

fn status_order(status_label: &str) -> usize {
    match status_label {
        "allowed" => 0,
        "completed" => 1,
        "failed" => 2,
        "blocked" => 3,
        "skipped" => 4,
        _ => 5,
    }
}

fn format_count(count: u64, singular: &str, plural: &str) -> String {
    if count == 1 {
        format!("{count} {singular}")
    } else {
        format!("{count} {plural}")
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use rebecca_core::plan::{CleanupPlan, CleanupSummary, CleanupTargetIssueReason};
    use rebecca_core::{CleanupWorkflow, DeleteMode, PlanRequest, Platform};

    use super::*;

    fn plan_with_targets(targets: Vec<CleanupTarget>) -> CleanupPlan {
        let mut plan = CleanupPlan {
            request: PlanRequest::for_platform(Platform::Windows, DeleteMode::DryRun)
                .with_workflow(CleanupWorkflow::ProjectArtifacts),
            summary: CleanupSummary::default(),
            targets,
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
}

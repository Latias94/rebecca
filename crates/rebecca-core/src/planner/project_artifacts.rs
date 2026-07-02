use rayon::prelude::*;

use crate::TargetStatus;
use crate::error::Result;
use crate::model::PlanRequest;
use crate::plan::{CleanupPlan, CleanupTarget, CleanupTargetIssueReason};
use crate::project_artifacts::{
    ProjectArtifactCandidate, ProjectArtifactScanOptions,
    discover_project_artifacts_with_diagnostics, policy_for_rule_id,
    project_artifact_policy_matches_selectors, validate_project_artifact_selectors,
};
use crate::scan::run_scoped_scan;

use super::measure::{
    emit_measured_target_progress, emit_target_finished, finalize_plan,
    measure_project_artifact_candidate, project_artifact_skipped_target, prune_scan_cache,
};
use super::{PlanBuildContext, PlanProgressEvent};

pub(crate) fn build_project_artifact_plan_with_context<F>(
    request: &PlanRequest,
    context: PlanBuildContext<'_>,
    mut progress: F,
) -> Result<CleanupPlan>
where
    F: for<'a> FnMut(PlanProgressEvent<'a>),
{
    validate_project_artifact_selectors(&request.project_artifact_selectors)?;
    let scan_options = ProjectArtifactScanOptions::new(request.project_artifact_roots.clone())
        .with_max_depth(request.project_artifact_max_depth);
    let discovery =
        discover_project_artifacts_with_diagnostics(&scan_options, context.cancellation())?;
    let mut filtered_artifacts = discovery
        .candidates
        .into_iter()
        .filter(|artifact| {
            project_artifact_policy_matches_selectors(
                artifact.policy,
                &request.project_artifact_selectors,
            )
        })
        .collect::<Vec<_>>();
    sort_candidates_for_bounded_measurement(&mut filtered_artifacts);

    let plan_candidates = if let Some(limit) = request.project_artifact_reclaim_limit_bytes {
        measure_project_artifacts_until_reclaim_limit(
            filtered_artifacts,
            request,
            context.clone(),
            limit,
            &mut progress,
        )?
    } else {
        measure_project_artifacts_in_parallel(
            filtered_artifacts,
            request,
            context.clone(),
            &mut progress,
        )?
    };

    prune_scan_cache(context.clone(), &mut progress);
    for target in &plan_candidates {
        emit_target_finished(&mut progress, target);
    }
    let mut plan = finalize_plan(request.clone(), plan_candidates);
    plan.discovery_diagnostics = discovery.diagnostics;
    Ok(plan)
}

fn measure_project_artifacts_in_parallel<F>(
    artifacts: Vec<ProjectArtifactCandidate>,
    request: &PlanRequest,
    context: PlanBuildContext<'_>,
    progress: &mut F,
) -> Result<Vec<CleanupTarget>>
where
    F: for<'a> FnMut(PlanProgressEvent<'a>),
{
    for artifact in &artifacts {
        progress(PlanProgressEvent::TargetScanning {
            rule_id: artifact.definition.rule_id,
            path: &artifact.path,
        });
    }

    let candidates = run_scoped_scan(|| {
        artifacts
            .into_par_iter()
            .map(|artifact| {
                measure_project_artifact_candidate(
                    artifact,
                    request.mode,
                    request.project_artifact_min_age_days,
                    context.clone(),
                )
            })
            .collect::<Vec<_>>()
    });

    let mut targets = Vec::with_capacity(candidates.len());
    for measured in candidates {
        let measured = measured?;
        emit_measured_target_progress(progress, &measured);
        targets.push(measured.target);
    }
    Ok(targets)
}

fn measure_project_artifacts_until_reclaim_limit<F>(
    artifacts: Vec<ProjectArtifactCandidate>,
    request: &PlanRequest,
    context: PlanBuildContext<'_>,
    limit: u64,
    progress: &mut F,
) -> Result<Vec<CleanupTarget>>
where
    F: for<'a> FnMut(PlanProgressEvent<'a>),
{
    let mut targets = Vec::with_capacity(artifacts.len());
    let mut selected_bytes = 0_u64;
    let mut limit_satisfied = false;

    for artifact in artifacts {
        if limit_satisfied && artifact.policy.trim_eligible {
            targets.push(reclaim_limit_satisfied_target(
                &artifact,
                request.mode,
                limit,
            ));
            continue;
        }

        if limit == 0 && artifact.policy.trim_eligible {
            limit_satisfied = true;
            targets.push(reclaim_limit_satisfied_target(
                &artifact,
                request.mode,
                limit,
            ));
            continue;
        }

        progress(PlanProgressEvent::TargetScanning {
            rule_id: artifact.definition.rule_id,
            path: &artifact.path,
        });
        let measured = measure_project_artifact_candidate(
            artifact,
            request.mode,
            request.project_artifact_min_age_days,
            context.clone(),
        )?;
        emit_measured_target_progress(progress, &measured);

        if measured.target.status == TargetStatus::Allowed
            && measured.target.estimated_bytes > 0
            && policy_trim_eligible(&measured.target)
        {
            selected_bytes = selected_bytes.saturating_add(measured.target.estimated_bytes);
            limit_satisfied = selected_bytes >= limit;
        }

        targets.push(measured.target);
    }

    Ok(targets)
}

fn reclaim_limit_satisfied_target(
    artifact: &ProjectArtifactCandidate,
    mode: crate::DeleteMode,
    limit: u64,
) -> CleanupTarget {
    project_artifact_skipped_target(
        artifact,
        mode,
        CleanupTargetIssueReason::ReclaimLimitSatisfied,
        format!(
            "project artifact was not measured because the reclaim limit of {limit} bytes was already satisfied"
        ),
    )
}

fn policy_trim_eligible(target: &CleanupTarget) -> bool {
    policy_for_rule_id(&target.rule_id)
        .map(|policy| policy.trim_eligible)
        .unwrap_or(true)
}

fn sort_candidates_for_bounded_measurement(candidates: &mut [ProjectArtifactCandidate]) {
    candidates.sort_by_key(candidate_sort_key);
}

fn candidate_sort_key(candidate: &ProjectArtifactCandidate) -> ProjectArtifactMeasureSortKey {
    ProjectArtifactMeasureSortKey {
        trim_ineligible: !candidate.policy.trim_eligible,
        ranking: candidate.policy.ranking.priority(),
        path: candidate.path.clone(),
        rule_id: candidate.definition.rule_id,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ProjectArtifactMeasureSortKey {
    trim_ineligible: bool,
    ranking: u8,
    path: std::path::PathBuf,
    rule_id: &'static str,
}

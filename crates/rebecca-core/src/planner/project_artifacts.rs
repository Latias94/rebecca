use rayon::prelude::*;

use crate::TargetStatus;
use crate::error::Result;
use crate::model::PlanRequest;
use crate::plan::{CleanupPlan, CleanupTarget, CleanupTargetIssueReason, EstimateSource};
use crate::project_artifacts::{
    ProjectArtifactScanOptions, discover_project_artifacts_with_diagnostics, policy_for_rule_id,
    project_artifact_policy_matches_selectors, validate_project_artifact_selectors,
};
use crate::scan::run_scoped_scan;

use super::measure::{
    emit_measured_target_progress, emit_target_finished, finalize_plan,
    measure_project_artifact_candidate, prune_scan_cache,
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
    let filtered_artifacts = discovery
        .candidates
        .into_iter()
        .filter(|artifact| {
            project_artifact_policy_matches_selectors(
                artifact.policy,
                &request.project_artifact_selectors,
            )
        })
        .collect::<Vec<_>>();

    for artifact in &filtered_artifacts {
        progress(PlanProgressEvent::TargetScanning {
            rule_id: artifact.definition.rule_id,
            path: &artifact.path,
        });
    }

    let candidates = run_scoped_scan(|| {
        filtered_artifacts
            .into_par_iter()
            .map(|artifact| {
                measure_project_artifact_candidate(
                    artifact,
                    request.mode,
                    request.project_artifact_min_age_days,
                    context,
                )
            })
            .collect::<Vec<_>>()
    });

    let mut plan_candidates = Vec::with_capacity(candidates.len());
    for measured in candidates {
        let measured = measured?;
        emit_measured_target_progress(&mut progress, &measured);
        plan_candidates.push(measured.target);
    }

    prune_scan_cache(context, &mut progress);
    apply_reclaim_limit(
        request.project_artifact_reclaim_limit_bytes,
        &mut plan_candidates,
    );
    for target in &plan_candidates {
        emit_target_finished(&mut progress, target);
    }
    let mut plan = finalize_plan(request.clone(), plan_candidates);
    plan.discovery_diagnostics = discovery.diagnostics;
    Ok(plan)
}

fn apply_reclaim_limit(limit: Option<u64>, targets: &mut [CleanupTarget]) {
    let Some(limit) = limit else {
        return;
    };

    let mut selected_bytes = 0_u64;
    let mut target_order = targets
        .iter()
        .enumerate()
        .filter(|(_, target)| {
            target.status == TargetStatus::Allowed
                && target.estimated_bytes > 0
                && policy_trim_eligible(target)
        })
        .map(|(index, target)| (index, target_sort_key(target)))
        .collect::<Vec<_>>();
    target_order.sort_by(|left, right| left.1.cmp(&right.1));

    for (index, _) in target_order {
        if selected_bytes >= limit {
            let reason = format!("project artifact was outside the reclaim limit of {limit} bytes");
            targets[index].status = TargetStatus::Skipped;
            targets[index].reason_code = Some(CleanupTargetIssueReason::ReclaimLimitExceeded);
            targets[index].reason = Some(reason);
            targets[index].estimated_bytes = 0;
            targets[index].estimate_source = EstimateSource::NotMeasured;
            targets[index].pending_reclaim_bytes = 0;
            continue;
        }

        selected_bytes = selected_bytes.saturating_add(targets[index].estimated_bytes);
    }
}

fn policy_trim_eligible(target: &CleanupTarget) -> bool {
    policy_for_rule_id(&target.rule_id)
        .map(|policy| policy.trim_eligible)
        .unwrap_or(true)
}

fn target_sort_key(target: &CleanupTarget) -> ProjectArtifactTrimSortKey {
    let policy = policy_for_rule_id(&target.rule_id);
    ProjectArtifactTrimSortKey {
        largest_bytes: std::cmp::Reverse(target.estimated_bytes),
        ranking: policy
            .map(|policy| policy.ranking.priority())
            .unwrap_or(u8::MAX),
        path: target.path.clone(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ProjectArtifactTrimSortKey {
    largest_bytes: std::cmp::Reverse<u64>,
    ranking: u8,
    path: std::path::PathBuf,
}

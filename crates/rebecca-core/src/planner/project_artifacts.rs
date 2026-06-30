use rayon::prelude::*;

use crate::error::Result;
use crate::model::PlanRequest;
use crate::plan::CleanupPlan;
use crate::project_artifacts::{
    ProjectArtifactScanOptions, discover_project_artifacts_with_diagnostics,
    project_artifact_matches_selectors, validate_project_artifact_selectors,
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
            project_artifact_matches_selectors(
                artifact.definition,
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
        emit_target_finished(&mut progress, &measured.target);
        plan_candidates.push(measured.target);
    }

    prune_scan_cache(context, &mut progress);
    let mut plan = finalize_plan(request.clone(), plan_candidates);
    plan.discovery_diagnostics = discovery.diagnostics;
    Ok(plan)
}

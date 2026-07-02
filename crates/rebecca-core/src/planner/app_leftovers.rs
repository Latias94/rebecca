use std::collections::BTreeSet;

use rayon::prelude::*;

use crate::app_leftovers::derive_app_leftover_candidates;
use crate::applications::ApplicationDiscovery;
use crate::environment::Environment;
use crate::error::Result;
use crate::model::PlanRequest;
use crate::plan::{CleanupPlan, CleanupTargetIssueReason};
use crate::scan::run_scoped_scan;

use super::measure::{
    app_leftover_rule_id, app_leftover_skipped_target, dedupe_key, emit_measured_target_progress,
    emit_target_finished, finalize_plan, measure_app_leftover_candidate, prune_scan_cache,
};
use super::{PlanBuildContext, PlanProgressEvent};

pub(crate) fn build_app_leftover_plan_with_context<E, A, F>(
    request: &PlanRequest,
    env: &E,
    applications: &A,
    context: PlanBuildContext<'_>,
    mut progress: F,
) -> Result<CleanupPlan>
where
    E: Environment,
    A: ApplicationDiscovery + ?Sized,
    F: for<'a> FnMut(PlanProgressEvent<'a>),
{
    let installed = applications.installed_applications()?;
    let leftovers = derive_app_leftover_candidates(&installed, env);
    let mut seen_paths = BTreeSet::new();
    let mut duplicate_targets = Vec::new();
    let mut unique_leftovers = Vec::new();

    for leftover in leftovers {
        let path_key = dedupe_key(&leftover.path, request.platform);
        if !seen_paths.insert(path_key) {
            let target = app_leftover_skipped_target(
                &leftover,
                request.mode,
                CleanupTargetIssueReason::DuplicateTargetPath,
                "duplicate target path already covered",
            );
            emit_target_finished(&mut progress, &target);
            duplicate_targets.push(target);
            continue;
        }

        progress(PlanProgressEvent::TargetScanning {
            rule_id: app_leftover_rule_id(&leftover),
            path: &leftover.path,
        });
        unique_leftovers.push(leftover);
    }

    let measured_targets = run_scoped_scan(|| {
        unique_leftovers
            .into_par_iter()
            .map(|leftover| measure_app_leftover_candidate(leftover, request.mode, context.clone()))
            .collect::<Vec<_>>()
    });

    let mut candidates = duplicate_targets;
    for measured in measured_targets {
        let measured = measured?;
        emit_measured_target_progress(&mut progress, &measured);
        emit_target_finished(&mut progress, &measured.target);
        candidates.push(measured.target);
    }

    prune_scan_cache(context.clone(), &mut progress);
    Ok(finalize_plan(request.clone(), candidates))
}

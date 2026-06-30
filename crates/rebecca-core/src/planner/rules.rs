use std::collections::BTreeSet;

use crate::applications::ApplicationDiscovery;
use crate::discovery::{TargetResolution, resolve_rule_target_with_applications};
use crate::environment::Environment;
use crate::error::{RebeccaError, Result};
use crate::model::{PlanRequest, RuleDefinition};
use crate::plan::{CleanupPlan, CleanupTarget, CleanupTargetIssueReason};
use crate::safety::{
    PATH_DOES_NOT_EXIST_REASON, PathDisposition, assess_existing_path_with_policy,
};

use super::measure::{
    PathMeasureProgressEvent, dedupe_key, emit_target_finished, finalize_plan,
    measure_path_with_optional_scan_cache, prune_scan_cache,
};
use super::{PlanBuildContext, PlanProgressEvent};

pub(crate) fn build_rule_plan_with_context<E, A, F>(
    request: &PlanRequest,
    rules: &[RuleDefinition],
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
    let selection = request.selection();
    selection.validate_against_rules(rules)?;

    let mut candidates = Vec::new();
    let mut seen_paths = BTreeSet::new();

    for rule in rules {
        if rule.platform != request.platform {
            continue;
        }

        if !selection.matches_rule(rule) {
            continue;
        }

        if !request.allows_safety_level(rule.safety_level) {
            for spec in &rule.path_templates {
                let reason = match rule.safety_level.opt_in_flag() {
                    Some(flag) => format!("{} rule requires {}", rule.safety_level.label(), flag),
                    None => format!(
                        "{} rule requires explicit opt-in",
                        rule.safety_level.label()
                    ),
                };
                candidates.push(with_rule_restore_hint(
                    CleanupTarget::skipped_with_reason_code(
                        rule.id.clone(),
                        spec.placeholder_path(),
                        request.mode,
                        CleanupTargetIssueReason::SafetyOptInRequired,
                        reason,
                    ),
                    rule,
                ));
            }
            continue;
        }

        for spec in &rule.path_templates {
            let expanded_paths =
                match resolve_rule_target_with_applications(spec, env, applications) {
                    Ok(TargetResolution::Paths(paths)) => paths,
                    Ok(TargetResolution::Skipped(reason)) => {
                        candidates.push(with_rule_restore_hint(
                            CleanupTarget::skipped_with_reason_code(
                                rule.id.clone(),
                                spec.placeholder_path(),
                                request.mode,
                                CleanupTargetIssueReason::TargetDiscoverySkipped,
                                reason,
                            ),
                            rule,
                        ));
                        continue;
                    }
                    Err(err) => {
                        candidates.push(with_rule_restore_hint(
                            CleanupTarget::blocked_with_reason_code(
                                rule.id.clone(),
                                spec.placeholder_path(),
                                request.mode,
                                CleanupTargetIssueReason::TargetDiscoveryFailed,
                                err.to_string(),
                            ),
                            rule,
                        ));
                        continue;
                    }
                };

            for expanded in expanded_paths {
                let path_key = dedupe_key(&expanded, request.platform);
                if !seen_paths.insert(path_key) {
                    let target = with_rule_restore_hint(
                        CleanupTarget::skipped_with_reason_code(
                            rule.id.clone(),
                            expanded,
                            request.mode,
                            CleanupTargetIssueReason::DuplicateTargetPath,
                            "duplicate target path already covered",
                        ),
                        rule,
                    );
                    emit_target_finished(&mut progress, &target);
                    candidates.push(target);
                    continue;
                }

                progress(PlanProgressEvent::TargetScanning {
                    rule_id: &rule.id,
                    path: &expanded,
                });

                match assess_existing_path_with_policy(&expanded, context.protection_policy()) {
                    PathDisposition::Allowed => {
                        match measure_path_with_optional_scan_cache(&expanded, context, |event| {
                            match event {
                                PathMeasureProgressEvent::Scan(
                                    crate::scan::ScanProgressEvent::FileMeasured {
                                        path,
                                        file_size,
                                        files_scanned,
                                        bytes_scanned,
                                    },
                                ) => {
                                    progress(PlanProgressEvent::FileMeasured {
                                        rule_id: &rule.id,
                                        target_path: &expanded,
                                        path,
                                        file_size,
                                        files_scanned,
                                        bytes_scanned,
                                    });
                                }
                                PathMeasureProgressEvent::ScanCacheHit { report } => {
                                    progress(PlanProgressEvent::ScanCacheHit {
                                        rule_id: &rule.id,
                                        path: &expanded,
                                        estimated_bytes: report.bytes_scanned,
                                    });
                                }
                                PathMeasureProgressEvent::ScanCacheMiss { reason, pruned } => {
                                    progress(PlanProgressEvent::ScanCacheMiss {
                                        rule_id: &rule.id,
                                        path: &expanded,
                                        reason,
                                        pruned,
                                    });
                                }
                                PathMeasureProgressEvent::ScanCacheWriteSkipped => {
                                    progress(PlanProgressEvent::ScanCacheWriteSkipped {
                                        rule_id: &rule.id,
                                        path: &expanded,
                                    });
                                }
                            }
                        }) {
                            Ok(measured_path) => {
                                let target = CleanupTarget::allowed(
                                    rule.id.clone(),
                                    expanded,
                                    measured_path.report.bytes_scanned,
                                    request.mode,
                                )
                                .with_estimate_source(measured_path.estimate_source);
                                let target = with_rule_restore_hint(target, rule);
                                emit_target_finished(&mut progress, &target);
                                candidates.push(target);
                            }
                            Err(err @ RebeccaError::OperationCancelled(_)) => return Err(err),
                            Err(err) => {
                                let target = CleanupTarget::failed_with_reason_code(
                                    rule.id.clone(),
                                    expanded,
                                    request.mode,
                                    0,
                                    CleanupTargetIssueReason::ScanFailed,
                                    err.to_string(),
                                );
                                let target = with_rule_restore_hint(target, rule);
                                emit_target_finished(&mut progress, &target);
                                candidates.push(target);
                            }
                        }
                    }
                    PathDisposition::Missing => {
                        let target = with_rule_restore_hint(
                            CleanupTarget::skipped_with_reason_code(
                                rule.id.clone(),
                                expanded,
                                request.mode,
                                CleanupTargetIssueReason::SafetyPolicySkipped,
                                PATH_DOES_NOT_EXIST_REASON,
                            ),
                            rule,
                        );
                        emit_target_finished(&mut progress, &target);
                        candidates.push(target);
                    }
                    PathDisposition::Skipped(reason) => {
                        let target = with_rule_restore_hint(
                            CleanupTarget::skipped_with_reason_code(
                                rule.id.clone(),
                                expanded,
                                request.mode,
                                CleanupTargetIssueReason::SafetyPolicySkipped,
                                reason,
                            ),
                            rule,
                        );
                        emit_target_finished(&mut progress, &target);
                        candidates.push(target);
                    }
                    PathDisposition::Blocked(reason) => {
                        let target = with_rule_restore_hint(
                            CleanupTarget::blocked_with_reason_code(
                                rule.id.clone(),
                                expanded,
                                request.mode,
                                CleanupTargetIssueReason::SafetyPolicyBlocked,
                                reason,
                            ),
                            rule,
                        );
                        emit_target_finished(&mut progress, &target);
                        candidates.push(target);
                    }
                }
            }
        }
    }

    prune_scan_cache(context, &mut progress);
    Ok(finalize_plan(request.clone(), candidates))
}

fn with_rule_restore_hint(target: CleanupTarget, rule: &RuleDefinition) -> CleanupTarget {
    target.with_restore_hint(rule.restore_hint.clone())
}

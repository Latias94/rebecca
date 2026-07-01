use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use rayon::prelude::*;

use crate::applications::ApplicationDiscovery;
use crate::discovery::{
    DiscoveryIndex, TargetResolution, resolve_rule_target_with_applications_and_index,
};
use crate::environment::Environment;
use crate::error::{RebeccaError, Result};
use crate::model::{PlanRequest, RuleDefinition};
use crate::plan::{CleanupPlan, CleanupTarget, CleanupTargetIssueReason};
use crate::safety::{
    PATH_DOES_NOT_EXIST_REASON, PathDisposition, assess_existing_path_with_policy,
};
use crate::scan::{ScanCancellationToken, ScanProgressEvent, run_scoped_scan};
use crate::scan_cache::ScanCacheMiss;
use crate::warnings::warning_gate_required_reason;

use super::measure::{
    MeasuredPath, PathMeasureProgressEvent, dedupe_key, emit_target_finished, finalize_plan,
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

    let mut staged_candidates = Vec::new();
    let mut seen_paths = BTreeSet::new();
    let mut discovery_index = DiscoveryIndex::new();

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
                staged_candidates.push(RulePlanCandidate::target(
                    with_rule_metadata(
                        CleanupTarget::skipped_with_reason_code(
                            rule.id.clone(),
                            spec.placeholder_path(),
                            request.mode,
                            CleanupTargetIssueReason::SafetyOptInRequired,
                            reason,
                        ),
                        rule,
                    ),
                    TargetProgressPolicy::Silent,
                ));
            }
            continue;
        }

        let missing_warning_gates = request.missing_warning_gates(&rule.warnings);
        if !missing_warning_gates.is_empty() {
            for spec in &rule.path_templates {
                staged_candidates.push(RulePlanCandidate::target(
                    with_rule_metadata(
                        CleanupTarget::skipped_with_reason_code(
                            rule.id.clone(),
                            spec.placeholder_path(),
                            request.mode,
                            CleanupTargetIssueReason::WarningGateRequired,
                            warning_gate_required_reason(&missing_warning_gates),
                        ),
                        rule,
                    ),
                    TargetProgressPolicy::Silent,
                ));
            }
            continue;
        }

        for spec in &rule.path_templates {
            let expanded_paths = match resolve_rule_target_with_applications_and_index(
                spec,
                env,
                applications,
                &mut discovery_index,
            ) {
                Ok(TargetResolution::Paths(paths)) => paths,
                Ok(TargetResolution::Skipped(reason)) => {
                    staged_candidates.push(RulePlanCandidate::target(
                        with_rule_metadata(
                            CleanupTarget::skipped_with_reason_code(
                                rule.id.clone(),
                                spec.placeholder_path(),
                                request.mode,
                                CleanupTargetIssueReason::TargetDiscoverySkipped,
                                reason,
                            ),
                            rule,
                        ),
                        TargetProgressPolicy::Silent,
                    ));
                    continue;
                }
                Err(err) => {
                    staged_candidates.push(RulePlanCandidate::target(
                        with_rule_metadata(
                            CleanupTarget::blocked_with_reason_code(
                                rule.id.clone(),
                                spec.placeholder_path(),
                                request.mode,
                                CleanupTargetIssueReason::TargetDiscoveryFailed,
                                err.to_string(),
                            ),
                            rule,
                        ),
                        TargetProgressPolicy::Silent,
                    ));
                    continue;
                }
            };

            for expanded in expanded_paths {
                let path_key = dedupe_key(&expanded, request.platform);
                if !seen_paths.insert(path_key) {
                    let target = with_rule_metadata(
                        CleanupTarget::skipped_with_reason_code(
                            rule.id.clone(),
                            expanded,
                            request.mode,
                            CleanupTargetIssueReason::DuplicateTargetPath,
                            "duplicate target path already covered",
                        ),
                        rule,
                    );
                    staged_candidates.push(RulePlanCandidate::target(
                        target,
                        TargetProgressPolicy::FinishedOnly,
                    ));
                    continue;
                }

                match assess_existing_path_with_policy(&expanded, context.protection_policy()) {
                    PathDisposition::Allowed => {
                        staged_candidates.push(RulePlanCandidate::measure(
                            expanded,
                            RuleTargetMetadata::from_rule(rule),
                        ));
                    }
                    PathDisposition::Missing => {
                        staged_candidates.push(RulePlanCandidate::target(
                            with_rule_metadata(
                                CleanupTarget::skipped_with_reason_code(
                                    rule.id.clone(),
                                    expanded,
                                    request.mode,
                                    CleanupTargetIssueReason::SafetyPolicySkipped,
                                    PATH_DOES_NOT_EXIST_REASON,
                                ),
                                rule,
                            ),
                            TargetProgressPolicy::ScanningAndFinished,
                        ));
                    }
                    PathDisposition::Skipped(reason) => {
                        staged_candidates.push(RulePlanCandidate::target(
                            with_rule_metadata(
                                CleanupTarget::skipped_with_reason_code(
                                    rule.id.clone(),
                                    expanded,
                                    request.mode,
                                    CleanupTargetIssueReason::SafetyPolicySkipped,
                                    reason,
                                ),
                                rule,
                            ),
                            TargetProgressPolicy::ScanningAndFinished,
                        ));
                    }
                    PathDisposition::Blocked(reason) => {
                        staged_candidates.push(RulePlanCandidate::target(
                            with_rule_metadata(
                                CleanupTarget::blocked_with_reason_code(
                                    rule.id.clone(),
                                    expanded,
                                    request.mode,
                                    CleanupTargetIssueReason::SafetyPolicyBlocked,
                                    reason,
                                ),
                                rule,
                            ),
                            TargetProgressPolicy::ScanningAndFinished,
                        ));
                    }
                }
            }
        }
    }

    let measurement_outputs = measure_rule_candidates_in_parallel(&staged_candidates, context);
    let mut measurement_outputs = measurement_outputs
        .into_iter()
        .map(|output| (output.candidate_index, output))
        .collect::<BTreeMap<_, _>>();
    let mut candidates = Vec::with_capacity(staged_candidates.len());

    for (candidate_index, candidate) in staged_candidates.into_iter().enumerate() {
        match candidate {
            RulePlanCandidate::Target {
                target,
                progress_policy,
            } => {
                emit_staged_target_progress(
                    &mut progress,
                    target.as_ref(),
                    progress_policy,
                    context,
                )?;
                candidates.push(*target);
            }
            RulePlanCandidate::Measure(measurement) => {
                progress(PlanProgressEvent::TargetScanning {
                    rule_id: &measurement.metadata.rule_id,
                    path: &measurement.path,
                });
                ensure_not_cancelled(context.cancellation())?;

                let output = measurement_outputs
                    .remove(&candidate_index)
                    .expect("measured rule candidate should have one output");
                emit_rule_measurement_progress(
                    &mut progress,
                    &measurement.metadata,
                    &measurement.path,
                    &output.progress_events,
                    context.cancellation(),
                )?;

                let target = match output.outcome {
                    Ok(measured_path) => measurement.allowed_target(measured_path, request.mode),
                    Err(err @ RebeccaError::OperationCancelled(_)) => return Err(err),
                    Err(err) => measurement.failed_target(err, request.mode),
                };
                emit_target_finished(&mut progress, &target);
                ensure_not_cancelled(context.cancellation())?;
                candidates.push(target);
            }
        }
    }

    prune_scan_cache(context, &mut progress);
    Ok(finalize_plan(request.clone(), candidates))
}

#[derive(Debug, Clone)]
enum RulePlanCandidate {
    Target {
        target: Box<CleanupTarget>,
        progress_policy: TargetProgressPolicy,
    },
    Measure(RuleMeasurementCandidate),
}

impl RulePlanCandidate {
    fn target(target: CleanupTarget, progress_policy: TargetProgressPolicy) -> Self {
        Self::Target {
            target: Box::new(target),
            progress_policy,
        }
    }

    fn measure(path: PathBuf, metadata: RuleTargetMetadata) -> Self {
        Self::Measure(RuleMeasurementCandidate { path, metadata })
    }
}

#[derive(Debug, Clone, Copy)]
enum TargetProgressPolicy {
    Silent,
    FinishedOnly,
    ScanningAndFinished,
}

#[derive(Debug, Clone)]
struct RuleMeasurementCandidate {
    path: PathBuf,
    metadata: RuleTargetMetadata,
}

impl RuleMeasurementCandidate {
    fn allowed_target(
        &self,
        measured_path: MeasuredPath,
        mode: crate::DeleteMode,
    ) -> CleanupTarget {
        self.metadata.apply(
            CleanupTarget::allowed(
                self.metadata.rule_id.clone(),
                self.path.clone(),
                measured_path.report.bytes_scanned,
                mode,
            )
            .with_estimate_source(measured_path.estimate_source)
            .with_estimate_provenance(measured_path.estimate_provenance),
        )
    }

    fn failed_target(&self, err: RebeccaError, mode: crate::DeleteMode) -> CleanupTarget {
        self.metadata.apply(CleanupTarget::failed_with_reason_code(
            self.metadata.rule_id.clone(),
            self.path.clone(),
            mode,
            0,
            CleanupTargetIssueReason::ScanFailed,
            err.to_string(),
        ))
    }
}

#[derive(Debug, Clone)]
struct RuleTargetMetadata {
    rule_id: String,
    restore_hint: Option<String>,
    warnings: Vec<String>,
}

impl RuleTargetMetadata {
    fn from_rule(rule: &RuleDefinition) -> Self {
        Self {
            rule_id: rule.id.clone(),
            restore_hint: rule.restore_hint.clone(),
            warnings: rule.warnings.clone(),
        }
    }

    fn apply(&self, target: CleanupTarget) -> CleanupTarget {
        target
            .with_restore_hint(self.restore_hint.clone())
            .with_warnings(self.warnings.clone())
    }
}

#[derive(Debug)]
struct RuleMeasurementOutput {
    candidate_index: usize,
    progress_events: Vec<OwnedRuleMeasureProgressEvent>,
    outcome: Result<MeasuredPath>,
}

#[derive(Debug, Clone)]
enum OwnedRuleMeasureProgressEvent {
    FileMeasured {
        path: PathBuf,
        file_size: u64,
        files_scanned: u64,
        bytes_scanned: u64,
    },
    ScanCacheHit {
        estimated_bytes: u64,
    },
    ScanCacheMiss {
        reason: ScanCacheMiss,
        pruned: bool,
    },
    ScanCacheWriteSkipped,
}

fn measure_rule_candidates_in_parallel(
    candidates: &[RulePlanCandidate],
    context: PlanBuildContext<'_>,
) -> Vec<RuleMeasurementOutput> {
    let measurement_jobs = candidates
        .iter()
        .enumerate()
        .filter_map(|(candidate_index, candidate)| match candidate {
            RulePlanCandidate::Target { .. } => None,
            RulePlanCandidate::Measure(measurement) => Some((candidate_index, measurement.clone())),
        })
        .collect::<Vec<_>>();

    run_scoped_scan(|| {
        measurement_jobs
            .into_par_iter()
            .map(|(candidate_index, measurement)| {
                let mut progress_events = Vec::new();
                let outcome =
                    measure_path_with_optional_scan_cache(&measurement.path, context, |event| {
                        progress_events.push(OwnedRuleMeasureProgressEvent::from(event));
                    });

                RuleMeasurementOutput {
                    candidate_index,
                    progress_events,
                    outcome,
                }
            })
            .collect()
    })
}

fn emit_staged_target_progress<F>(
    progress: &mut F,
    target: &CleanupTarget,
    progress_policy: TargetProgressPolicy,
    context: PlanBuildContext<'_>,
) -> Result<()>
where
    F: for<'a> FnMut(PlanProgressEvent<'a>),
{
    match progress_policy {
        TargetProgressPolicy::Silent => {}
        TargetProgressPolicy::FinishedOnly => {
            emit_target_finished(progress, target);
            ensure_not_cancelled(context.cancellation())?;
        }
        TargetProgressPolicy::ScanningAndFinished => {
            progress(PlanProgressEvent::TargetScanning {
                rule_id: &target.rule_id,
                path: &target.path,
            });
            ensure_not_cancelled(context.cancellation())?;
            emit_target_finished(progress, target);
            ensure_not_cancelled(context.cancellation())?;
        }
    }

    Ok(())
}

fn emit_rule_measurement_progress<F>(
    progress: &mut F,
    metadata: &RuleTargetMetadata,
    target_path: &PathBuf,
    events: &[OwnedRuleMeasureProgressEvent],
    cancellation: &ScanCancellationToken,
) -> Result<()>
where
    F: for<'a> FnMut(PlanProgressEvent<'a>),
{
    for event in events {
        match event {
            OwnedRuleMeasureProgressEvent::FileMeasured {
                path,
                file_size,
                files_scanned,
                bytes_scanned,
            } => {
                progress(PlanProgressEvent::FileMeasured {
                    rule_id: &metadata.rule_id,
                    target_path,
                    path,
                    file_size: *file_size,
                    files_scanned: *files_scanned,
                    bytes_scanned: *bytes_scanned,
                });
            }
            OwnedRuleMeasureProgressEvent::ScanCacheHit { estimated_bytes } => {
                progress(PlanProgressEvent::ScanCacheHit {
                    rule_id: &metadata.rule_id,
                    path: target_path,
                    estimated_bytes: *estimated_bytes,
                });
            }
            OwnedRuleMeasureProgressEvent::ScanCacheMiss { reason, pruned } => {
                progress(PlanProgressEvent::ScanCacheMiss {
                    rule_id: &metadata.rule_id,
                    path: target_path,
                    reason: *reason,
                    pruned: *pruned,
                });
            }
            OwnedRuleMeasureProgressEvent::ScanCacheWriteSkipped => {
                progress(PlanProgressEvent::ScanCacheWriteSkipped {
                    rule_id: &metadata.rule_id,
                    path: target_path,
                });
            }
        }
        ensure_not_cancelled(cancellation)?;
    }

    Ok(())
}

impl From<PathMeasureProgressEvent<'_>> for OwnedRuleMeasureProgressEvent {
    fn from(event: PathMeasureProgressEvent<'_>) -> Self {
        match event {
            PathMeasureProgressEvent::Scan(ScanProgressEvent::FileMeasured {
                path,
                file_size,
                files_scanned,
                bytes_scanned,
            }) => Self::FileMeasured {
                path: path.to_path_buf(),
                file_size,
                files_scanned,
                bytes_scanned,
            },
            PathMeasureProgressEvent::ScanCacheHit { report } => Self::ScanCacheHit {
                estimated_bytes: report.bytes_scanned,
            },
            PathMeasureProgressEvent::ScanCacheMiss { reason, pruned } => {
                Self::ScanCacheMiss { reason, pruned }
            }
            PathMeasureProgressEvent::ScanCacheWriteSkipped => Self::ScanCacheWriteSkipped,
        }
    }
}

fn ensure_not_cancelled(cancellation: &ScanCancellationToken) -> Result<()> {
    if cancellation.is_cancelled() {
        Err(RebeccaError::OperationCancelled(
            "scan was cancelled".to_string(),
        ))
    } else {
        Ok(())
    }
}

fn with_rule_metadata(target: CleanupTarget, rule: &RuleDefinition) -> CleanupTarget {
    target
        .with_restore_hint(rule.restore_hint.clone())
        .with_warnings(rule.warnings.clone())
}

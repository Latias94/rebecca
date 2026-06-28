use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::app_leftovers::derive_app_leftover_candidates;
use crate::applications::{ApplicationDiscovery, NoopApplicationDiscovery};
use crate::config::AppStorageEntry;
use crate::discovery::{TargetResolution, resolve_rule_target_with_applications};
use crate::environment::{Environment, SystemEnvironment};
use crate::error::{RebeccaError, Result};
use crate::model::{CleanupWorkflow, PlanRequest, RuleDefinition};
use crate::plan::{CleanupPlan, CleanupTarget, CleanupTargetIssueReason};
use crate::project_artifacts::{
    ProjectArtifactScanOptions, discover_project_artifacts, project_artifact_matches_selectors,
    validate_project_artifact_selectors,
};
use crate::protection::ProtectionPolicy;
use crate::safety::{PathDisposition, assess_existing_path_with_policy};
use crate::scan::{ScanCancellationToken, run_scoped_scan};
use crate::scan_cache::{ScanCacheMiss, ScanCachePolicy, ScanCacheStore};
use rayon::prelude::*;

mod measure;

use measure::{
    PathMeasureProgressEvent, app_leftover_rule_id, app_leftover_skipped_target, dedupe_key,
    emit_measured_target_progress, emit_target_finished, finalize_plan,
    measure_app_leftover_candidate, measure_path_with_optional_scan_cache,
    measure_project_artifact_candidate, prune_scan_cache,
};

#[derive(Debug, Clone, Copy)]
pub enum PlanProgressEvent<'a> {
    TargetScanning {
        rule_id: &'a str,
        path: &'a Path,
    },
    TargetFinished {
        rule_id: &'a str,
        path: &'a Path,
        status: crate::TargetStatus,
        estimated_bytes: u64,
    },
    FileMeasured {
        rule_id: &'a str,
        target_path: &'a Path,
        path: &'a Path,
        file_size: u64,
        files_scanned: u64,
        bytes_scanned: u64,
    },
    ScanCacheHit {
        rule_id: &'a str,
        path: &'a Path,
        estimated_bytes: u64,
    },
    ScanCacheMiss {
        rule_id: &'a str,
        path: &'a Path,
        reason: ScanCacheMiss,
    },
    ScanCacheWriteSkipped {
        rule_id: &'a str,
        path: &'a Path,
    },
}

#[derive(Debug, Clone, Copy)]
pub struct PlanBuildContext<'a> {
    cancellation: &'a ScanCancellationToken,
    scan_cache: Option<&'a ScanCacheStore>,
    scan_cache_policy: ScanCachePolicy,
    protection_policy: ProtectionPolicy<'a>,
}

impl<'a> PlanBuildContext<'a> {
    pub fn new(cancellation: &'a ScanCancellationToken) -> Self {
        Self {
            cancellation,
            scan_cache: None,
            scan_cache_policy: ScanCachePolicy::default(),
            protection_policy: ProtectionPolicy::new(),
        }
    }

    pub fn with_scan_cache(mut self, scan_cache: &'a ScanCacheStore) -> Self {
        self.scan_cache = Some(scan_cache);
        self
    }

    pub fn with_scan_cache_policy(mut self, scan_cache_policy: ScanCachePolicy) -> Self {
        self.scan_cache_policy = scan_cache_policy;
        self
    }

    pub fn with_protected_storage(mut self, protected_storage: &'a [AppStorageEntry]) -> Self {
        self.protection_policy = self
            .protection_policy
            .with_protected_storage(protected_storage);
        self
    }

    pub fn with_protected_paths(mut self, protected_paths: &'a [PathBuf]) -> Self {
        self.protection_policy = self.protection_policy.with_protected_paths(protected_paths);
        self
    }

    pub fn cancellation(&self) -> &'a ScanCancellationToken {
        self.cancellation
    }

    pub fn scan_cache(&self) -> Option<&'a ScanCacheStore> {
        self.scan_cache
    }

    pub fn scan_cache_policy(&self) -> ScanCachePolicy {
        self.scan_cache_policy
    }

    pub fn protection_policy(&self) -> ProtectionPolicy<'a> {
        self.protection_policy
    }
}

pub fn build_cleanup_plan(request: &PlanRequest, rules: &[RuleDefinition]) -> Result<CleanupPlan> {
    build_cleanup_plan_with_progress(request, rules, |_| {})
}

pub fn build_cleanup_plan_with_progress<F>(
    request: &PlanRequest,
    rules: &[RuleDefinition],
    progress: F,
) -> Result<CleanupPlan>
where
    F: for<'a> FnMut(PlanProgressEvent<'a>),
{
    build_cleanup_plan_with_environment_and_progress_and_cancellation(
        request,
        rules,
        &SystemEnvironment,
        &ScanCancellationToken::new(),
        progress,
    )
}

pub fn build_cleanup_plan_with_environment(
    request: &PlanRequest,
    rules: &[RuleDefinition],
    env: &impl Environment,
) -> Result<CleanupPlan> {
    build_cleanup_plan_with_environment_and_progress(request, rules, env, |_| {})
}

pub fn build_cleanup_plan_with_environment_and_progress<E, F>(
    request: &PlanRequest,
    rules: &[RuleDefinition],
    env: &E,
    progress: F,
) -> Result<CleanupPlan>
where
    E: Environment,
    F: for<'a> FnMut(PlanProgressEvent<'a>),
{
    build_cleanup_plan_with_environment_and_progress_and_cancellation(
        request,
        rules,
        env,
        &ScanCancellationToken::new(),
        progress,
    )
}

pub fn build_cleanup_plan_with_environment_and_progress_and_cancellation<E, F>(
    request: &PlanRequest,
    rules: &[RuleDefinition],
    env: &E,
    cancellation: &ScanCancellationToken,
    progress: F,
) -> Result<CleanupPlan>
where
    E: Environment,
    F: for<'a> FnMut(PlanProgressEvent<'a>),
{
    build_cleanup_plan_with_environment_applications_progress_and_cancellation(
        request,
        rules,
        env,
        &NoopApplicationDiscovery::new(),
        cancellation,
        progress,
    )
}

pub fn build_cleanup_plan_with_environment_and_applications<E, A>(
    request: &PlanRequest,
    rules: &[RuleDefinition],
    env: &E,
    applications: &A,
) -> Result<CleanupPlan>
where
    E: Environment,
    A: ApplicationDiscovery + ?Sized,
{
    build_cleanup_plan_with_context(
        request,
        rules,
        env,
        applications,
        PlanBuildContext::new(&ScanCancellationToken::new()),
        |_| {},
    )
}

pub fn build_cleanup_plan_with_environment_applications_progress_and_cancellation<E, A, F>(
    request: &PlanRequest,
    rules: &[RuleDefinition],
    env: &E,
    applications: &A,
    cancellation: &ScanCancellationToken,
    progress: F,
) -> Result<CleanupPlan>
where
    E: Environment,
    A: ApplicationDiscovery + ?Sized,
    F: for<'a> FnMut(PlanProgressEvent<'a>),
{
    build_cleanup_plan_with_context(
        request,
        rules,
        env,
        applications,
        PlanBuildContext::new(cancellation),
        progress,
    )
}

pub fn build_cleanup_plan_with_context<E, A, F>(
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
    if request.workflow == CleanupWorkflow::AppLeftovers {
        return build_app_leftover_plan_with_context(request, env, applications, context, progress);
    }
    if request.workflow == CleanupWorkflow::ProjectArtifacts {
        return build_project_artifact_plan_with_context(request, context, progress);
    }

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
                                PathMeasureProgressEvent::ScanCacheMiss { reason } => {
                                    progress(PlanProgressEvent::ScanCacheMiss {
                                        rule_id: &rule.id,
                                        path: &expanded,
                                        reason,
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
                            Ok(report) => {
                                let size = report.bytes_scanned;
                                let target = CleanupTarget::allowed(
                                    rule.id.clone(),
                                    expanded,
                                    size,
                                    request.mode,
                                );
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

    prune_scan_cache(context);
    Ok(finalize_plan(request.clone(), candidates))
}

pub fn build_project_artifact_plan_with_context<F>(
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
    let artifacts = discover_project_artifacts(&scan_options, context.cancellation())?;
    let filtered_artifacts = artifacts
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

    prune_scan_cache(context);
    Ok(finalize_plan(request.clone(), plan_candidates))
}

pub fn build_app_leftover_plan_with_context<E, A, F>(
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
            .map(|leftover| measure_app_leftover_candidate(leftover, request.mode, context))
            .collect::<Vec<_>>()
    });

    let mut candidates = duplicate_targets;
    for measured in measured_targets {
        let measured = measured?;
        emit_measured_target_progress(&mut progress, &measured);
        emit_target_finished(&mut progress, &measured.target);
        candidates.push(measured.target);
    }

    prune_scan_cache(context);
    Ok(finalize_plan(request.clone(), candidates))
}

fn with_rule_restore_hint(target: CleanupTarget, rule: &RuleDefinition) -> CleanupTarget {
    target.with_restore_hint(rule.restore_hint.clone())
}

pub fn validate_rule_catalog(rules: &[RuleDefinition]) -> Result<()> {
    let mut ids = BTreeSet::new();
    let mut target_specs = BTreeMap::<String, String>::new();

    for rule in rules {
        if rule.id.trim().is_empty() {
            return Err(RebeccaError::RuleCatalogInvalid(
                "rule id cannot be empty".to_string(),
            ));
        }

        if !ids.insert(rule.id.clone()) {
            return Err(RebeccaError::RuleCatalogInvalid(format!(
                "duplicate rule id: {}",
                rule.id
            )));
        }

        if rule.category.trim().is_empty() {
            return Err(RebeccaError::RuleCatalogInvalid(format!(
                "rule {} must define a category",
                rule.id
            )));
        }

        if rule.name.trim().is_empty() {
            return Err(RebeccaError::RuleCatalogInvalid(format!(
                "rule {} must define a name",
                rule.id
            )));
        }

        if rule.provenance.license.trim().is_empty() {
            return Err(RebeccaError::RuleCatalogInvalid(format!(
                "rule {} is missing provenance license",
                rule.id
            )));
        }

        if rule.provenance.notes.trim().is_empty() {
            return Err(RebeccaError::RuleCatalogInvalid(format!(
                "rule {} is missing provenance notes",
                rule.id
            )));
        }

        if rule.path_templates.is_empty() {
            return Err(RebeccaError::RuleCatalogInvalid(format!(
                "rule {} must define at least one path template",
                rule.id
            )));
        }

        for spec in &rule.path_templates {
            if spec
                .placeholder_path()
                .as_os_str()
                .to_string_lossy()
                .trim()
                .is_empty()
            {
                return Err(RebeccaError::RuleCatalogInvalid(format!(
                    "rule {} contains an empty target path",
                    rule.id
                )));
            }

            let key = spec.dedupe_key(rule.platform);
            if let Some(previous_rule) = target_specs.insert(key.clone(), rule.id.clone()) {
                return Err(RebeccaError::RuleCatalogInvalid(format!(
                    "duplicate target spec {key} used by rules {previous_rule} and {}",
                    rule.id
                )));
            }
        }
    }

    Ok(())
}

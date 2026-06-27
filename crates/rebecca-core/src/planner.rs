use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::app_leftovers::{AppLeftoverCandidate, derive_app_leftover_candidates};
use crate::applications::{ApplicationDiscovery, NoopApplicationDiscovery};
use crate::config::AppStorageEntry;
use crate::discovery::{TargetResolution, resolve_rule_target_with_applications};
use crate::environment::{Environment, SystemEnvironment};
use crate::error::{RebeccaError, Result};
use crate::model::{CleanupWorkflow, PlanRequest, Platform, RuleDefinition};
use crate::plan::{CleanupPlan, CleanupTarget, CleanupTargetIssueReason};
use crate::project_artifacts::{
    ProjectArtifactCandidate, ProjectArtifactScanOptions, discover_project_artifacts,
    project_artifact_matches_selectors, recently_modified_reason,
    validate_project_artifact_selectors,
};
use crate::protection::{AppLeftoverPathDisposition, ProtectionPolicy};
use crate::safety::{PathDisposition, assess_existing_path_with_policy};
use crate::scan::{
    ScanCancellationToken, ScanProgressEvent, ScanReport, measure_path_with_progress,
};
use crate::scan_cache::{ScanCacheLookup, ScanCacheMiss, ScanCachePolicy, ScanCacheStore};
use rayon::prelude::*;

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
                                    ScanProgressEvent::FileMeasured {
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

    candidates.sort_by(|left, right| {
        left.rule_id
            .cmp(&right.rule_id)
            .then_with(|| left.path.cmp(&right.path))
    });

    let mut plan = CleanupPlan::empty(request.clone());
    plan.targets = candidates;
    plan.recompute_summary();
    Ok(plan)
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

    let mut candidates = filtered_artifacts
        .into_par_iter()
        .map(|artifact| {
            measure_project_artifact_candidate(
                artifact,
                request.mode,
                request.project_artifact_min_age_days,
                context,
            )
        })
        .collect::<Vec<_>>();

    let mut plan_candidates = Vec::with_capacity(candidates.len());
    for measured in candidates {
        let measured = measured?;
        emit_measured_target_progress(&mut progress, &measured);
        emit_target_finished(&mut progress, &measured.target);
        plan_candidates.push(measured.target);
    }

    plan_candidates.sort_by(|left, right| {
        left.rule_id
            .cmp(&right.rule_id)
            .then_with(|| left.path.cmp(&right.path))
    });

    let mut plan = CleanupPlan::empty(request.clone());
    plan.targets = plan_candidates;
    plan.recompute_summary();
    Ok(plan)
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

    let mut measured_targets = unique_leftovers
        .into_par_iter()
        .map(|leftover| measure_app_leftover_candidate(leftover, request.mode, context))
        .collect::<Vec<_>>();

    let mut candidates = duplicate_targets;
    for measured in measured_targets {
        let measured = measured?;
        emit_measured_target_progress(&mut progress, &measured);
        emit_target_finished(&mut progress, &measured.target);
        candidates.push(measured.target);
    }

    candidates.sort_by(|left, right| {
        left.rule_id
            .cmp(&right.rule_id)
            .then_with(|| left.path.cmp(&right.path))
    });

    let mut plan = CleanupPlan::empty(request.clone());
    plan.targets = candidates;
    plan.recompute_summary();
    Ok(plan)
}

fn project_artifact_allowed_target(
    artifact: &ProjectArtifactCandidate,
    estimated_bytes: u64,
    mode: crate::DeleteMode,
) -> CleanupTarget {
    CleanupTarget::allowed(
        artifact.definition.rule_id,
        artifact.path.clone(),
        estimated_bytes,
        mode,
    )
    .with_restore_hint(Some(artifact.definition.restore_hint.to_string()))
    .with_deletion_style(crate::plan::CleanupTargetDeletionStyle::DeleteWholePath)
    .with_modified_at_unix_seconds(artifact.modified_at_unix_seconds)
}

fn project_artifact_skipped_target(
    artifact: &ProjectArtifactCandidate,
    mode: crate::DeleteMode,
    reason_code: CleanupTargetIssueReason,
    reason: impl Into<String>,
) -> CleanupTarget {
    CleanupTarget::skipped_with_reason_code(
        artifact.definition.rule_id,
        artifact.path.clone(),
        mode,
        reason_code,
        reason,
    )
    .with_restore_hint(Some(artifact.definition.restore_hint.to_string()))
    .with_deletion_style(crate::plan::CleanupTargetDeletionStyle::DeleteWholePath)
    .with_modified_at_unix_seconds(artifact.modified_at_unix_seconds)
}

fn project_artifact_blocked_target(
    artifact: &ProjectArtifactCandidate,
    mode: crate::DeleteMode,
    reason_code: CleanupTargetIssueReason,
    reason: impl Into<String>,
) -> CleanupTarget {
    CleanupTarget::blocked_with_reason_code(
        artifact.definition.rule_id,
        artifact.path.clone(),
        mode,
        reason_code,
        reason,
    )
    .with_restore_hint(Some(artifact.definition.restore_hint.to_string()))
    .with_deletion_style(crate::plan::CleanupTargetDeletionStyle::DeleteWholePath)
    .with_modified_at_unix_seconds(artifact.modified_at_unix_seconds)
}

fn project_artifact_failed_target(
    artifact: &ProjectArtifactCandidate,
    mode: crate::DeleteMode,
    reason_code: CleanupTargetIssueReason,
    reason: impl Into<String>,
) -> CleanupTarget {
    CleanupTarget::failed_with_reason_code(
        artifact.definition.rule_id,
        artifact.path.clone(),
        mode,
        0,
        reason_code,
        reason,
    )
    .with_restore_hint(Some(artifact.definition.restore_hint.to_string()))
    .with_deletion_style(crate::plan::CleanupTargetDeletionStyle::DeleteWholePath)
    .with_modified_at_unix_seconds(artifact.modified_at_unix_seconds)
}

fn app_leftover_allowed_target(
    leftover: &AppLeftoverCandidate,
    estimated_bytes: u64,
    mode: crate::DeleteMode,
) -> CleanupTarget {
    CleanupTarget::allowed(
        app_leftover_rule_id(leftover),
        leftover.path.clone(),
        estimated_bytes,
        mode,
    )
    .with_restore_hint(Some(app_leftover_restore_hint(leftover)))
    .with_deletion_style(crate::plan::CleanupTargetDeletionStyle::PreserveRootContents)
    .with_modified_at_unix_seconds(leftover.modified_at_unix_seconds)
}

fn app_leftover_skipped_target(
    leftover: &AppLeftoverCandidate,
    mode: crate::DeleteMode,
    reason_code: CleanupTargetIssueReason,
    reason: impl Into<String>,
) -> CleanupTarget {
    CleanupTarget::skipped_with_reason_code(
        app_leftover_rule_id(leftover),
        leftover.path.clone(),
        mode,
        reason_code,
        reason,
    )
    .with_restore_hint(Some(app_leftover_restore_hint(leftover)))
    .with_deletion_style(crate::plan::CleanupTargetDeletionStyle::PreserveRootContents)
    .with_modified_at_unix_seconds(leftover.modified_at_unix_seconds)
}

fn app_leftover_blocked_target(
    leftover: &AppLeftoverCandidate,
    mode: crate::DeleteMode,
    reason_code: CleanupTargetIssueReason,
    reason: impl Into<String>,
) -> CleanupTarget {
    CleanupTarget::blocked_with_reason_code(
        app_leftover_rule_id(leftover),
        leftover.path.clone(),
        mode,
        reason_code,
        reason,
    )
    .with_restore_hint(Some(app_leftover_restore_hint(leftover)))
    .with_deletion_style(crate::plan::CleanupTargetDeletionStyle::PreserveRootContents)
    .with_modified_at_unix_seconds(leftover.modified_at_unix_seconds)
}

fn app_leftover_failed_target(
    leftover: &AppLeftoverCandidate,
    mode: crate::DeleteMode,
    reason_code: CleanupTargetIssueReason,
    reason: impl Into<String>,
) -> CleanupTarget {
    CleanupTarget::failed_with_reason_code(
        app_leftover_rule_id(leftover),
        leftover.path.clone(),
        mode,
        0,
        reason_code,
        reason,
    )
    .with_restore_hint(Some(app_leftover_restore_hint(leftover)))
    .with_deletion_style(crate::plan::CleanupTargetDeletionStyle::PreserveRootContents)
    .with_modified_at_unix_seconds(leftover.modified_at_unix_seconds)
}

struct MeasuredTarget {
    target: CleanupTarget,
    file_progress: Vec<MeasuredFileProgress>,
    scan_cache_event: Option<MeasuredScanCacheEvent>,
}

struct MeasuredFileProgress {
    path: PathBuf,
    file_size: u64,
    files_scanned: u64,
    bytes_scanned: u64,
}

enum MeasuredScanCacheEvent {
    Hit { estimated_bytes: u64 },
    Miss { reason: ScanCacheMiss },
    WriteSkipped,
}

fn measure_project_artifact_candidate(
    artifact: ProjectArtifactCandidate,
    mode: crate::DeleteMode,
    min_age_days: u64,
    context: PlanBuildContext<'_>,
) -> Result<MeasuredTarget> {
    measure_project_candidate(
        artifact,
        mode,
        min_age_days,
        context,
        project_artifact_allowed_target,
        project_artifact_skipped_target,
        project_artifact_blocked_target,
        project_artifact_failed_target,
    )
}

fn measure_app_leftover_candidate(
    leftover: AppLeftoverCandidate,
    mode: crate::DeleteMode,
    context: PlanBuildContext<'_>,
) -> Result<MeasuredTarget> {
    measure_project_candidate(
        leftover,
        mode,
        0,
        context,
        app_leftover_allowed_target,
        app_leftover_skipped_target,
        app_leftover_blocked_target,
        app_leftover_failed_target,
    )
}

fn measure_project_candidate<T, Allowed, Skipped, Blocked, Failed>(
    candidate: T,
    mode: crate::DeleteMode,
    min_age_days: u64,
    context: PlanBuildContext<'_>,
    allowed_target: Allowed,
    skipped_target: Skipped,
    blocked_target: Blocked,
    failed_target: Failed,
) -> Result<MeasuredTarget>
where
    T: CandidatePath,
    Allowed: FnOnce(&T, u64, crate::DeleteMode) -> CleanupTarget,
    Skipped: FnOnce(&T, crate::DeleteMode, CleanupTargetIssueReason, String) -> CleanupTarget,
    Blocked: FnOnce(&T, crate::DeleteMode, CleanupTargetIssueReason, String) -> CleanupTarget,
    Failed: FnOnce(&T, crate::DeleteMode, CleanupTargetIssueReason, String) -> CleanupTarget,
{
    match T::assess(&candidate, context.protection_policy()) {
        CandidateDisposition::Allowed => {
            if min_age_days > 0
                && let Some(reason) = T::recently_modified_reason(&candidate, min_age_days)
            {
                let target = skipped_target(
                    &candidate,
                    mode,
                    CleanupTargetIssueReason::ProjectArtifactRecentlyModified,
                    reason,
                );
                return Ok(MeasuredTarget {
                    file_progress: Vec::new(),
                    scan_cache_event: None,
                    target,
                });
            }

            let mut file_progress = Vec::new();
            let mut scan_cache_event = None;
            let report =
                measure_path_with_optional_scan_cache(T::path(&candidate), context, |event| {
                    match event {
                        PathMeasureProgressEvent::Scan(ScanProgressEvent::FileMeasured {
                            path,
                            file_size,
                            files_scanned,
                            bytes_scanned,
                        }) => {
                            file_progress.push(MeasuredFileProgress {
                                path: path.to_path_buf(),
                                file_size,
                                files_scanned,
                                bytes_scanned,
                            });
                        }
                        PathMeasureProgressEvent::ScanCacheHit { report } => {
                            scan_cache_event = Some(MeasuredScanCacheEvent::Hit {
                                estimated_bytes: report.bytes_scanned,
                            });
                        }
                        PathMeasureProgressEvent::ScanCacheMiss { reason } => {
                            scan_cache_event = Some(MeasuredScanCacheEvent::Miss { reason });
                        }
                        PathMeasureProgressEvent::ScanCacheWriteSkipped => {
                            scan_cache_event = Some(MeasuredScanCacheEvent::WriteSkipped);
                        }
                    }
                })?;

            let target = allowed_target(&candidate, report.bytes_scanned, mode);
            Ok(MeasuredTarget {
                target,
                file_progress,
                scan_cache_event,
            })
        }
        CandidateDisposition::Skipped(reason) => Ok(MeasuredTarget {
            target: skipped_target(
                &candidate,
                mode,
                CleanupTargetIssueReason::SafetyPolicySkipped,
                reason,
            ),
            file_progress: Vec::new(),
            scan_cache_event: None,
        }),
        CandidateDisposition::Blocked(reason) => Ok(MeasuredTarget {
            target: blocked_target(
                &candidate,
                mode,
                CleanupTargetIssueReason::SafetyPolicyBlocked,
                reason,
            ),
            file_progress: Vec::new(),
            scan_cache_event: None,
        }),
    }
}

fn emit_measured_target_progress<F>(progress: &mut F, measured: &MeasuredTarget)
where
    F: for<'a> FnMut(PlanProgressEvent<'a>),
{
    if let Some(event) = &measured.scan_cache_event {
        match event {
            MeasuredScanCacheEvent::Hit { estimated_bytes } => {
                progress(PlanProgressEvent::ScanCacheHit {
                    rule_id: &measured.target.rule_id,
                    path: &measured.target.path,
                    estimated_bytes: *estimated_bytes,
                })
            }
            MeasuredScanCacheEvent::Miss { reason } => progress(PlanProgressEvent::ScanCacheMiss {
                rule_id: &measured.target.rule_id,
                path: &measured.target.path,
                reason: *reason,
            }),
            MeasuredScanCacheEvent::WriteSkipped => {
                progress(PlanProgressEvent::ScanCacheWriteSkipped {
                    rule_id: &measured.target.rule_id,
                    path: &measured.target.path,
                })
            }
        }
    }

    for event in &measured.file_progress {
        progress(PlanProgressEvent::FileMeasured {
            rule_id: &measured.target.rule_id,
            target_path: &measured.target.path,
            path: event.path.as_path(),
            file_size: event.file_size,
            files_scanned: event.files_scanned,
            bytes_scanned: event.bytes_scanned,
        });
    }
}

enum CandidateDisposition {
    Allowed,
    Skipped(String),
    Blocked(String),
}

trait CandidatePath {
    fn path(&self) -> &Path;
    fn assess(candidate: &Self, policy: ProtectionPolicy<'_>) -> CandidateDisposition;
    fn recently_modified_reason(candidate: &Self, min_age_days: u64) -> Option<String>;
}

impl CandidatePath for ProjectArtifactCandidate {
    fn path(&self) -> &Path {
        self.path.as_path()
    }

    fn assess(candidate: &Self, policy: ProtectionPolicy<'_>) -> CandidateDisposition {
        match assess_existing_path_with_policy(&candidate.path, policy) {
            PathDisposition::Allowed => CandidateDisposition::Allowed,
            PathDisposition::Skipped(reason) => CandidateDisposition::Skipped(reason),
            PathDisposition::Blocked(reason) => CandidateDisposition::Blocked(reason),
        }
    }

    fn recently_modified_reason(candidate: &Self, min_age_days: u64) -> Option<String> {
        recently_modified_reason(candidate.path(), min_age_days)
    }
}

impl CandidatePath for AppLeftoverCandidate {
    fn path(&self) -> &Path {
        self.path.as_path()
    }

    fn assess(candidate: &Self, policy: ProtectionPolicy<'_>) -> CandidateDisposition {
        match policy.assess_existing_app_leftover_path(&candidate.path) {
            AppLeftoverPathDisposition::Allowed => CandidateDisposition::Allowed,
            AppLeftoverPathDisposition::Missing => {
                CandidateDisposition::Skipped("path does not exist".to_string())
            }
            AppLeftoverPathDisposition::Blocked(reason) => CandidateDisposition::Blocked(reason),
        }
    }

    fn recently_modified_reason(_candidate: &Self, _min_age_days: u64) -> Option<String> {
        None
    }
}

fn app_leftover_rule_id(leftover: &AppLeftoverCandidate) -> &'static str {
    match leftover.source {
        crate::app_leftovers::AppLeftoverSource::LocalAppData => "windows.app-leftover-local-cache",
        crate::app_leftovers::AppLeftoverSource::RoamingAppData => {
            "windows.app-leftover-roaming-cache"
        }
        crate::app_leftovers::AppLeftoverSource::LocalLowAppData => {
            "windows.app-leftover-local-low-cache"
        }
    }
}

fn app_leftover_restore_hint(leftover: &AppLeftoverCandidate) -> String {
    format!(
        "{} {} cache data is rebuildable.",
        leftover.app.display_name(),
        leftover.source.label()
    )
}

fn measure_path_with_optional_scan_cache<F>(
    path: &Path,
    context: PlanBuildContext<'_>,
    mut progress: F,
) -> Result<ScanReport>
where
    F: for<'a> FnMut(PathMeasureProgressEvent<'a>),
{
    if context.cancellation().is_cancelled() {
        return Err(RebeccaError::OperationCancelled(
            "scan was cancelled".to_string(),
        ));
    }

    let cacheable_target = context.scan_cache().is_some() && is_cacheable_scan_target(path);
    if cacheable_target {
        if let Some(store) = context.scan_cache() {
            match store.load_with_policy(path, context.scan_cache_policy()) {
                ScanCacheLookup::Hit(report) => {
                    progress(PathMeasureProgressEvent::ScanCacheHit { report });
                    return Ok(report);
                }
                ScanCacheLookup::Miss(reason) => {
                    progress(PathMeasureProgressEvent::ScanCacheMiss { reason });
                }
            }
        }
    }

    let report = measure_path_with_progress(path, context.cancellation(), |event| {
        progress(PathMeasureProgressEvent::Scan(event));
    })?;
    if cacheable_target {
        if let Some(store) = context.scan_cache() {
            if let Err(err) = store.store(path, report) {
                tracing::debug!(
                    path = %path.display(),
                    error = %err,
                    "scan cache write skipped"
                );
                progress(PathMeasureProgressEvent::ScanCacheWriteSkipped);
            }
        }
    }

    Ok(report)
}

enum PathMeasureProgressEvent<'a> {
    Scan(ScanProgressEvent<'a>),
    ScanCacheHit { report: ScanReport },
    ScanCacheMiss { reason: ScanCacheMiss },
    ScanCacheWriteSkipped,
}

fn is_cacheable_scan_target(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|metadata| metadata.is_file() || metadata.is_dir())
        .unwrap_or(false)
}

fn emit_target_finished<F>(progress: &mut F, target: &CleanupTarget)
where
    F: for<'a> FnMut(PlanProgressEvent<'a>),
{
    progress(PlanProgressEvent::TargetFinished {
        rule_id: &target.rule_id,
        path: &target.path,
        status: target.status,
        estimated_bytes: target.estimated_bytes,
    });
}

fn dedupe_key(path: &Path, _platform: Platform) -> String {
    let mut normalized = path.as_os_str().to_string_lossy().replace('\\', "/");

    while normalized.ends_with('/') && normalized.len() > 3 {
        normalized.pop();
    }

    normalized.to_ascii_lowercase()
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

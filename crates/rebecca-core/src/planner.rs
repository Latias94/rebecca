use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::applications::{ApplicationDiscovery, NoopApplicationDiscovery};
use crate::discovery::{TargetResolution, resolve_rule_target_with_applications};
use crate::environment::{Environment, SystemEnvironment};
use crate::error::{RebeccaError, Result};
use crate::model::{PlanRequest, Platform, RuleDefinition};
use crate::plan::{CleanupPlan, CleanupTarget};
use crate::safety::{PathDisposition, assess_existing_path};
use crate::scan::{
    ScanCancellationToken, ScanProgressEvent, ScanReport, measure_path_with_progress,
};
use crate::scan_cache::{ScanCacheLookup, ScanCacheMiss, ScanCacheStore};

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
}

impl<'a> PlanBuildContext<'a> {
    pub fn new(cancellation: &'a ScanCancellationToken) -> Self {
        Self {
            cancellation,
            scan_cache: None,
        }
    }

    pub fn with_scan_cache(mut self, scan_cache: &'a ScanCacheStore) -> Self {
        self.scan_cache = Some(scan_cache);
        self
    }

    pub fn cancellation(&self) -> &'a ScanCancellationToken {
        self.cancellation
    }

    pub fn scan_cache(&self) -> Option<&'a ScanCacheStore> {
        self.scan_cache
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
                    CleanupTarget::skipped(
                        rule.id.clone(),
                        spec.placeholder_path(),
                        request.mode,
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
                            CleanupTarget::skipped(
                                rule.id.clone(),
                                spec.placeholder_path(),
                                request.mode,
                                reason,
                            ),
                            rule,
                        ));
                        continue;
                    }
                    Err(err) => {
                        candidates.push(with_rule_restore_hint(
                            CleanupTarget::blocked(
                                rule.id.clone(),
                                spec.placeholder_path(),
                                request.mode,
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
                        CleanupTarget::skipped(
                            rule.id.clone(),
                            expanded,
                            request.mode,
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

                match assess_existing_path(&expanded) {
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
                                let target = CleanupTarget::failed(
                                    rule.id.clone(),
                                    expanded,
                                    request.mode,
                                    0,
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
                            CleanupTarget::skipped(rule.id.clone(), expanded, request.mode, reason),
                            rule,
                        );
                        emit_target_finished(&mut progress, &target);
                        candidates.push(target);
                    }
                    PathDisposition::Blocked(reason) => {
                        let target = with_rule_restore_hint(
                            CleanupTarget::blocked(rule.id.clone(), expanded, request.mode, reason),
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
            match store.load(path) {
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

fn dedupe_key(path: &Path, platform: Platform) -> String {
    let mut normalized = path.as_os_str().to_string_lossy().replace('\\', "/");

    while normalized.ends_with('/') && normalized.len() > 3 {
        normalized.pop();
    }

    if platform == Platform::Windows {
        normalized.to_ascii_lowercase()
    } else {
        normalized
    }
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

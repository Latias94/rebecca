use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::applications::{ApplicationDiscovery, NoopApplicationDiscovery};
use crate::discovery::{TargetResolution, resolve_rule_target_with_applications};
use crate::environment::{Environment, SystemEnvironment};
use crate::error::{RebeccaError, Result};
use crate::model::{PlanRequest, Platform, RuleDefinition, RuleTargetSpec, SafetyLevel};
use crate::plan::{CleanupPlan, CleanupTarget};
use crate::safety::{PathDisposition, assess_existing_path};
use crate::scan::{ScanCancellationToken, ScanProgressEvent, measure_path_size_with_progress};

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
    build_cleanup_plan_with_environment_applications_progress_and_cancellation(
        request,
        rules,
        env,
        applications,
        &ScanCancellationToken::new(),
        |_| {},
    )
}

pub fn build_cleanup_plan_with_environment_applications_progress_and_cancellation<E, A, F>(
    request: &PlanRequest,
    rules: &[RuleDefinition],
    env: &E,
    applications: &A,
    cancellation: &ScanCancellationToken,
    mut progress: F,
) -> Result<CleanupPlan>
where
    E: Environment,
    A: ApplicationDiscovery + ?Sized,
    F: for<'a> FnMut(PlanProgressEvent<'a>),
{
    validate_selected_rule_ids(request, rules)?;

    let mut candidates = Vec::new();
    let mut seen_paths = BTreeSet::new();
    let selection = request.selection();

    for rule in rules {
        if rule.platform != request.platform {
            continue;
        }

        if !selection.matches_rule(rule) {
            continue;
        }

        if !safety_allowed(rule.safety_level, request) {
            for spec in &rule.path_templates {
                candidates.push(CleanupTarget::skipped(
                    rule.id.clone(),
                    spec_placeholder(spec),
                    request.mode,
                    format!(
                        "{} rule requires explicit opt-in",
                        safety_name(rule.safety_level)
                    ),
                ));
            }
            continue;
        }

        for spec in &rule.path_templates {
            let expanded_paths =
                match resolve_rule_target_with_applications(spec, env, applications) {
                    Ok(TargetResolution::Paths(paths)) => paths,
                    Ok(TargetResolution::Skipped(reason)) => {
                        candidates.push(CleanupTarget::skipped(
                            rule.id.clone(),
                            spec_placeholder(spec),
                            request.mode,
                            reason,
                        ));
                        continue;
                    }
                    Err(err) => {
                        candidates.push(CleanupTarget::blocked(
                            rule.id.clone(),
                            spec_placeholder(spec),
                            request.mode,
                            err.to_string(),
                        ));
                        continue;
                    }
                };

            for expanded in expanded_paths {
                let path_key = dedupe_key(&expanded, request.platform);
                if !seen_paths.insert(path_key) {
                    let target = CleanupTarget::skipped(
                        rule.id.clone(),
                        expanded,
                        request.mode,
                        "duplicate target path already covered",
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
                        match measure_path_size_with_progress(&expanded, cancellation, |event| {
                            match event {
                                ScanProgressEvent::FileMeasured {
                                    path,
                                    file_size,
                                    files_scanned,
                                    bytes_scanned,
                                } => {
                                    progress(PlanProgressEvent::FileMeasured {
                                        rule_id: &rule.id,
                                        target_path: &expanded,
                                        path,
                                        file_size,
                                        files_scanned,
                                        bytes_scanned,
                                    });
                                }
                            }
                        }) {
                            Ok(size) => {
                                let target = CleanupTarget::allowed(
                                    rule.id.clone(),
                                    expanded,
                                    size,
                                    request.mode,
                                );
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
                                emit_target_finished(&mut progress, &target);
                                candidates.push(target);
                            }
                        }
                    }
                    PathDisposition::Skipped(reason) => {
                        let target =
                            CleanupTarget::skipped(rule.id.clone(), expanded, request.mode, reason);
                        emit_target_finished(&mut progress, &target);
                        candidates.push(target);
                    }
                    PathDisposition::Blocked(reason) => {
                        let target =
                            CleanupTarget::blocked(rule.id.clone(), expanded, request.mode, reason);
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

fn validate_selected_rule_ids(request: &PlanRequest, rules: &[RuleDefinition]) -> Result<()> {
    for selected in &request.selected_rule_ids {
        let known = rules
            .iter()
            .any(|rule| rule.id.eq_ignore_ascii_case(selected));
        if !known {
            return Err(RebeccaError::InvalidRuleId(selected.clone()));
        }
    }

    Ok(())
}

fn safety_allowed(rule_level: SafetyLevel, request: &PlanRequest) -> bool {
    match rule_level {
        SafetyLevel::Safe => true,
        SafetyLevel::Moderate => request.allow_moderate || request.allow_risky,
        SafetyLevel::Risky | SafetyLevel::Dangerous => request.allow_risky,
    }
}

fn safety_name(level: SafetyLevel) -> &'static str {
    match level {
        SafetyLevel::Safe => "safe",
        SafetyLevel::Moderate => "moderate",
        SafetyLevel::Risky => "risky",
        SafetyLevel::Dangerous => "dangerous",
    }
}

fn spec_placeholder(spec: &RuleTargetSpec) -> PathBuf {
    match spec {
        RuleTargetSpec::Template(template) => PathBuf::from(template.raw()),
        RuleTargetSpec::ExactPath(path) => path.clone(),
        RuleTargetSpec::GlobTemplate(template) => PathBuf::from(template.raw()),
        RuleTargetSpec::SteamInstallTemplate(template) => PathBuf::from(template.raw()),
        RuleTargetSpec::SteamLibraryTemplate(template) => PathBuf::from(template.raw()),
    }
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
            let key = target_spec_key(rule.platform, spec);
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

fn target_spec_key(platform: Platform, spec: &RuleTargetSpec) -> String {
    let target = match spec {
        RuleTargetSpec::Template(template) => format!("template:{}", template.raw()),
        RuleTargetSpec::ExactPath(path) => format!("exact-path:{}", path.display()),
        RuleTargetSpec::GlobTemplate(template) => format!("glob-template:{}", template.raw()),
        RuleTargetSpec::SteamInstallTemplate(template) => {
            format!("steam-install-template:{}", template.raw())
        }
        RuleTargetSpec::SteamLibraryTemplate(template) => {
            format!("steam-library-template:{}", template.raw())
        }
    }
    .replace('\\', "/");

    if platform == Platform::Windows {
        format!("{platform:?}:{}", target.to_ascii_lowercase())
    } else {
        format!("{platform:?}:{target}")
    }
}

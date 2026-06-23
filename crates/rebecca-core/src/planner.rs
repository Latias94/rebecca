use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::environment::{Environment, SystemEnvironment};
use crate::error::{RebeccaError, Result};
use crate::model::{PlanRequest, Platform, RuleDefinition, RuleTargetSpec, SafetyLevel};
use crate::path_template::expand_rule_target;
use crate::plan::{CleanupPlan, CleanupTarget};
use crate::safety::{PathDisposition, assess_existing_path};
use crate::scan::measure_path_size;

pub fn build_cleanup_plan(request: &PlanRequest, rules: &[RuleDefinition]) -> Result<CleanupPlan> {
    build_cleanup_plan_with_environment(request, rules, &SystemEnvironment)
}

pub fn build_cleanup_plan_with_environment(
    request: &PlanRequest,
    rules: &[RuleDefinition],
    env: &impl Environment,
) -> Result<CleanupPlan> {
    validate_selected_rule_ids(request, rules)?;

    let mut candidates = Vec::new();
    let mut seen_paths = BTreeSet::new();

    for rule in rules {
        if rule.platform != request.platform {
            continue;
        }

        if !selected_rule(rule, request) {
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
            let expanded = match expand_rule_target(spec, env) {
                Ok(Some(path)) => path,
                Ok(None) => {
                    candidates.push(CleanupTarget::skipped(
                        rule.id.clone(),
                        spec_placeholder(spec),
                        request.mode,
                        "path template could not be resolved in the current environment",
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

            let path_key = dedupe_key(&expanded, request.platform);
            if !seen_paths.insert(path_key) {
                candidates.push(CleanupTarget::skipped(
                    rule.id.clone(),
                    expanded,
                    request.mode,
                    "duplicate target path already covered",
                ));
                continue;
            }

            match assess_existing_path(&expanded) {
                PathDisposition::Allowed => match measure_path_size(&expanded) {
                    Ok(size) => candidates.push(CleanupTarget::allowed(
                        rule.id.clone(),
                        expanded,
                        size,
                        request.mode,
                    )),
                    Err(err) => candidates.push(CleanupTarget::failed(
                        rule.id.clone(),
                        expanded,
                        request.mode,
                        0,
                        err.to_string(),
                    )),
                },
                PathDisposition::Skipped(reason) => candidates.push(CleanupTarget::skipped(
                    rule.id.clone(),
                    expanded,
                    request.mode,
                    reason,
                )),
                PathDisposition::Blocked(reason) => candidates.push(CleanupTarget::blocked(
                    rule.id.clone(),
                    expanded,
                    request.mode,
                    reason,
                )),
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

fn selected_rule(rule: &RuleDefinition, request: &PlanRequest) -> bool {
    let selected_category = request.selected_categories.is_empty()
        || request
            .selected_categories
            .iter()
            .any(|category| category.eq_ignore_ascii_case(&rule.category));

    let selected_id = request.selected_rule_ids.is_empty()
        || request
            .selected_rule_ids
            .iter()
            .any(|id| id.eq_ignore_ascii_case(&rule.id));

    selected_category && selected_id
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
    }
    .replace('\\', "/");

    if platform == Platform::Windows {
        format!("{platform:?}:{}", target.to_ascii_lowercase())
    } else {
        format!("{platform:?}:{target}")
    }
}

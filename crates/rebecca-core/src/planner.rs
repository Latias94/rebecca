use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::applications::{ApplicationDiscovery, NoopApplicationDiscovery};
use crate::config::AppStorageEntry;
use crate::environment::{Environment, SystemEnvironment};
use crate::error::{RebeccaError, Result};
use crate::model::{CleanupWorkflow, PlanRequest, RuleDefinition};
use crate::plan::CleanupPlan;
use crate::protection::ProtectionPolicy;
use crate::scan::ScanCancellationToken;
use crate::scan_cache::{ScanCacheMiss, ScanCachePolicy, ScanCachePruneReport, ScanCacheStore};

mod app_leftovers;
mod measure;
mod project_artifacts;
mod rules;

use app_leftovers::build_app_leftover_plan_with_context;
use project_artifacts::build_project_artifact_plan_with_context;
use rules::build_rule_plan_with_context;

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
        pruned: bool,
    },
    ScanCacheWriteSkipped {
        rule_id: &'a str,
        path: &'a Path,
    },
    ScanCachePruned {
        report: ScanCachePruneReport,
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
    progress: F,
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

    build_rule_plan_with_context(request, rules, env, applications, context, progress)
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

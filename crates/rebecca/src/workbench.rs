use std::path::PathBuf;

use anyhow::{Result, anyhow};
use rebecca::core::config::AppRuntimeConfig;
use rebecca::core::environment::SystemEnvironment;
use rebecca::core::executor::execute_cleanup_plan_parallel_with_policy_and_cancellation;
use rebecca::core::history::HistoryStore;
use rebecca::core::planner::{
    PlanBuildContext, PlanProgressEvent, build_cleanup_plan_with_context,
};
use rebecca::core::protection::ProtectionPolicy;
use rebecca::core::scan::ScanBackendKind;
use rebecca::core::scan_cache::ScanCacheStore;
use rebecca::core::{CleanupPlan, DeleteMode, PlanRequest, Platform};

use crate::runtime::CliRuntime;
use crate::trash_backend::recoverable_trash_backend;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CleanupWorkbenchRequest {
    pub(crate) selected_rule_ids: Vec<String>,
    pub(crate) allow_moderate: bool,
    pub(crate) allow_risky: bool,
    pub(crate) allowed_warnings: Vec<String>,
    pub(crate) scan_cache: bool,
    pub(crate) scan_backend: ScanBackendKind,
    pub(crate) exclude_paths: Vec<PathBuf>,
}

impl CleanupWorkbenchRequest {
    pub(crate) fn dry_run_plan_request(&self) -> PlanRequest {
        self.plan_request(DeleteMode::DryRun)
    }

    fn recoverable_delete_plan_request(&self) -> PlanRequest {
        self.plan_request(DeleteMode::RecoverableDelete)
    }

    fn plan_request(&self, mode: DeleteMode) -> PlanRequest {
        let mut request = PlanRequest::for_platform(Platform::current(), mode);
        request.selected_rule_ids = self.selected_rule_ids.clone();
        request.allow_moderate = self.allow_moderate;
        request.allow_risky = self.allow_risky;
        for warning in &self.allowed_warnings {
            request.add_allowed_warning(warning);
        }
        request
    }
}

pub(crate) fn preview_cleanup_plan(
    request: &CleanupWorkbenchRequest,
    runtime_config: &AppRuntimeConfig,
    runtime: &CliRuntime,
) -> Result<CleanupPlan> {
    preview_cleanup_plan_with_progress(request, runtime_config, runtime, |_| {})
}

pub(crate) fn preview_cleanup_plan_with_progress<F>(
    request: &CleanupWorkbenchRequest,
    runtime_config: &AppRuntimeConfig,
    runtime: &CliRuntime,
    progress: F,
) -> Result<CleanupPlan>
where
    F: for<'a> FnMut(PlanProgressEvent<'a>),
{
    build_plan(
        request.dry_run_plan_request(),
        request,
        runtime_config,
        runtime,
        progress,
    )
}

pub(crate) fn execute_recoverable_cleanup(
    request: &CleanupWorkbenchRequest,
    runtime_config: &AppRuntimeConfig,
    runtime: &CliRuntime,
) -> Result<CleanupPlan> {
    execute_recoverable_cleanup_with_progress(request, runtime_config, runtime, |_| {})
}

pub(crate) fn execute_recoverable_cleanup_with_progress<F>(
    request: &CleanupWorkbenchRequest,
    runtime_config: &AppRuntimeConfig,
    runtime: &CliRuntime,
    progress: F,
) -> Result<CleanupPlan>
where
    F: for<'a> FnMut(PlanProgressEvent<'a>),
{
    let mut plan = build_plan(
        request.recoverable_delete_plan_request(),
        request,
        runtime_config,
        runtime,
        progress,
    )?;
    if plan.summary.allowed_targets == 0 {
        return Ok(plan);
    }

    let safety_knowledge =
        rebecca::rules::builtin_safety_knowledge_for_platform(plan.request.platform)?;
    let protected_storage = runtime_config.app_paths.storage_entries();
    let protected_paths = merged_protected_paths(
        runtime_config.protected_paths.as_slice(),
        request.exclude_paths.as_slice(),
    )?;
    let mut execution_policy = ProtectionPolicy::new()
        .with_safety_knowledge(&safety_knowledge)
        .with_protected_storage(&protected_storage);
    if !protected_paths.is_empty() {
        execution_policy = execution_policy.with_protected_paths(&protected_paths);
    }

    let backend = recoverable_trash_backend();
    let mut execution_report = execute_cleanup_plan_parallel_with_policy_and_cancellation(
        &mut plan,
        &backend,
        execution_policy,
        runtime.cancellation(),
    )?;
    let history_append =
        HistoryStore::new(runtime_config.app_paths.history_file.clone()).append_plan_report(&plan);
    if let Some(warning) = history_append.warning {
        execution_report.push_warning(warning);
    }
    plan.execution_report = Some(execution_report);
    Ok(plan)
}

fn build_plan(
    plan_request: PlanRequest,
    request: &CleanupWorkbenchRequest,
    runtime_config: &AppRuntimeConfig,
    runtime: &CliRuntime,
    progress: impl for<'a> FnMut(PlanProgressEvent<'a>),
) -> Result<CleanupPlan> {
    let catalog = rebecca::rules::builtin_rules()?;
    let safety_knowledge =
        rebecca::rules::builtin_safety_knowledge_for_platform(plan_request.platform)?;
    let applications = crate::info::application_discovery();
    let protected_storage = runtime_config.app_paths.storage_entries();
    let protected_paths = merged_protected_paths(
        runtime_config.protected_paths.as_slice(),
        request.exclude_paths.as_slice(),
    )?;
    let scan_cache_store = request
        .scan_cache
        .then(|| ScanCacheStore::from_app_paths(&runtime_config.app_paths));
    let mut context = PlanBuildContext::new(runtime.cancellation())
        .with_scan_backend(request.scan_backend)
        .with_safety_knowledge(&safety_knowledge)
        .with_protected_storage(&protected_storage);
    if !protected_paths.is_empty() {
        context = context.with_protected_paths(&protected_paths);
    }
    if request.scan_cache {
        context = context.with_scan_cache_policy(runtime_config.scan_cache_policy);
        if let Some(store) = &scan_cache_store {
            context = context.with_scan_cache(store);
        }
    }

    build_cleanup_plan_with_context(
        &plan_request,
        &catalog,
        &SystemEnvironment,
        applications.as_ref(),
        context,
        progress,
    )
    .map_err(Into::into)
}

fn merged_protected_paths(config_paths: &[PathBuf], cli_paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut merged = Vec::with_capacity(config_paths.len() + cli_paths.len());
    for path in config_paths.iter().chain(cli_paths) {
        rebecca::core::config::validate_user_protected_path(path)
            .map_err(|message| anyhow!("invalid protected path {}: {message}", path.display()))?;
        if merged.iter().all(|existing| existing != path) {
            merged.push(path.clone());
        }
    }
    Ok(merged)
}

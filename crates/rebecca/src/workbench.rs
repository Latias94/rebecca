use std::path::PathBuf;

use anyhow::Result;
use rebecca_core::config::AppRuntimeConfig;
use rebecca_core::execution::ExecutionProgressEvent;
use rebecca_core::planner::PlanProgressEvent;
use rebecca_core::scan::ScanBackendKind;
use rebecca_core::{CleanupPlan, DeleteMode, PlanRequest, Platform};

use crate::runtime::CliRuntime;
use crate::workflow_execution::{execute_plan_with_progress, record_execution_report};
use crate::workflow_planner::{
    WorkflowPlanCoreBuild, WorkflowPlanCoreOptions, WorkflowRuleSource, build_workflow_plan_core,
};

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
    .map(|build| build.plan)
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
    execute_recoverable_cleanup_with_progresses(request, runtime_config, runtime, progress, |_| {})
}

pub(crate) fn execute_recoverable_cleanup_with_progresses<P, E>(
    request: &CleanupWorkbenchRequest,
    runtime_config: &AppRuntimeConfig,
    runtime: &CliRuntime,
    plan_progress: P,
    execution_progress: E,
) -> Result<CleanupPlan>
where
    P: for<'a> FnMut(PlanProgressEvent<'a>),
    E: for<'a> FnMut(ExecutionProgressEvent<'a>),
{
    let build = build_plan(
        request.recoverable_delete_plan_request(),
        request,
        runtime_config,
        runtime,
        plan_progress,
    )?;
    let mut plan = build.plan;
    if plan.summary.allowed_targets == 0 {
        return Ok(plan);
    }

    let execution_policy = build.execution_guards.protection_policy();
    let mut execution_report = execute_plan_with_progress(
        &mut plan,
        execution_policy,
        runtime.cancellation(),
        DeleteMode::RecoverableDelete,
        execution_progress,
    )?;
    record_execution_report(
        &mut plan,
        &mut execution_report,
        runtime_config.app_paths.history_file.clone(),
    );
    Ok(plan)
}

fn build_plan(
    plan_request: PlanRequest,
    request: &CleanupWorkbenchRequest,
    runtime_config: &AppRuntimeConfig,
    runtime: &CliRuntime,
    progress: impl for<'a> FnMut(PlanProgressEvent<'a>),
) -> Result<WorkflowPlanCoreBuild> {
    let build = build_workflow_plan_core(
        WorkflowPlanCoreOptions {
            request: &plan_request,
            rule_source: WorkflowRuleSource::BuiltInCatalog,
            runtime_config,
            runtime,
            scan_cache: request.scan_cache,
            scan_backend: request.scan_backend,
            exclude_paths: request.exclude_paths.as_slice(),
        },
        progress,
    )?;
    for diagnostic in &build.rule_diagnostics {
        tracing::warn!(
            message = %diagnostic.message,
            "external cleanup rule skipped during workbench planning"
        );
    }
    Ok(build)
}

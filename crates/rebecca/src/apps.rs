use std::path::PathBuf;

use anyhow::Result;
use rebecca::core::{CleanupWorkflow, DeleteMode, PlanRequest, Platform};

use crate::clean::{ConfirmationKind, WorkflowRuleSource, WorkflowRunOptions, run_workflow};
use crate::cli::OutputMode;
use crate::output::WorkflowOutputContract;
use crate::render;
use crate::runtime::CliRuntime;

#[derive(Debug)]
pub struct AppsScanOptions {
    pub output_mode: OutputMode,
    pub no_progress: bool,
    pub scan_cache: bool,
    pub exclude_paths: Vec<PathBuf>,
}

#[derive(Debug)]
pub struct AppsCleanOptions {
    pub dry_run: bool,
    pub output_mode: OutputMode,
    pub yes: bool,
    pub no_progress: bool,
    pub scan_cache: bool,
    pub exclude_paths: Vec<PathBuf>,
}

pub(crate) fn scan_with_runtime(options: AppsScanOptions, runtime: &CliRuntime) -> Result<()> {
    let request = PlanRequest::for_platform(Platform::Windows, DeleteMode::DryRun)
        .with_workflow(CleanupWorkflow::AppLeftovers);

    run_workflow(
        WorkflowRunOptions {
            request,
            rule_source: WorkflowRuleSource::NativeWorkflow,
            output_mode: options.output_mode,
            yes: false,
            no_progress: options.no_progress,
            scan_cache: options.scan_cache,
            exclude_paths: options.exclude_paths,
            output_contract: WorkflowOutputContract::v1("apps scan", "app-leftovers-cleanup-plan"),
            human_renderer: render::clean::print_plan,
            success_renderer: crate::output::print_plan_with_events,
            cancellation_message: "App leftovers scan cancelled.",
            unsupported_execution_message: "app leftovers cleanup execution is Windows-only; use apps scan to preview",
            confirmation_kind: ConfirmationKind::AppLeftovers,
        },
        runtime,
    )
}

pub(crate) fn clean_with_runtime(options: AppsCleanOptions, runtime: &CliRuntime) -> Result<()> {
    let mode = if options.yes && !options.dry_run {
        DeleteMode::RecycleBin
    } else {
        DeleteMode::DryRun
    };
    let request = PlanRequest::for_platform(Platform::Windows, mode)
        .with_workflow(CleanupWorkflow::AppLeftovers);

    run_workflow(
        WorkflowRunOptions {
            request,
            rule_source: WorkflowRuleSource::NativeWorkflow,
            output_mode: options.output_mode,
            yes: options.yes,
            no_progress: options.no_progress,
            scan_cache: options.scan_cache,
            exclude_paths: options.exclude_paths,
            output_contract: WorkflowOutputContract::v1("apps clean", "app-leftovers-cleanup-plan"),
            human_renderer: render::clean::print_plan,
            success_renderer: crate::output::print_plan_with_events,
            cancellation_message: "App leftovers cleanup cancelled.",
            unsupported_execution_message: "app leftovers cleanup execution is Windows-only; use apps clean without --yes to preview",
            confirmation_kind: ConfirmationKind::AppLeftovers,
        },
        runtime,
    )
}

use std::path::PathBuf;

use anyhow::{Result, anyhow};
use rebecca::core::scan::ScanBackendKind;
use rebecca::core::{CleanupWorkflow, DeleteMode, PlanRequest, Platform};

use crate::clean::{ConfirmationKind, WorkflowRuleSource, WorkflowRunOptions, run_workflow};
use crate::cli::{OutputMode, ProgressDetail};
use crate::output::WorkflowOutputContract;
use crate::render;
use crate::runtime::CliRuntime;

#[derive(Debug)]
pub struct AppsScanOptions {
    pub output_mode: OutputMode,
    pub no_progress: bool,
    pub progress_detail: ProgressDetail,
    pub scan_cache: bool,
    pub exclude_paths: Vec<PathBuf>,
}

#[derive(Debug)]
pub struct AppsCleanOptions {
    pub dry_run: bool,
    pub output_mode: OutputMode,
    pub yes: bool,
    pub permanent: bool,
    pub no_progress: bool,
    pub progress_detail: ProgressDetail,
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
            progress_detail: options.progress_detail,
            scan_cache: options.scan_cache,
            scan_backend: ScanBackendKind::PortableRecursive,
            exclude_paths: options.exclude_paths,
            output_contract: WorkflowOutputContract::v1("apps scan", "app-leftovers-cleanup-plan"),
            human_renderer: render::clean::print_plan,
            success_renderer: crate::output::print_plan_with_events,
            cancellation_message: "App leftovers scan cancelled.",
            confirmation_kind: ConfirmationKind::AppLeftovers,
        },
        runtime,
    )
}

pub(crate) fn clean_with_runtime(options: AppsCleanOptions, runtime: &CliRuntime) -> Result<()> {
    if options.dry_run && options.yes {
        return Err(anyhow!("--dry-run cannot be combined with --yes"));
    }
    if options.permanent && (options.dry_run || !options.yes) {
        return Err(anyhow!(
            "--permanent requires --yes and cannot be combined with --dry-run"
        ));
    }

    let mode = if options.yes && !options.dry_run {
        if options.permanent {
            DeleteMode::PermanentDelete
        } else {
            DeleteMode::RecoverableDelete
        }
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
            progress_detail: options.progress_detail,
            scan_cache: options.scan_cache,
            scan_backend: ScanBackendKind::PortableRecursive,
            exclude_paths: options.exclude_paths,
            output_contract: WorkflowOutputContract::v1("apps clean", "app-leftovers-cleanup-plan"),
            human_renderer: render::clean::print_plan,
            success_renderer: crate::output::print_plan_with_events,
            cancellation_message: "App leftovers cleanup cancelled.",
            confirmation_kind: ConfirmationKind::AppLeftovers,
        },
        runtime,
    )
}

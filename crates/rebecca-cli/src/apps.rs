use anyhow::Result;
use rebecca_core::{CleanupWorkflow, DeleteMode, PlanRequest, Platform, RuleDefinition};

use crate::clean::{ConfirmationKind, WorkflowRunOptions, run_workflow};

const APP_LEFTOVER_RULES: &[RuleDefinition] = &[];

#[derive(Debug)]
pub struct AppsScanOptions {
    pub json: bool,
    pub no_progress: bool,
    pub scan_cache: bool,
}

#[derive(Debug)]
pub struct AppsCleanOptions {
    pub dry_run: bool,
    pub json: bool,
    pub yes: bool,
    pub no_progress: bool,
    pub scan_cache: bool,
}

pub fn scan(options: AppsScanOptions) -> Result<()> {
    let request = PlanRequest::for_platform(Platform::Windows, DeleteMode::DryRun)
        .with_workflow(CleanupWorkflow::AppLeftovers);

    run_workflow(WorkflowRunOptions {
        request,
        rules: APP_LEFTOVER_RULES,
        json: options.json,
        yes: false,
        no_progress: options.no_progress,
        scan_cache: options.scan_cache,
        cancellation_message: "App leftovers scan cancelled.",
        unsupported_execution_message: "app leftovers cleanup execution is Windows-only; use apps scan to preview",
        confirmation_kind: ConfirmationKind::AppLeftovers,
    })
}

pub fn clean(options: AppsCleanOptions) -> Result<()> {
    let mode = if options.yes && !options.dry_run {
        DeleteMode::RecycleBin
    } else {
        DeleteMode::DryRun
    };
    let request = PlanRequest::for_platform(Platform::Windows, mode)
        .with_workflow(CleanupWorkflow::AppLeftovers);

    run_workflow(WorkflowRunOptions {
        request,
        rules: APP_LEFTOVER_RULES,
        json: options.json,
        yes: options.yes,
        no_progress: options.no_progress,
        scan_cache: options.scan_cache,
        cancellation_message: "App leftovers cleanup cancelled.",
        unsupported_execution_message: "app leftovers cleanup execution is Windows-only; use apps clean without --yes to preview",
        confirmation_kind: ConfirmationKind::AppLeftovers,
    })
}

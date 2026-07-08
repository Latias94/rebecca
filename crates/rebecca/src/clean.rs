use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use rebecca::core::config::{AppRuntimeConfig, load_runtime_config};
use rebecca::core::plan::CleanupPlan;
use rebecca::core::scan::ScanBackendKind;
use rebecca::core::{DeleteMode, PlanRequest, Platform, RebeccaError};

use crate::cli::{OutputMode, ProgressDetail};
use crate::output::{
    HumanPlanRenderer, MachineErrorRendered, NdjsonEventWriter, WorkflowOutputContract,
    WorkflowSuccessRenderer,
};
use crate::runtime::CliRuntime;
use crate::text::format_count;
use crate::workflow_artifacts::WorkflowArtifacts;
use crate::workflow_execution::execute_plan;
use crate::workflow_planner::{
    WorkflowPlanBuildOptions, WorkflowPlanBuildOutcome, WorkflowRuleSource, build_workflow_plan,
};
use crate::{output, render};

#[derive(Debug)]
pub struct CleanOptions {
    pub dry_run: bool,
    pub output_mode: OutputMode,
    pub yes: bool,
    pub permanent: bool,
    pub no_progress: bool,
    pub progress_detail: ProgressDetail,
    pub scan_cache: bool,
    pub scan_backend: ScanBackendKind,
    pub categories: Vec<String>,
    pub rules: Vec<String>,
    pub exclude_paths: Vec<PathBuf>,
    pub save_plan: Option<PathBuf>,
    pub receipt: Option<PathBuf>,
    pub allow_moderate: bool,
    pub allow_risky: bool,
    pub allow_warnings: Vec<String>,
}

pub(crate) struct WorkflowRunOptions<'a> {
    pub(crate) request: PlanRequest,
    pub(crate) rule_source: WorkflowRuleSource<'a>,
    pub(crate) output_mode: OutputMode,
    pub(crate) yes: bool,
    pub(crate) no_progress: bool,
    pub(crate) progress_detail: ProgressDetail,
    pub(crate) scan_cache: bool,
    pub(crate) scan_backend: ScanBackendKind,
    pub(crate) exclude_paths: Vec<PathBuf>,
    pub(crate) save_plan: Option<PathBuf>,
    pub(crate) receipt: Option<PathBuf>,
    pub(crate) output_contract: WorkflowOutputContract,
    pub(crate) human_renderer: HumanPlanRenderer,
    pub(crate) success_renderer: WorkflowSuccessRenderer,
    pub(crate) cancellation_message: &'static str,
    pub(crate) confirmation_kind: ConfirmationKind,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum ConfirmationKind {
    Cleanup,
    AppLeftovers,
    ProjectArtifacts,
}

pub(crate) fn run_with_runtime(options: CleanOptions, runtime: &CliRuntime) -> Result<()> {
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

    let mut request = PlanRequest::for_platform(Platform::current(), mode);
    request.selected_categories = options.categories;
    request.selected_rule_ids = options.rules;
    request.allow_moderate = options.allow_moderate;
    request.allow_risky = options.allow_risky;
    for warning in &options.allow_warnings {
        request.add_allowed_warning(warning);
    }

    let catalog = rebecca::rules::builtin_rules()?;
    run_workflow(
        WorkflowRunOptions {
            request,
            rule_source: WorkflowRuleSource::RuleCatalog(&catalog),
            output_mode: options.output_mode,
            yes: options.yes,
            no_progress: options.no_progress,
            progress_detail: options.progress_detail,
            scan_cache: options.scan_cache,
            scan_backend: options.scan_backend,
            exclude_paths: options.exclude_paths,
            save_plan: options.save_plan,
            receipt: options.receipt,
            output_contract: WorkflowOutputContract::v1("clean", "cleanup-plan"),
            human_renderer: render::clean::print_plan,
            success_renderer: output::print_plan_with_events,
            cancellation_message: "Cleanup cancelled.",
            confirmation_kind: ConfirmationKind::Cleanup,
        },
        runtime,
    )
}

pub(crate) fn run_workflow(options: WorkflowRunOptions<'_>, runtime: &CliRuntime) -> Result<()> {
    let runtime_config = load_runtime_config()?;
    run_workflow_with_runtime_config(options, runtime_config, runtime)
}

pub(crate) fn run_workflow_with_runtime_config(
    options: WorkflowRunOptions<'_>,
    runtime_config: AppRuntimeConfig,
    runtime: &CliRuntime,
) -> Result<()> {
    let artifacts = WorkflowArtifacts::new(
        options.output_contract.command,
        options.output_mode,
        options.save_plan.as_deref(),
        options.receipt.as_deref(),
    );
    artifacts.validate_for_mode(options.request.mode)?;

    let build = match build_workflow_plan(WorkflowPlanBuildOptions {
        request: &options.request,
        rule_source: options.rule_source,
        runtime_config: &runtime_config,
        runtime,
        output_mode: options.output_mode,
        no_progress: options.no_progress,
        progress_detail: options.progress_detail,
        scan_cache: options.scan_cache,
        scan_backend: options.scan_backend,
        exclude_paths: options.exclude_paths.as_slice(),
        output_contract: options.output_contract,
    })? {
        WorkflowPlanBuildOutcome::Built(build) => build,
        WorkflowPlanBuildOutcome::PlannerError { err, event_writer } => {
            if err
                .downcast_ref::<rebecca::core::RebeccaError>()
                .is_some_and(|err| {
                    matches!(err, rebecca::core::RebeccaError::OperationCancelled(_))
                })
            {
                return finish_stream_with_cancellation(event_writer, options.cancellation_message);
            }

            return finish_stream_with_error(event_writer, err);
        }
    };
    let mut plan = build.plan;
    let scan_cache_summary = build.scan_cache_summary;
    let event_writer = build.event_writer;

    if options.request.mode.is_dry_run() {
        artifacts.write_preview_plan(&plan)?;
        (options.success_renderer)(
            &plan,
            options.output_contract,
            options.output_mode,
            options.human_renderer,
            scan_cache_summary,
            event_writer,
        )?;
        artifacts.print_preview_guidance();
        return Ok(());
    }

    if plan.summary.allowed_targets == 0 {
        artifacts.write_execution_receipt(&plan)?;
        (options.success_renderer)(
            &plan,
            options.output_contract,
            options.output_mode,
            options.human_renderer,
            scan_cache_summary,
            event_writer,
        )?;
        artifacts.print_execution_guidance(&plan);
        return Ok(());
    }

    let confirmed = if options.yes {
        true
    } else {
        match confirm_cleanup(&plan, options.confirmation_kind) {
            Ok(confirmed) => confirmed,
            Err(err) => return finish_stream_with_error(event_writer, err),
        }
    };
    if !confirmed {
        return finish_stream_with_cancellation(event_writer, options.cancellation_message);
    }

    let execution_policy = build.execution_guards.protection_policy();
    let execution_report = match execute_plan(
        &mut plan,
        execution_policy,
        runtime.cancellation(),
        options.request.mode,
    ) {
        Ok(report) => report,
        Err(RebeccaError::OperationCancelled(_)) => {
            return finish_stream_with_cancellation(event_writer, options.cancellation_message);
        }
        Err(err) => return finish_stream_with_error(event_writer, err.into()),
    };

    artifacts.record_execution(
        &mut plan,
        execution_report,
        runtime_config.app_paths.history_file,
    )?;

    (options.success_renderer)(
        &plan,
        options.output_contract,
        options.output_mode,
        options.human_renderer,
        scan_cache_summary,
        event_writer,
    )?;
    artifacts.print_execution_guidance(&plan);
    Ok(())
}

fn finish_stream_with_error(
    event_writer: Option<NdjsonEventWriter>,
    err: anyhow::Error,
) -> Result<()> {
    if let Some(mut writer) = event_writer {
        writer.emit_error(&err)?;
        return Err(MachineErrorRendered.into());
    }

    Err(err)
}

fn finish_stream_with_cancellation(
    event_writer: Option<NdjsonEventWriter>,
    message: &str,
) -> Result<()> {
    if let Some(mut writer) = event_writer {
        writer.emit_cancelled(message)?;
    } else {
        println!("{message}");
    }

    Ok(())
}

fn confirm_cleanup(plan: &CleanupPlan, kind: ConfirmationKind) -> Result<bool> {
    let target_label = match kind {
        ConfirmationKind::Cleanup => {
            format_count(plan.summary.allowed_targets as u64, "target", "targets")
        }
        ConfirmationKind::AppLeftovers => format_count(
            plan.summary.allowed_targets as u64,
            "app leftover target",
            "app leftover targets",
        ),
        ConfirmationKind::ProjectArtifacts => format_count(
            plan.summary.allowed_targets as u64,
            "project artifact target",
            "project artifact targets",
        ),
    };
    let action = match plan.request.mode {
        DeleteMode::RecoverableDelete => "Move",
        DeleteMode::PermanentDelete => "Permanently delete",
        DeleteMode::DryRun => "Preview",
    };
    let destination = match plan.request.mode {
        DeleteMode::RecoverableDelete => " to the system trash or Recycle Bin",
        DeleteMode::PermanentDelete | DeleteMode::DryRun => "",
    };
    let prompt = format!(
        "{} {}, {} bytes{}?",
        action, target_label, plan.summary.estimated_bytes, destination
    );

    dialoguer::Confirm::new()
        .with_prompt(prompt)
        .default(false)
        .interact()
        .context("cleanup confirmation failed")
}

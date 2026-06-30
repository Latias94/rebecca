use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use rebecca::core::config::load_runtime_config;
use rebecca::core::{CleanupWorkflow, DeleteMode, PlanRequest, Platform};

use crate::clean::{
    ConfirmationKind, WorkflowRuleSource, WorkflowRunOptions, run_workflow_with_runtime_config,
};
use crate::cli::OutputMode;
use crate::inspect;
use crate::output::WorkflowOutputContract;
use crate::purge_view::project_artifact_catalog_entries;
use crate::render;
use crate::runtime::CliRuntime;

#[derive(Debug)]
pub struct PurgeOptions {
    pub dry_run: bool,
    pub output_mode: OutputMode,
    pub yes: bool,
    pub no_progress: bool,
    pub scan_cache: bool,
    pub list_artifacts: bool,
    pub roots: Vec<PathBuf>,
    pub max_depth: Option<usize>,
    pub min_age_days: Option<u64>,
    pub reclaim_limit_bytes: Option<u64>,
    pub artifacts: Vec<String>,
    pub exclude_paths: Vec<PathBuf>,
}

#[derive(Debug)]
pub struct PurgeInspectOptions {
    pub output_mode: OutputMode,
    pub no_progress: bool,
    pub scan_cache: bool,
    pub roots: Vec<PathBuf>,
    pub max_depth: Option<usize>,
    pub min_age_days: Option<u64>,
    pub reclaim_limit_bytes: Option<u64>,
    pub artifacts: Vec<String>,
    pub exclude_paths: Vec<PathBuf>,
}

pub(crate) fn run_with_runtime(options: PurgeOptions, runtime: &CliRuntime) -> Result<()> {
    if options.list_artifacts {
        return print_project_artifact_catalog(options.output_mode);
    }

    let runtime_config = load_runtime_config()?;
    let mode = if options.yes && !options.dry_run {
        DeleteMode::RecycleBin
    } else {
        DeleteMode::DryRun
    };
    let mut request = PlanRequest::for_platform(Platform::Windows, mode)
        .with_workflow(CleanupWorkflow::ProjectArtifacts);
    request.project_artifact_roots = resolve_roots(options.roots, &runtime_config.purge.roots)?;
    request.project_artifact_max_depth =
        options.max_depth.unwrap_or(runtime_config.purge.max_depth);
    request.project_artifact_min_age_days = options
        .min_age_days
        .unwrap_or(runtime_config.purge.min_age_days);
    request.project_artifact_reclaim_limit_bytes = options.reclaim_limit_bytes;
    request.project_artifact_selectors = options.artifacts;

    run_workflow_with_runtime_config(
        WorkflowRunOptions {
            request,
            rule_source: WorkflowRuleSource::NativeWorkflow,
            output_mode: options.output_mode,
            yes: options.yes,
            no_progress: options.no_progress,
            scan_cache: options.scan_cache,
            exclude_paths: options.exclude_paths,
            output_contract: WorkflowOutputContract::v1("purge", "project-artifact-cleanup-plan"),
            human_renderer: render::purge::print_plan,
            success_renderer: crate::output::print_plan_with_events,
            cancellation_message: "Project artifact purge cancelled.",
            unsupported_execution_message: "project artifact purge execution is Windows-only; omit --yes to preview",
            confirmation_kind: ConfirmationKind::ProjectArtifacts,
        },
        runtime_config,
        runtime,
    )
}

pub(crate) fn inspect_with_runtime(
    options: PurgeInspectOptions,
    runtime: &CliRuntime,
) -> Result<()> {
    inspect::artifacts_with_runtime(
        inspect::InspectArtifactsOptions {
            output_mode: options.output_mode,
            no_progress: options.no_progress,
            scan_cache: options.scan_cache,
            roots: options.roots,
            max_depth: options.max_depth,
            min_age_days: options.min_age_days,
            reclaim_limit_bytes: options.reclaim_limit_bytes,
            artifacts: options.artifacts,
            exclude_paths: options.exclude_paths,
            command: "purge inspect",
        },
        runtime,
    )
}

fn print_project_artifact_catalog(output_mode: OutputMode) -> Result<()> {
    let catalog = project_artifact_catalog_entries();

    crate::output::print_command_success(
        "purge",
        "project-artifact-catalog",
        output_mode,
        || &catalog,
        || {
            render::purge::print_project_artifact_catalog(&catalog);
            Ok(())
        },
    )
}

pub(crate) fn resolve_roots(
    cli_roots: Vec<PathBuf>,
    config_roots: &[PathBuf],
) -> Result<Vec<PathBuf>> {
    if !cli_roots.is_empty() {
        return cli_roots
            .into_iter()
            .map(resolve_existing_root)
            .collect::<Result<Vec<_>>>();
    }

    if !config_roots.is_empty() {
        return config_roots
            .iter()
            .cloned()
            .map(resolve_config_root)
            .collect::<Result<Vec<_>>>();
    }

    Ok(vec![
        std::env::current_dir().context("failed to resolve current directory")?,
    ])
}

fn resolve_config_root(root: PathBuf) -> Result<PathBuf> {
    if root.as_os_str().is_empty() {
        return Err(anyhow!("purge root cannot be empty"));
    }

    resolve_absolute_root(root)
}

fn resolve_existing_root(root: PathBuf) -> Result<PathBuf> {
    if root.as_os_str().is_empty() {
        return Err(anyhow!("purge root cannot be empty"));
    }

    let absolute = resolve_absolute_root(root)?;
    let metadata = std::fs::symlink_metadata(&absolute)
        .with_context(|| format!("purge root {} is not accessible", absolute.display()))?;

    if !metadata.is_dir() {
        return Err(anyhow!(
            "purge root {} must be an existing directory",
            absolute.display()
        ));
    }

    if rebecca::core::safety::is_reparse_like(&metadata) {
        return Err(anyhow!(
            "purge root {} must not be a symlink or reparse point",
            absolute.display()
        ));
    }

    Ok(absolute)
}

fn resolve_absolute_root(root: PathBuf) -> Result<PathBuf> {
    Ok(if root.is_absolute() {
        root
    } else {
        std::env::current_dir()
            .context("failed to resolve current directory")?
            .join(root)
    })
}

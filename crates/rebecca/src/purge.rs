use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use rebecca::core::config::load_runtime_config;
use rebecca::core::scan::ScanBackendKind;
use rebecca::core::{CleanupWorkflow, DeleteMode, PlanRequest, Platform};

use crate::clean::{ConfirmationKind, WorkflowRunOptions, run_workflow_with_runtime_config};
use crate::cli::{OutputMode, ProgressDetail};
use crate::output::WorkflowOutputContract;
use crate::render;
use crate::runtime::CliRuntime;
use crate::workflow_planner::WorkflowRuleSource;

#[derive(Debug)]
pub struct PurgeOptions {
    pub dry_run: bool,
    pub output_mode: OutputMode,
    pub yes: bool,
    pub permanent: bool,
    pub no_progress: bool,
    pub progress_detail: ProgressDetail,
    pub scan_cache: bool,
    pub roots: Vec<PathBuf>,
    pub max_depth: Option<usize>,
    pub min_age_days: Option<u64>,
    pub reclaim_limit_bytes: Option<u64>,
    pub artifacts: Vec<String>,
    pub exclude_paths: Vec<PathBuf>,
    pub save_plan: Option<PathBuf>,
    pub receipt: Option<PathBuf>,
}

pub(crate) fn run_with_runtime(options: PurgeOptions, runtime: &CliRuntime) -> Result<()> {
    if options.dry_run && options.yes {
        return Err(anyhow!("--dry-run cannot be combined with --yes"));
    }
    if options.permanent && (options.dry_run || !options.yes) {
        return Err(anyhow!(
            "--permanent requires --yes and cannot be combined with --dry-run"
        ));
    }

    let runtime_config = load_runtime_config()?;
    let mode = if options.yes && !options.dry_run {
        if options.permanent {
            DeleteMode::PermanentDelete
        } else {
            DeleteMode::RecoverableDelete
        }
    } else {
        DeleteMode::DryRun
    };
    let mut request = PlanRequest::for_platform(Platform::current(), mode)
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
            progress_detail: options.progress_detail,
            scan_cache: options.scan_cache,
            scan_backend: ScanBackendKind::PortableRecursive,
            exclude_paths: options.exclude_paths,
            save_plan: options.save_plan,
            receipt: options.receipt,
            output_contract: WorkflowOutputContract::v1("purge", "project-artifact-cleanup-plan"),
            human_renderer: render::purge::print_plan,
            success_renderer: crate::output::print_plan_with_events,
            cancellation_message: "Project artifact purge cancelled.",
            confirmation_kind: ConfirmationKind::ProjectArtifacts,
        },
        runtime_config,
        runtime,
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

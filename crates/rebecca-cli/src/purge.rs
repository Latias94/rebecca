use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use rebecca_core::config::load_runtime_config;
use rebecca_core::{CleanupWorkflow, DeleteMode, PlanRequest, Platform, RuleDefinition};

use crate::clean::{ConfirmationKind, WorkflowRunOptions, run_workflow_with_runtime_config};

const PROJECT_ARTIFACT_RULES: &[RuleDefinition] = &[];

#[derive(Debug)]
pub struct PurgeOptions {
    pub dry_run: bool,
    pub json: bool,
    pub yes: bool,
    pub no_progress: bool,
    pub scan_cache: bool,
    pub roots: Vec<PathBuf>,
    pub max_depth: Option<usize>,
    pub min_age_days: Option<u64>,
    pub artifacts: Vec<String>,
    pub exclude_paths: Vec<PathBuf>,
}

pub fn run(options: PurgeOptions) -> Result<()> {
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
    request.project_artifact_selectors = options.artifacts;

    run_workflow_with_runtime_config(
        WorkflowRunOptions {
            request,
            rules: PROJECT_ARTIFACT_RULES,
            json: options.json,
            yes: options.yes,
            no_progress: options.no_progress,
            scan_cache: options.scan_cache,
            exclude_paths: options.exclude_paths,
            cancellation_message: "Project artifact purge cancelled.",
            unsupported_execution_message: "project artifact purge execution is Windows-only; omit --yes to preview",
            confirmation_kind: ConfirmationKind::ProjectArtifacts,
        },
        runtime_config,
    )
}

fn resolve_roots(cli_roots: Vec<PathBuf>, config_roots: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let roots = if !cli_roots.is_empty() {
        cli_roots
    } else if !config_roots.is_empty() {
        config_roots.to_vec()
    } else {
        vec![std::env::current_dir().context("failed to resolve current directory")?]
    };

    roots
        .into_iter()
        .map(resolve_root)
        .collect::<Result<Vec<_>>>()
}

fn resolve_root(root: PathBuf) -> Result<PathBuf> {
    if root.as_os_str().is_empty() {
        return Err(anyhow!("purge root cannot be empty"));
    }

    let absolute = if root.is_absolute() {
        root
    } else {
        std::env::current_dir()
            .context("failed to resolve current directory")?
            .join(root)
    };
    let metadata = std::fs::symlink_metadata(&absolute)
        .with_context(|| format!("purge root {} is not accessible", absolute.display()))?;

    if !metadata.is_dir() {
        return Err(anyhow!(
            "purge root {} must be an existing directory",
            absolute.display()
        ));
    }

    if rebecca_core::safety::is_reparse_like(&metadata) {
        return Err(anyhow!(
            "purge root {} must not be a symlink or reparse point",
            absolute.display()
        ));
    }

    Ok(absolute)
}

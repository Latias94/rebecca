use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use rebecca_core::{
    CleanupWorkflow, DEFAULT_PROJECT_ARTIFACT_MAX_DEPTH, DeleteMode, PlanRequest, Platform,
    RuleDefinition,
};

use crate::clean::{ConfirmationKind, WorkflowRunOptions, run_workflow};

const PROJECT_ARTIFACT_RULES: &[RuleDefinition] = &[];

#[derive(Debug)]
pub struct PurgeOptions {
    pub dry_run: bool,
    pub json: bool,
    pub yes: bool,
    pub no_progress: bool,
    pub scan_cache: bool,
    pub roots: Vec<PathBuf>,
    pub max_depth: usize,
    pub exclude_paths: Vec<PathBuf>,
}

pub fn run(options: PurgeOptions) -> Result<()> {
    let mode = if options.yes && !options.dry_run {
        DeleteMode::RecycleBin
    } else {
        DeleteMode::DryRun
    };
    let mut request = PlanRequest::for_platform(Platform::Windows, mode)
        .with_workflow(CleanupWorkflow::ProjectArtifacts);
    request.project_artifact_roots = resolve_roots(options.roots)?;
    request.project_artifact_max_depth = options.max_depth;

    run_workflow(WorkflowRunOptions {
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
    })
}

fn resolve_roots(roots: Vec<PathBuf>) -> Result<Vec<PathBuf>> {
    let current_dir = std::env::current_dir().context("failed to resolve current directory")?;
    let roots = if roots.is_empty() {
        vec![current_dir]
    } else {
        roots
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

pub(crate) fn default_max_depth() -> usize {
    DEFAULT_PROJECT_ARTIFACT_MAX_DEPTH
}

use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use rebecca_core::config::load_runtime_config;
use rebecca_core::project_artifacts::{
    ProjectArtifactDefinition, all_project_artifact_definitions,
};
use rebecca_core::{CleanupWorkflow, DeleteMode, PlanRequest, Platform, RuleDefinition};

use crate::clean::{ConfirmationKind, WorkflowRunOptions, run_workflow_with_runtime_config};
use crate::cli::OutputMode;

const PROJECT_ARTIFACT_RULES: &[RuleDefinition] = &[];

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
    pub artifacts: Vec<String>,
    pub exclude_paths: Vec<PathBuf>,
}

pub fn run(options: PurgeOptions) -> Result<()> {
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
    request.project_artifact_selectors = options.artifacts;

    run_workflow_with_runtime_config(
        WorkflowRunOptions {
            request,
            rules: PROJECT_ARTIFACT_RULES,
            output_mode: options.output_mode,
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

fn print_project_artifact_catalog(output_mode: OutputMode) -> Result<()> {
    let definitions = all_project_artifact_definitions().collect::<Vec<_>>();

    if output_mode.is_json() {
        let values = definitions
            .iter()
            .map(|definition| {
                serde_json::json!({
                    "artifact": definition.directory_name,
                    "rule_id": definition.rule_id,
                    "rule_suffix": project_artifact_rule_suffix(definition.rule_id),
                    "restore_hint": definition.restore_hint,
                })
            })
            .collect::<Vec<_>>();
        return crate::output::print_success("purge", "project-artifact-catalog", &values);
    }

    if output_mode.is_ndjson() {
        let values = definitions
            .iter()
            .map(|definition| {
                serde_json::json!({
                    "artifact": definition.directory_name,
                    "rule_id": definition.rule_id,
                    "rule_suffix": project_artifact_rule_suffix(definition.rule_id),
                    "restore_hint": definition.restore_hint,
                })
            })
            .collect::<Vec<_>>();
        return crate::output::print_success_event("purge", "project-artifact-catalog", &values);
    }

    println!("Supported project artifacts: {}", definitions.len());
    for definition in definitions {
        println!("- {}", definition.directory_name);
        println!(
            "  selectors: {}",
            project_artifact_selectors_label(definition)
        );
        println!("  rule: {}", definition.rule_id);
        println!("  restore: {}", definition.restore_hint);
    }

    Ok(())
}

fn project_artifact_selectors_label(definition: ProjectArtifactDefinition) -> String {
    let rule_suffix = project_artifact_rule_suffix(definition.rule_id);
    if definition.directory_name.eq_ignore_ascii_case(rule_suffix) {
        format!("{}, {}", definition.directory_name, definition.rule_id)
    } else {
        format!(
            "{}, {}, {}",
            definition.directory_name, rule_suffix, definition.rule_id
        )
    }
}

fn project_artifact_rule_suffix(rule_id: &str) -> &str {
    rule_id
        .strip_prefix("windows.project-artifact-")
        .unwrap_or(rule_id)
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

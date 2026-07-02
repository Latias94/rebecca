use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use rebecca::core::config::{AppRuntimeConfig, load_runtime_config};
use rebecca::core::disk_map::{DiskMapRequest, inspect_map as inspect_map_core};
use rebecca::core::inspect::{
    SpaceInsightRequest, SpaceInsightScanCache, inspect_space as inspect_space_core,
};
use rebecca::core::lint::{LintReportRequest, inspect_lint as inspect_lint_core};
use rebecca::core::scan::ScanBackendKind;
use rebecca::core::scan_cache::ScanCacheStore;
use rebecca::core::{CleanupWorkflow, DeleteMode, PlanRequest, Platform};

use crate::clean::{
    ConfirmationKind, WorkflowRuleSource, WorkflowRunOptions, run_workflow_with_runtime_config,
};
use crate::clean_view::ScanCacheProgressSummary;
use crate::cli::{OutputMode, ProgressDetail, ScanBackendArg};
use crate::output::{
    CliApiContract, HumanPlanRenderer, NdjsonEventWriter, WorkflowOutputContract,
    print_command_success_with_contract, print_workflow_success_payload,
};
use crate::purge::resolve_roots;
use crate::purge_view::ProjectArtifactInsightReport;
use crate::render;
use crate::runtime::CliRuntime;

#[derive(Debug)]
pub struct InspectSpaceOptions {
    pub output_mode: OutputMode,
    pub no_progress: bool,
    pub scan_cache: bool,
    pub scan_backend: ScanBackendArg,
    pub roots: Vec<PathBuf>,
    pub top_limit: usize,
}

#[derive(Debug)]
pub struct InspectMapOptions {
    pub output_mode: OutputMode,
    pub scan_backend: ScanBackendArg,
    pub roots: Vec<PathBuf>,
    pub top_limit: usize,
    pub max_depth: Option<usize>,
}

#[derive(Debug)]
pub struct InspectArtifactsOptions {
    pub output_mode: OutputMode,
    pub no_progress: bool,
    pub progress_detail: ProgressDetail,
    pub scan_cache: bool,
    pub roots: Vec<PathBuf>,
    pub max_depth: Option<usize>,
    pub min_age_days: Option<u64>,
    pub reclaim_limit_bytes: Option<u64>,
    pub artifacts: Vec<String>,
    pub exclude_paths: Vec<PathBuf>,
    pub command: &'static str,
}

#[derive(Debug)]
pub struct InspectLintOptions {
    pub output_mode: OutputMode,
    pub roots: Vec<PathBuf>,
    pub reference_roots: Vec<PathBuf>,
    pub exclude_paths: Vec<PathBuf>,
    pub large_file_threshold_bytes: u64,
    pub top_limit: usize,
}

pub(crate) fn space_with_runtime(options: InspectSpaceOptions, runtime: &CliRuntime) -> Result<()> {
    let _progress_enabled = options.output_mode.is_human() && !options.no_progress;
    let runtime_config = load_runtime_config()?;
    let roots = resolve_space_roots(options.roots)?;
    let mut request = SpaceInsightRequest::new(roots)
        .with_top_limit(options.top_limit.max(1))
        .with_scan_backend(options.scan_backend.into());
    if options.scan_cache {
        request = request.with_scan_cache(SpaceInsightScanCache::new(
            ScanCacheStore::from_app_paths(&runtime_config.app_paths),
            runtime_config.scan_cache_policy,
        ));
    }

    let report = inspect_space_core(&request, runtime.cancellation())?;
    print_command_success_with_contract(
        CliApiContract::v1("inspect space", "inspect-space"),
        options.output_mode,
        || &report,
        || render::inspect::print_space_report(&report),
    )
}

pub(crate) fn map_with_runtime(options: InspectMapOptions, runtime: &CliRuntime) -> Result<()> {
    let roots = resolve_space_roots(options.roots)?;
    let request = DiskMapRequest::new(roots)
        .with_top_limit(options.top_limit)
        .with_max_depth(options.max_depth)
        .with_scan_backend(options.scan_backend.into());

    let report = inspect_map_core(&request, runtime.cancellation())?;
    print_command_success_with_contract(
        CliApiContract::v1("inspect map", "inspect-map"),
        options.output_mode,
        || &report,
        || render::inspect::print_map_report(&report),
    )
}

pub(crate) fn artifacts_with_runtime(
    options: InspectArtifactsOptions,
    runtime: &CliRuntime,
) -> Result<()> {
    let runtime_config = load_runtime_config()?;
    artifacts_with_runtime_config(options, runtime_config, runtime)
}

fn artifacts_with_runtime_config(
    options: InspectArtifactsOptions,
    runtime_config: AppRuntimeConfig,
    runtime: &CliRuntime,
) -> Result<()> {
    let mut request = PlanRequest::for_platform(Platform::Windows, DeleteMode::DryRun)
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
            yes: false,
            no_progress: options.no_progress,
            progress_detail: options.progress_detail,
            scan_cache: options.scan_cache,
            scan_backend: ScanBackendKind::PortableRecursive,
            exclude_paths: options.exclude_paths,
            output_contract: WorkflowOutputContract::v1(options.command, "inspect-artifacts"),
            human_renderer: render::purge::print_project_artifact_insight,
            success_renderer: print_project_artifact_insight_with_events,
            cancellation_message: "Project artifact inspection cancelled.",
            unsupported_execution_message: "project artifact inspection is read-only",
            confirmation_kind: ConfirmationKind::ProjectArtifacts,
        },
        runtime_config,
        runtime,
    )
}

pub(crate) fn lint_with_runtime(options: InspectLintOptions, runtime: &CliRuntime) -> Result<()> {
    let runtime_config = load_runtime_config()?;
    let reference_roots = resolve_optional_roots(options.reference_roots)?;
    let roots = merge_lint_roots(
        resolve_space_roots(options.roots)?,
        reference_roots.as_slice(),
    );
    let exclude_paths = resolve_optional_roots(options.exclude_paths)?;
    let protected_roots = runtime_config
        .app_paths
        .storage_entries()
        .into_iter()
        .map(|entry| entry.path)
        .chain(runtime_config.protected_paths)
        .collect::<Vec<_>>();

    let request = LintReportRequest::new(roots)
        .with_reference_roots(reference_roots)
        .with_protected_roots(protected_roots)
        .with_exclude_paths(exclude_paths)
        .with_large_file_threshold_bytes(options.large_file_threshold_bytes)
        .with_top_limit(options.top_limit.max(1));
    let report = inspect_lint_core(&request, runtime.cancellation())?;

    print_command_success_with_contract(
        CliApiContract::v1("inspect lint", "inspect-lint"),
        options.output_mode,
        || &report,
        || render::inspect::print_lint_report(&report),
    )
}

fn print_project_artifact_insight_with_events(
    plan: &rebecca::core::plan::CleanupPlan,
    contract: WorkflowOutputContract,
    mode: OutputMode,
    human_renderer: HumanPlanRenderer,
    scan_cache_summary: Option<ScanCacheProgressSummary>,
    event_writer: Option<NdjsonEventWriter>,
) -> Result<()> {
    let insight = ProjectArtifactInsightReport::from_plan(plan);
    match mode {
        OutputMode::Human => human_renderer(plan, scan_cache_summary),
        OutputMode::Json => print_command_success_with_contract(
            contract,
            mode,
            || &insight,
            || unreachable!("json mode renders machine payload"),
        ),
        OutputMode::Ndjson => print_workflow_success_payload(
            plan,
            &insight,
            contract,
            mode,
            human_renderer,
            scan_cache_summary,
            event_writer,
        ),
    }
}

fn resolve_space_roots(cli_roots: Vec<PathBuf>) -> Result<Vec<PathBuf>> {
    let roots = if cli_roots.is_empty() {
        vec![std::env::current_dir().context("failed to resolve current directory")?]
    } else {
        cli_roots
    };

    roots
        .into_iter()
        .map(resolve_existing_space_root)
        .collect::<Result<Vec<_>>>()
}

fn resolve_existing_space_root(root: PathBuf) -> Result<PathBuf> {
    if root.as_os_str().is_empty() {
        return Err(anyhow!("inspect root cannot be empty"));
    }

    let absolute = if root.is_absolute() {
        root
    } else {
        std::env::current_dir()
            .context("failed to resolve current directory")?
            .join(root)
    };
    Ok(absolute)
}

fn resolve_optional_roots(roots: Vec<PathBuf>) -> Result<Vec<PathBuf>> {
    roots
        .into_iter()
        .map(resolve_existing_space_root)
        .collect::<Result<Vec<_>>>()
}

fn merge_lint_roots(mut roots: Vec<PathBuf>, reference_roots: &[PathBuf]) -> Vec<PathBuf> {
    for reference in reference_roots {
        if !roots.iter().any(|root| same_lint_root(root, reference)) {
            roots.push(reference.clone());
        }
    }
    roots
}

fn same_lint_root(left: &Path, right: &Path) -> bool {
    let left = left.as_os_str().to_string_lossy();
    let right = right.as_os_str().to_string_lossy();
    if cfg!(windows) {
        left.eq_ignore_ascii_case(&right)
    } else {
        left == right
    }
}

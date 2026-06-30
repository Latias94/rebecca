use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use rebecca::core::config::load_runtime_config;
use rebecca::core::plan::{CleanupPlan, CleanupSummary};
use rebecca::core::project_artifacts::{
    ProjectArtifactDefinition, ProjectArtifactDiscoveryDiagnostic, all_project_artifact_definitions,
};
use rebecca::core::{
    CleanupWorkflow, DeleteMode, EstimateSource, PlanRequest, Platform, RuleDefinition,
    TargetStatus,
};
use serde::Serialize;

use crate::clean::{ConfirmationKind, WorkflowRunOptions, run_workflow_with_runtime_config};
use crate::clean_view::ScanCacheProgressSummary;
use crate::cli::OutputMode;
use crate::output::{HumanPlanRenderer, NdjsonEventWriter, WorkflowOutputContract, print_success};
use crate::render;
use crate::runtime::CliRuntime;

const PROJECT_ARTIFACT_RULES: &[RuleDefinition] = &[];
const INSIGHT_TOP_TARGET_LIMIT: usize = 10;

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

#[derive(Debug)]
pub struct PurgeInspectOptions {
    pub output_mode: OutputMode,
    pub no_progress: bool,
    pub scan_cache: bool,
    pub roots: Vec<PathBuf>,
    pub max_depth: Option<usize>,
    pub min_age_days: Option<u64>,
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
            output_contract: WorkflowOutputContract {
                command: "purge",
                payload_kind: "project-artifact-cleanup-plan",
            },
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
    let runtime_config = load_runtime_config()?;
    let mut request = PlanRequest::for_platform(Platform::Windows, DeleteMode::DryRun)
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
            yes: false,
            no_progress: options.no_progress,
            scan_cache: options.scan_cache,
            exclude_paths: options.exclude_paths,
            output_contract: WorkflowOutputContract {
                command: "purge inspect",
                payload_kind: "project-artifact-insight",
            },
            human_renderer: print_project_artifact_insight_human,
            success_renderer: print_project_artifact_insight_with_events,
            cancellation_message: "Project artifact inspection cancelled.",
            unsupported_execution_message: "project artifact inspection is read-only",
            confirmation_kind: ConfirmationKind::ProjectArtifacts,
        },
        runtime_config,
        runtime,
    )
}

fn print_project_artifact_insight_with_events(
    plan: &CleanupPlan,
    contract: WorkflowOutputContract,
    mode: OutputMode,
    human_renderer: HumanPlanRenderer,
    scan_cache_summary: Option<ScanCacheProgressSummary>,
    event_writer: Option<NdjsonEventWriter>,
) -> Result<()> {
    let insight = ProjectArtifactInsightReport::from_plan(plan);

    if mode.is_json() {
        return print_success(contract.command, contract.payload_kind, &insight);
    }

    if mode.is_ndjson() {
        let mut writer = event_writer.unwrap_or_else(|| NdjsonEventWriter::new(contract.command));
        return writer.emit_completed(contract.payload_kind, &insight);
    }

    human_renderer(plan, scan_cache_summary)
}

fn print_project_artifact_insight_human(
    plan: &CleanupPlan,
    scan_cache_summary: Option<ScanCacheProgressSummary>,
) -> Result<()> {
    let insight = ProjectArtifactInsightReport::from_plan(plan);

    println!("Project artifact insight");
    println!(
        "Roots: {}",
        crate::render::format_count(insight.roots.len() as u64, "root", "roots")
    );
    println!("Targets: {}", insight.summary.total_targets);
    println!(
        "Estimated bytes: {} ({})",
        insight.summary.estimated_bytes,
        crate::output::format_bytes(insight.summary.estimated_bytes)
    );
    println!(
        "Diagnostics: {}",
        crate::render::format_count(
            insight.discovery_diagnostics.len() as u64,
            "observation",
            "observations",
        )
    );
    if let Some(summary) = scan_cache_summary.filter(|summary| {
        summary.hits > 0 || summary.misses > 0 || summary.write_skipped > 0 || summary.pruned > 0
    }) {
        println!(
            "Scan cache summary: {} {}, {} {}, {} {}, {} {}",
            summary.hits,
            if summary.hits == 1 { "hit" } else { "hits" },
            summary.misses,
            if summary.misses == 1 {
                "miss"
            } else {
                "misses"
            },
            summary.write_skipped,
            if summary.write_skipped == 1 {
                "skipped write"
            } else {
                "skipped writes"
            },
            summary.pruned,
            if summary.pruned == 1 {
                "pruned record"
            } else {
                "pruned records"
            }
        );
    }

    if !insight.totals_by_artifact.is_empty() {
        println!();
        println!("Artifact totals:");
        for total in &insight.totals_by_artifact {
            println!(
                "- {}: {}, {} bytes ({})",
                total.label,
                total.targets,
                total.estimated_bytes,
                crate::output::format_bytes(total.estimated_bytes)
            );
        }
    }

    if !insight.top_targets.is_empty() {
        println!();
        println!("Top project artifact targets:");
        for target in &insight.top_targets {
            println!(
                "  - {} [{}] {} bytes ({}) [{}] - {}",
                target.artifact,
                target.status.label(),
                target.estimated_bytes,
                crate::output::format_bytes(target.estimated_bytes),
                target.estimate_source.label(),
                target.path.display()
            );
        }
    }

    if !insight.discovery_diagnostics.is_empty() {
        println!();
        println!("Discovery diagnostics:");
        for diagnostic in &insight.discovery_diagnostics {
            println!(
                "  - {} {} - {}",
                diagnostic.kind.label(),
                diagnostic.path.display(),
                diagnostic.detail
            );
        }
    }

    Ok(())
}

#[derive(Debug, Serialize)]
struct ProjectArtifactInsightReport {
    roots: Vec<PathBuf>,
    summary: CleanupSummary,
    totals_by_root: Vec<ProjectArtifactInsightTotal>,
    totals_by_project: Vec<ProjectArtifactInsightTotal>,
    totals_by_artifact: Vec<ProjectArtifactInsightTotal>,
    top_targets: Vec<ProjectArtifactInsightTarget>,
    discovery_diagnostics: Vec<ProjectArtifactDiscoveryDiagnostic>,
}

impl ProjectArtifactInsightReport {
    fn from_plan(plan: &CleanupPlan) -> Self {
        Self {
            roots: plan.request.project_artifact_roots.clone(),
            summary: plan.summary.clone(),
            totals_by_root: insight_totals(plan, RootGrouping::Root),
            totals_by_project: insight_totals(plan, RootGrouping::Project),
            totals_by_artifact: insight_totals(plan, RootGrouping::Artifact),
            top_targets: insight_top_targets(plan),
            discovery_diagnostics: plan.discovery_diagnostics.clone(),
        }
    }
}

#[derive(Debug, Serialize)]
struct ProjectArtifactInsightTotal {
    key: String,
    label: String,
    targets: usize,
    estimated_bytes: u64,
}

#[derive(Debug, Serialize)]
struct ProjectArtifactInsightTarget {
    rule_id: String,
    artifact: String,
    path: PathBuf,
    project_root: Option<PathBuf>,
    status: TargetStatus,
    estimated_bytes: u64,
    estimate_source: EstimateSource,
    reason: Option<String>,
}

#[derive(Debug, Clone, Copy)]
enum RootGrouping {
    Root,
    Project,
    Artifact,
}

#[derive(Debug, Default)]
struct InsightTotalAccumulator {
    targets: usize,
    estimated_bytes: u64,
}

fn insight_totals(plan: &CleanupPlan, grouping: RootGrouping) -> Vec<ProjectArtifactInsightTotal> {
    let mut totals = BTreeMap::<String, InsightTotalAccumulator>::new();

    for target in &plan.targets {
        let key = match grouping {
            RootGrouping::Root => root_for_target(plan, target.path.as_path())
                .unwrap_or_else(|| PathBuf::from("(outside configured roots)"))
                .display()
                .to_string(),
            RootGrouping::Project => target
                .project_artifact
                .as_ref()
                .map(|context| context.project_root.display().to_string())
                .unwrap_or_else(|| {
                    project_path_for(target.path.as_path())
                        .display()
                        .to_string()
                }),
            RootGrouping::Artifact => artifact_label(&target.rule_id, target.path.as_path()),
        };
        let entry = totals.entry(key).or_default();
        entry.targets = entry.targets.saturating_add(1);
        entry.estimated_bytes = entry.estimated_bytes.saturating_add(target.estimated_bytes);
    }

    totals
        .into_iter()
        .map(|(key, total)| ProjectArtifactInsightTotal {
            label: key.clone(),
            key,
            targets: total.targets,
            estimated_bytes: total.estimated_bytes,
        })
        .collect()
}

fn insight_top_targets(plan: &CleanupPlan) -> Vec<ProjectArtifactInsightTarget> {
    let mut targets = plan
        .targets
        .iter()
        .map(|target| ProjectArtifactInsightTarget {
            rule_id: target.rule_id.clone(),
            artifact: artifact_label(&target.rule_id, target.path.as_path()),
            path: target.path.clone(),
            project_root: target
                .project_artifact
                .as_ref()
                .map(|context| context.project_root.clone()),
            status: target.status,
            estimated_bytes: target.estimated_bytes,
            estimate_source: target.estimate_source,
            reason: target.reason.clone(),
        })
        .collect::<Vec<_>>();

    targets.sort_by(|left, right| {
        right
            .estimated_bytes
            .cmp(&left.estimated_bytes)
            .then_with(|| left.artifact.cmp(&right.artifact))
            .then_with(|| left.path.cmp(&right.path))
    });
    targets.truncate(INSIGHT_TOP_TARGET_LIMIT);
    targets
}

fn root_for_target(plan: &CleanupPlan, path: &Path) -> Option<PathBuf> {
    plan.request
        .project_artifact_roots
        .iter()
        .filter(|root| path.starts_with(root))
        .max_by_key(|root| root.components().count())
        .cloned()
}

fn artifact_label(rule_id: &str, path: &Path) -> String {
    match rule_id {
        "windows.project-artifact-cachedir-tag" => "CACHEDIR.TAG".to_string(),
        "windows.project-artifact-composer-vendor" => "vendor (Composer)".to_string(),
        "windows.project-artifact-dotnet-bin" => "bin (.NET)".to_string(),
        "windows.project-artifact-dotnet-obj" => "obj (.NET)".to_string(),
        _ => path
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_string)
            .unwrap_or_else(|| {
                rule_id
                    .strip_prefix("windows.project-artifact-")
                    .unwrap_or(rule_id)
                    .replace('-', "_")
            }),
    }
}

fn project_path_for(path: &Path) -> PathBuf {
    path.parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| path.to_path_buf())
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

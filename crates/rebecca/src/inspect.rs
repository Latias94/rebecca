use std::borrow::Cow;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, ensure};
use rebecca::core::config::{AppRuntimeConfig, load_runtime_config};
use rebecca::core::disk_map::{
    DiskMapEntry, DiskMapGroup, DiskMapGroupKind, DiskMapMetrics, DiskMapReport, DiskMapRequest,
    DiskMapSortField, inspect_map as inspect_map_core,
};
use rebecca::core::inspect::{
    SpaceInsightRequest, SpaceInsightScanCache, inspect_space as inspect_space_core,
};
use rebecca::core::lint::{LintReportRequest, inspect_lint as inspect_lint_core};
use rebecca::core::scan::ScanBackendKind;
use rebecca::core::scan_cache::ScanCacheStore;
use rebecca::core::{CleanupWorkflow, DeleteMode, EstimateProvenance, PlanRequest, Platform};
use serde::Serialize;

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
    pub diagnostic_limit: usize,
}

#[derive(Debug)]
pub struct InspectMapOptions {
    pub output_mode: OutputMode,
    pub scan_backend: ScanBackendArg,
    pub roots: Vec<PathBuf>,
    pub top_limit: usize,
    pub sort: DiskMapSortField,
    pub min_logical_bytes: Option<u64>,
    pub entry_kind: Option<rebecca::core::disk_map::DiskMapEntryKind>,
    pub path_contains: Option<String>,
    pub group_kinds: Vec<DiskMapGroupKind>,
    pub group_limit: usize,
    pub group_sort: DiskMapSortField,
    pub table_format: Option<InspectMapTableFormat>,
    pub table_row_kinds: Vec<InspectMapTableRowKind>,
    pub diagnostic_limit: usize,
    pub max_depth: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InspectMapTableFormat {
    Csv,
    Tsv,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InspectMapTableRowKind {
    Total,
    Root,
    Entry,
    Group,
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
        .with_diagnostic_limit(options.diagnostic_limit)
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
    if !options.table_row_kinds.is_empty() {
        ensure!(
            options.table_format.is_some(),
            "--table-row requires --table csv|tsv"
        );
    }

    if options.table_format.is_some() {
        ensure!(
            options.output_mode.is_human(),
            "--table cannot be combined with --format {}; table output writes raw rows",
            options.output_mode
        );
    }

    let roots = resolve_space_roots(options.roots)?;
    let request = DiskMapRequest::new(roots)
        .with_top_limit(options.top_limit)
        .with_top_sort(options.sort)
        .with_min_logical_bytes(options.min_logical_bytes)
        .with_entry_kind(options.entry_kind)
        .with_path_contains(options.path_contains)
        .with_group_kinds(options.group_kinds)
        .with_group_limit(options.group_limit)
        .with_group_sort(options.group_sort)
        .with_diagnostic_limit(options.diagnostic_limit)
        .with_max_depth(options.max_depth)
        .with_scan_backend(options.scan_backend.into());

    let report = inspect_map_core(&request, runtime.cancellation())?;
    if let Some(table_format) = options.table_format {
        return print_map_report_table(table_format, &options.table_row_kinds, &report);
    }

    let contract = CliApiContract::v1("inspect map", "inspect-map");
    match options.output_mode {
        OutputMode::Ndjson => print_map_report_ndjson(contract, &report),
        _ => print_command_success_with_contract(
            contract,
            options.output_mode,
            || &report,
            || render::inspect::print_map_report(&report),
        ),
    }
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

#[derive(Debug, Serialize)]
struct InspectMapEntryEvent<'a> {
    rank: usize,
    entry: &'a DiskMapEntry,
}

#[derive(Debug, Serialize)]
struct InspectMapGroupEvent<'a> {
    rank: usize,
    group: &'a DiskMapGroup,
}

fn print_map_report_ndjson(contract: CliApiContract, report: &DiskMapReport) -> Result<()> {
    let mut writer = NdjsonEventWriter::with_contract(contract);

    for (index, entry) in report.top_entries.iter().enumerate() {
        writer.emit_payload(
            "map-entry",
            "inspect-map-entry",
            &InspectMapEntryEvent {
                rank: index + 1,
                entry,
            },
        )?;
    }

    for (index, group) in report.groups.iter().enumerate() {
        writer.emit_payload(
            "map-group",
            "inspect-map-group",
            &InspectMapGroupEvent {
                rank: index + 1,
                group,
            },
        )?;
    }

    writer.emit_completed(contract.payload_kind, report)
}

const INSPECT_MAP_TABLE_HEADER: [&str; 23] = [
    "row_kind",
    "rank",
    "path",
    "root",
    "status",
    "entry_kind",
    "group_kind",
    "group_key",
    "group_label",
    "depth",
    "logical_bytes",
    "allocated_bytes",
    "unique_logical_bytes",
    "unique_allocated_bytes",
    "files",
    "directories",
    "estimate_source",
    "estimate_backend",
    "estimate_backend_source",
    "estimate_confidence",
    "estimate_fallback_reason",
    "estimate_caveats",
    "reason",
];

fn print_map_report_table(
    format: InspectMapTableFormat,
    row_kinds: &[InspectMapTableRowKind],
    report: &DiskMapReport,
) -> Result<()> {
    let stdout = io::stdout();
    let mut writer = stdout.lock();

    write_table_row(&mut writer, format, INSPECT_MAP_TABLE_HEADER)?;
    if includes_table_row(row_kinds, InspectMapTableRowKind::Total) {
        write_table_row(&mut writer, format, total_table_row(report))?;
    }

    if includes_table_row(row_kinds, InspectMapTableRowKind::Root) {
        for (index, root) in report.roots.iter().enumerate() {
            let mut row = table_row_prefix(TableRowPrefix {
                row_kind: "root",
                rank: Some(index + 1),
                path: root.path.display().to_string(),
                status: root.status.label().to_string(),
                ..TableRowPrefix::default()
            });
            push_metrics(&mut row, &root.metrics);
            push_provenance(
                &mut row,
                Some(root.estimate_source.label()),
                &root.estimate_provenance,
            );
            row.push(root.reason.clone().unwrap_or_default());
            write_table_row(&mut writer, format, row)?;
        }
    }

    if includes_table_row(row_kinds, InspectMapTableRowKind::Entry) {
        for (index, entry) in report.top_entries.iter().enumerate() {
            let mut row = table_row_prefix(TableRowPrefix {
                row_kind: "entry",
                rank: Some(index + 1),
                path: entry.path.display().to_string(),
                root: entry.root.display().to_string(),
                entry_kind: entry.kind.label().to_string(),
                depth: Some(entry.depth),
                ..TableRowPrefix::default()
            });
            push_entry_metrics(&mut row, entry);
            push_provenance(
                &mut row,
                Some(entry.estimate_source.label()),
                &entry.estimate_provenance,
            );
            row.push(String::new());
            write_table_row(&mut writer, format, row)?;
        }
    }

    if includes_table_row(row_kinds, InspectMapTableRowKind::Group) {
        for (index, group) in report.groups.iter().enumerate() {
            let mut row = table_row_prefix(TableRowPrefix {
                row_kind: "group",
                rank: Some(index + 1),
                group_kind: group.kind.label().to_string(),
                group_key: group.key.clone(),
                group_label: group.label.clone(),
                ..TableRowPrefix::default()
            });
            push_metrics(&mut row, &group.metrics);
            push_empty_provenance(&mut row);
            row.push(String::new());
            write_table_row(&mut writer, format, row)?;
        }
    }

    writer.flush()?;
    Ok(())
}

fn includes_table_row(
    row_kinds: &[InspectMapTableRowKind],
    row_kind: InspectMapTableRowKind,
) -> bool {
    row_kinds.is_empty() || row_kinds.contains(&row_kind)
}

fn total_table_row(report: &DiskMapReport) -> Vec<String> {
    let mut row = table_row_prefix(TableRowPrefix {
        row_kind: "total",
        ..TableRowPrefix::default()
    });
    push_metrics(&mut row, &report.totals);
    push_empty_provenance(&mut row);
    row.push(String::new());
    row
}

#[derive(Debug, Default)]
struct TableRowPrefix {
    row_kind: &'static str,
    rank: Option<usize>,
    path: String,
    root: String,
    status: String,
    entry_kind: String,
    group_kind: String,
    group_key: String,
    group_label: String,
    depth: Option<usize>,
}

fn table_row_prefix(prefix: TableRowPrefix) -> Vec<String> {
    vec![
        prefix.row_kind.to_string(),
        optional_usize(prefix.rank),
        prefix.path,
        prefix.root,
        prefix.status,
        prefix.entry_kind,
        prefix.group_kind,
        prefix.group_key,
        prefix.group_label,
        optional_usize(prefix.depth),
    ]
}

fn push_entry_metrics(row: &mut Vec<String>, entry: &DiskMapEntry) {
    row.extend([
        entry.logical_bytes.to_string(),
        optional_u64(entry.allocated_bytes),
        optional_u64(entry.unique_logical_bytes),
        optional_u64(entry.unique_allocated_bytes),
        entry.files.to_string(),
        entry.directories.to_string(),
    ]);
}

fn push_metrics(row: &mut Vec<String>, metrics: &DiskMapMetrics) {
    row.extend([
        metrics.logical_bytes.to_string(),
        optional_u64(metrics.allocated_bytes),
        optional_u64(metrics.unique_logical_bytes),
        optional_u64(metrics.unique_allocated_bytes),
        metrics.files.to_string(),
        metrics.directories.to_string(),
    ]);
}

fn push_provenance(
    row: &mut Vec<String>,
    estimate_source: Option<&str>,
    provenance: &EstimateProvenance,
) {
    row.extend([
        estimate_source.unwrap_or_default().to_string(),
        provenance
            .estimate_backend
            .map(|backend| backend.label().to_string())
            .unwrap_or_default(),
        provenance
            .estimate_backend_source
            .clone()
            .unwrap_or_default(),
        provenance
            .estimate_confidence
            .map(|confidence| confidence.label().to_string())
            .unwrap_or_default(),
        provenance
            .estimate_fallback_reason
            .clone()
            .unwrap_or_default(),
        provenance
            .estimate_caveats
            .iter()
            .map(|caveat| caveat.code.as_str())
            .collect::<Vec<_>>()
            .join(";"),
    ]);
}

fn push_empty_provenance(row: &mut Vec<String>) {
    row.extend(std::iter::repeat_n(String::new(), 6));
}

fn optional_usize(value: Option<usize>) -> String {
    value.map(|value| value.to_string()).unwrap_or_default()
}

fn optional_u64(value: Option<u64>) -> String {
    value.map(|value| value.to_string()).unwrap_or_default()
}

fn write_table_row<W, I, S>(
    writer: &mut W,
    format: InspectMapTableFormat,
    fields: I,
) -> io::Result<()>
where
    W: Write,
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    for (index, field) in fields.into_iter().enumerate() {
        if index > 0 {
            match format {
                InspectMapTableFormat::Csv => writer.write_all(b",")?,
                InspectMapTableFormat::Tsv => writer.write_all(b"\t")?,
            }
        }
        write_table_field(writer, format, field.as_ref())?;
    }
    writer.write_all(b"\n")
}

fn write_table_field<W: Write>(
    writer: &mut W,
    format: InspectMapTableFormat,
    field: &str,
) -> io::Result<()> {
    match format {
        InspectMapTableFormat::Csv => write_csv_field(writer, field),
        InspectMapTableFormat::Tsv => {
            writer.write_all(normalized_table_field(format, field).as_bytes())
        }
    }
}

fn write_csv_field<W: Write>(writer: &mut W, field: &str) -> io::Result<()> {
    let normalized = normalized_table_field(InspectMapTableFormat::Csv, field);
    let needs_quotes = normalized.contains(',')
        || normalized.contains('"')
        || normalized.starts_with(' ')
        || normalized.ends_with(' ');

    if !needs_quotes {
        return writer.write_all(normalized.as_bytes());
    }

    writer.write_all(b"\"")?;
    for byte in normalized.bytes() {
        if byte == b'"' {
            writer.write_all(b"\"\"")?;
        } else {
            writer.write_all(&[byte])?;
        }
    }
    writer.write_all(b"\"")
}

fn normalized_table_field(format: InspectMapTableFormat, field: &str) -> Cow<'_, str> {
    let escape_tabs = matches!(format, InspectMapTableFormat::Tsv);
    let needs_escape = field
        .chars()
        .any(|value| matches!(value, '\r' | '\n') || (escape_tabs && value == '\t'));
    if !needs_escape {
        return Cow::Borrowed(field);
    }

    let mut normalized = String::with_capacity(field.len());
    for value in field.chars() {
        match value {
            '\r' => normalized.push_str("\\r"),
            '\n' => normalized.push_str("\\n"),
            '\t' if escape_tabs => normalized.push_str("\\t"),
            _ => normalized.push(value),
        }
    }
    Cow::Owned(normalized)
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

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use indicatif::ProgressBar;
use rebecca::core::config::{AppRuntimeConfig, load_runtime_config};
use rebecca::core::environment::SystemEnvironment;
use rebecca::core::executor::{
    PermanentDeleteBackend, execute_cleanup_plan_parallel_with_policy_and_cancellation,
};
use rebecca::core::external_rules::ExternalRuleStore;
use rebecca::core::history::HistoryStore;
use rebecca::core::plan::CleanupPlan;
use rebecca::core::planner::{
    PlanBuildContext, PlanProgressEvent, build_cleanup_plan_with_context,
};
use rebecca::core::protection::ProtectionPolicy;
use rebecca::core::scan::ScanBackendKind;
use rebecca::core::scan_cache::ScanCacheStore;
use rebecca::core::{
    DeleteMode, PlanRequest, Platform, RebeccaError, RuleDefinition, TargetStatus,
};

use crate::clean_view::ScanCacheProgressSummary;
use crate::cli::{OutputMode, ProgressDetail};
use crate::output::{
    HumanPlanRenderer, MachineErrorRendered, NdjsonEventWriter, WorkflowOutputContract,
    WorkflowSuccessRenderer, format_bytes,
};
use crate::progress::{
    HumanProgressThrottle, PROGRESS_PATH_MAX_CHARS, compact_progress_path, format_byte_rate,
    format_file_rate, stderr_spinner,
};
use crate::runtime::CliRuntime;
use crate::text::format_count;
use crate::trash_backend::recoverable_trash_backend;
use crate::{info, output, render};

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
    pub(crate) output_contract: WorkflowOutputContract,
    pub(crate) human_renderer: HumanPlanRenderer,
    pub(crate) success_renderer: WorkflowSuccessRenderer,
    pub(crate) cancellation_message: &'static str,
    pub(crate) confirmation_kind: ConfirmationKind,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum WorkflowRuleSource<'a> {
    RuleCatalog(&'a [RuleDefinition]),
    NativeWorkflow,
}

impl<'a> WorkflowRuleSource<'a> {
    fn rules(self) -> &'a [RuleDefinition] {
        match self {
            Self::RuleCatalog(rules) => rules,
            Self::NativeWorkflow => &[],
        }
    }
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
    let safety_knowledge =
        rebecca::rules::builtin_safety_knowledge_for_platform(options.request.platform)?;
    let cancellation = runtime.cancellation();
    let mut progress = PlanProgressReporter::new(
        options.output_mode.is_human() && !options.no_progress,
        options.progress_detail,
        options
            .output_mode
            .is_ndjson()
            .then(|| NdjsonEventWriter::with_contract(options.output_contract)),
    );
    let applications = info::application_discovery();
    let protected_storage = runtime_config.app_paths.storage_entries();
    let protected_paths = merged_protected_paths(
        runtime_config.protected_paths.as_slice(),
        options.exclude_paths.as_slice(),
    )?;
    let scan_cache_store = options
        .scan_cache
        .then(|| ScanCacheStore::from_app_paths(&runtime_config.app_paths));
    let mut context = PlanBuildContext::new(cancellation)
        .with_scan_backend(options.scan_backend)
        .with_safety_knowledge(&safety_knowledge)
        .with_protected_storage(&protected_storage);
    if !protected_paths.is_empty() {
        context = context.with_protected_paths(&protected_paths);
    }
    if options.scan_cache {
        context = context.with_scan_cache_policy(runtime_config.scan_cache_policy);
        if let Some(store) = &scan_cache_store {
            context = context.with_scan_cache(store);
        }
    }
    progress.started()?;
    let combined_rules;
    let rules = match options.rule_source {
        WorkflowRuleSource::RuleCatalog(rules) => {
            let external_rules =
                ExternalRuleStore::default_for_state_dir(&runtime_config.app_paths.state_dir)
                    .load_enabled_rules();
            for diagnostic in &external_rules.diagnostics {
                eprintln!("Warning: external rule skipped: {}", diagnostic.message);
            }
            if external_rules.rules.is_empty() {
                rules
            } else {
                combined_rules = rules
                    .iter()
                    .cloned()
                    .chain(external_rules.rules)
                    .collect::<Vec<_>>();
                &combined_rules
            }
        }
        WorkflowRuleSource::NativeWorkflow => options.rule_source.rules(),
    };
    let plan_result = build_cleanup_plan_with_context(
        &options.request,
        rules,
        &SystemEnvironment,
        applications.as_ref(),
        context,
        |event| progress.on_event(event),
    );
    progress.finish();
    if let Some(err) = progress.take_event_error() {
        return Err(err);
    }
    let mut plan = match plan_result {
        Ok(plan) => plan,
        Err(err) => {
            let event_writer = progress.into_event_writer();
            if matches!(&err, rebecca::core::RebeccaError::OperationCancelled(_)) {
                return finish_stream_with_cancellation(event_writer, options.cancellation_message);
            }

            return finish_stream_with_error(event_writer, err.into());
        }
    };

    let scan_cache_summary = options
        .output_mode
        .is_human()
        .then(|| progress.scan_cache_summary());
    let event_writer = progress.into_event_writer();

    if options.request.mode.is_dry_run() {
        return (options.success_renderer)(
            &plan,
            options.output_contract,
            options.output_mode,
            options.human_renderer,
            scan_cache_summary,
            event_writer,
        );
    }

    if plan.summary.allowed_targets == 0 {
        return (options.success_renderer)(
            &plan,
            options.output_contract,
            options.output_mode,
            options.human_renderer,
            scan_cache_summary,
            event_writer,
        );
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

    let mut execution_policy = ProtectionPolicy::new()
        .with_safety_knowledge(&safety_knowledge)
        .with_protected_storage(&protected_storage);
    if !protected_paths.is_empty() {
        execution_policy = execution_policy.with_protected_paths(&protected_paths);
    }
    let mut execution_report = match execute_plan(
        &mut plan,
        execution_policy,
        cancellation,
        options.request.mode,
    ) {
        Ok(report) => report,
        Err(RebeccaError::OperationCancelled(_)) => {
            return finish_stream_with_cancellation(event_writer, options.cancellation_message);
        }
        Err(err) => return finish_stream_with_error(event_writer, err.into()),
    };

    let history_append =
        HistoryStore::new(runtime_config.app_paths.history_file).append_plan_report(&plan);
    if let Some(warning) = history_append.warning {
        eprintln!("Warning: {}", warning.message);
        execution_report.push_warning(warning);
    }
    plan.execution_report = Some(execution_report);

    (options.success_renderer)(
        &plan,
        options.output_contract,
        options.output_mode,
        options.human_renderer,
        scan_cache_summary,
        event_writer,
    )
}

fn execute_plan(
    plan: &mut CleanupPlan,
    execution_policy: ProtectionPolicy<'_>,
    cancellation: &rebecca::core::scan::ScanCancellationToken,
    mode: DeleteMode,
) -> std::result::Result<rebecca::core::execution::ExecutionReport, RebeccaError> {
    match mode {
        DeleteMode::RecoverableDelete => {
            let backend = recoverable_trash_backend();
            execute_cleanup_plan_parallel_with_policy_and_cancellation(
                plan,
                &backend,
                execution_policy,
                cancellation,
            )
        }
        DeleteMode::PermanentDelete => {
            let backend = PermanentDeleteBackend;
            execute_cleanup_plan_parallel_with_policy_and_cancellation(
                plan,
                &backend,
                execution_policy,
                cancellation,
            )
        }
        DeleteMode::DryRun => unreachable!("dry-run returns before execution"),
    }
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

fn merged_protected_paths(config_paths: &[PathBuf], cli_paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut merged = Vec::with_capacity(config_paths.len() + cli_paths.len());
    for path in config_paths.iter().chain(cli_paths) {
        rebecca::core::config::validate_user_protected_path(path)
            .map_err(|message| anyhow!("invalid protected path {}: {message}", path.display()))?;
        if merged.iter().all(|existing| existing != path) {
            merged.push(path.clone());
        }
    }
    Ok(merged)
}

struct PlanProgressReporter {
    bar: Option<ProgressBar>,
    detail: ProgressDetail,
    event_writer: Option<NdjsonEventWriter>,
    event_error: Option<anyhow::Error>,
    scanned_targets: u64,
    planned_bytes: u64,
    current_target_started_at: Instant,
    human_file_progress: HumanProgressThrottle,
    scan_cache_summary: ScanCacheProgressSummary,
}

impl PlanProgressReporter {
    fn new(enabled: bool, detail: ProgressDetail, event_writer: Option<NdjsonEventWriter>) -> Self {
        let now = Instant::now();
        let bar = stderr_spinner(enabled, "plan | building cleanup plan");

        Self {
            bar,
            detail,
            event_writer,
            event_error: None,
            scanned_targets: 0,
            planned_bytes: 0,
            current_target_started_at: now,
            human_file_progress: HumanProgressThrottle::new(),
            scan_cache_summary: ScanCacheProgressSummary::default(),
        }
    }

    fn started(&mut self) -> Result<()> {
        if let Some(writer) = &mut self.event_writer {
            writer.emit_started()?;
        }
        Ok(())
    }

    fn on_event(&mut self, event: PlanProgressEvent<'_>) {
        self.record_event(event);

        if self.event_error.is_none()
            && let Some(writer) = &mut self.event_writer
            && self.detail.should_emit_machine_event(event)
            && let Err(err) = writer.emit_plan_progress(event)
        {
            self.event_error = Some(err);
        }

        let Some(bar) = &self.bar else {
            return;
        };

        match event {
            PlanProgressEvent::TargetScanning { rule_id, path } => {
                self.current_target_started_at = Instant::now();
                bar.set_message(target_scanning_message(
                    self.scanned_targets.saturating_add(1),
                    rule_id,
                    path,
                ));
            }
            PlanProgressEvent::TargetFinished {
                status,
                estimated_bytes,
                ..
            } => {
                self.scanned_targets = self.scanned_targets.saturating_add(1);
                self.planned_bytes = self.planned_bytes.saturating_add(estimated_bytes);
                bar.set_message(target_finished_message(
                    self.scanned_targets,
                    self.planned_bytes,
                    status,
                    estimated_bytes,
                ));
                bar.tick();
            }
            PlanProgressEvent::FileMeasured {
                rule_id,
                files_scanned,
                bytes_scanned,
                ..
            } => {
                if self.detail.includes_file_events() && self.human_file_progress.should_refresh() {
                    bar.set_message(file_progress_message(
                        rule_id,
                        files_scanned,
                        bytes_scanned,
                        self.current_target_started_at.elapsed(),
                    ));
                }
            }
            PlanProgressEvent::ScanCacheHit {
                rule_id,
                path,
                estimated_bytes,
            } => {
                bar.set_message(scan_cache_hit_message(
                    self.scan_cache_summary,
                    rule_id,
                    path,
                    estimated_bytes,
                ));
                bar.tick();
            }
            PlanProgressEvent::ScanCacheMiss {
                rule_id,
                path,
                reason,
                ..
            } => {
                bar.set_message(scan_cache_miss_message(
                    self.scan_cache_summary,
                    rule_id,
                    path,
                    reason.label(),
                ));
                bar.tick();
            }
            PlanProgressEvent::ScanCacheWriteSkipped { rule_id, path } => {
                bar.set_message(scan_cache_write_skipped_message(
                    self.scan_cache_summary,
                    rule_id,
                    path,
                ));
                bar.tick();
            }
            PlanProgressEvent::ScanCachePruned { report } => {
                bar.set_message(scan_cache_pruned_message(
                    self.scan_cache_summary,
                    report.pruned as u64,
                    report.inspected as u64,
                ));
                bar.tick();
            }
        }
    }

    fn record_event(&mut self, event: PlanProgressEvent<'_>) {
        match event {
            PlanProgressEvent::ScanCacheHit { .. } => {
                self.scan_cache_summary.hits = self.scan_cache_summary.hits.saturating_add(1);
            }
            PlanProgressEvent::ScanCacheMiss { pruned, .. } => {
                self.scan_cache_summary.misses = self.scan_cache_summary.misses.saturating_add(1);
                if pruned {
                    self.scan_cache_summary.pruned =
                        self.scan_cache_summary.pruned.saturating_add(1);
                }
            }
            PlanProgressEvent::ScanCacheWriteSkipped { .. } => {
                self.scan_cache_summary.write_skipped =
                    self.scan_cache_summary.write_skipped.saturating_add(1);
            }
            PlanProgressEvent::ScanCachePruned { report } => {
                self.scan_cache_summary.pruned = self
                    .scan_cache_summary
                    .pruned
                    .saturating_add(report.pruned as u64);
            }
            _ => {}
        }
    }

    fn scan_cache_summary(&self) -> ScanCacheProgressSummary {
        self.scan_cache_summary
    }

    fn finish(&self) {
        if let Some(bar) = &self.bar {
            bar.finish_and_clear();
        }
    }

    fn take_event_error(&mut self) -> Option<anyhow::Error> {
        self.event_error.take()
    }

    fn into_event_writer(self) -> Option<NdjsonEventWriter> {
        self.event_writer
    }
}

fn target_scanning_message(next_target: u64, rule_id: &str, path: &Path) -> String {
    format!(
        "plan | target {next_target} | scanning {rule_id} | {}",
        compact_progress_path(path, PROGRESS_PATH_MAX_CHARS)
    )
}

fn target_finished_message(
    scanned_targets: u64,
    planned_bytes: u64,
    status: TargetStatus,
    estimated_bytes: u64,
) -> String {
    format!(
        "plan | {} | {} found | last {}, {}",
        format_count(scanned_targets, "target", "targets"),
        format_bytes(planned_bytes),
        status.label(),
        format_bytes(estimated_bytes)
    )
}

fn file_progress_message(
    rule_id: &str,
    files_scanned: u64,
    bytes_scanned: u64,
    elapsed: Duration,
) -> String {
    format!(
        "scan | {rule_id} | {} | {} | {}, {}",
        format_count(files_scanned, "file", "files"),
        format_bytes(bytes_scanned),
        format_file_rate(files_scanned, elapsed),
        format_byte_rate(bytes_scanned, elapsed)
    )
}

fn scan_cache_hit_message(
    summary: ScanCacheProgressSummary,
    rule_id: &str,
    path: &Path,
    estimated_bytes: u64,
) -> String {
    format!(
        "cache | {} | hit {rule_id} | {} | {}",
        scan_cache_counts(summary),
        compact_progress_path(path, PROGRESS_PATH_MAX_CHARS),
        format_bytes(estimated_bytes)
    )
}

fn scan_cache_miss_message(
    summary: ScanCacheProgressSummary,
    rule_id: &str,
    path: &Path,
    reason_label: &str,
) -> String {
    format!(
        "cache | {} | miss {rule_id} | {} | {reason_label}",
        scan_cache_counts(summary),
        compact_progress_path(path, PROGRESS_PATH_MAX_CHARS)
    )
}

fn scan_cache_write_skipped_message(
    summary: ScanCacheProgressSummary,
    rule_id: &str,
    path: &Path,
) -> String {
    format!(
        "cache | {} | skip write {rule_id} | {}",
        scan_cache_counts(summary),
        compact_progress_path(path, PROGRESS_PATH_MAX_CHARS)
    )
}

fn scan_cache_pruned_message(
    summary: ScanCacheProgressSummary,
    pruned: u64,
    inspected: u64,
) -> String {
    format!(
        "cache | {} | pruned {} after {}",
        scan_cache_counts(summary),
        format_count(pruned, "record", "records"),
        format_count(inspected, "inspection", "inspections")
    )
}

fn scan_cache_counts(summary: ScanCacheProgressSummary) -> String {
    format!(
        "{}, {}",
        format_count(summary.hits, "hit", "hits"),
        format_count(summary.misses, "miss", "misses")
    )
}

impl ProgressDetail {
    fn should_emit_machine_event(self, event: PlanProgressEvent<'_>) -> bool {
        !matches!(event, PlanProgressEvent::FileMeasured { .. }) || self.includes_file_events()
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_target_scanning_message_keeps_path_bounded() {
        let long_path = Path::new(
            r"C:\Users\Rebecca\AppData\Local\VeryLongVendorName\VeryLongProductName\Cache\Nested\Target",
        );

        let message = target_scanning_message(3, "windows.user-temp", long_path);

        assert!(message.starts_with("plan | target 3 | scanning windows.user-temp | "));
        let path = message
            .strip_prefix("plan | target 3 | scanning windows.user-temp | ")
            .expect("progress message should keep the path as the final segment");
        assert!(path.starts_with("..."));
        assert!(path.ends_with(r"\Cache\Nested\Target"));
        assert!(path.chars().count() <= PROGRESS_PATH_MAX_CHARS);
    }

    #[test]
    fn progress_target_finished_message_summarizes_targets_and_bytes() {
        let message = target_finished_message(2, 1536, TargetStatus::Allowed, 512);

        assert_eq!(
            message,
            "plan | 2 targets | 1.50 KiB found | last allowed, 512 B"
        );
    }

    #[test]
    fn progress_file_message_includes_scan_rates() {
        let message = file_progress_message("windows.user-temp", 4, 20, Duration::from_secs(1));

        assert_eq!(
            message,
            "scan | windows.user-temp | 4 files | 20 B | 4.0 files/s, 20 B/s"
        );
    }

    #[test]
    fn progress_cache_hit_message_includes_cache_counts() {
        let summary = ScanCacheProgressSummary {
            hits: 1,
            misses: 0,
            write_skipped: 0,
            pruned: 0,
        };

        let message =
            scan_cache_hit_message(summary, "windows.edge-cache", Path::new(r"C:\Cache"), 2048);

        assert_eq!(
            message,
            r"cache | 1 hit, 0 misses | hit windows.edge-cache | C:\Cache | 2.00 KiB"
        );
    }

    #[test]
    fn compact_progress_text_handles_tiny_widths() {
        assert_eq!(crate::progress::compact_progress_text("abcdef", 2), "..");
        assert_eq!(crate::progress::compact_progress_text("abcdef", 4), "...f");
    }
}

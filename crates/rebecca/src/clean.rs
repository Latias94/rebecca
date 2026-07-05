use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use indicatif::ProgressBar;
use rebecca::core::config::{AppRuntimeConfig, load_runtime_config};
use rebecca::core::environment::SystemEnvironment;
use rebecca::core::executor::execute_cleanup_plan_parallel_with_policy;
use rebecca::core::history::HistoryStore;
use rebecca::core::plan::CleanupPlan;
use rebecca::core::planner::{
    PlanBuildContext, PlanProgressEvent, build_cleanup_plan_with_context,
};
use rebecca::core::protection::ProtectionPolicy;
use rebecca::core::scan::ScanBackendKind;
use rebecca::core::scan_cache::ScanCacheStore;
use rebecca::core::{DeleteMode, PlanRequest, Platform, RuleDefinition};

use crate::clean_view::ScanCacheProgressSummary;
use crate::cli::{OutputMode, ProgressDetail};
use crate::output::{
    HumanPlanRenderer, MachineErrorRendered, NdjsonEventWriter, WorkflowOutputContract,
    WorkflowSuccessRenderer, format_bytes,
};
use crate::runtime::CliRuntime;
use crate::{info, output, render};

#[derive(Debug)]
pub struct CleanOptions {
    pub dry_run: bool,
    pub output_mode: OutputMode,
    pub yes: bool,
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
    #[cfg_attr(windows, allow(dead_code))]
    pub(crate) unsupported_execution_message: &'static str,
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
    let mode = if options.yes && !options.dry_run {
        DeleteMode::RecycleBin
    } else {
        DeleteMode::DryRun
    };

    let mut request = PlanRequest::for_platform(Platform::Windows, mode);
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
            unsupported_execution_message: "cleanup execution is Windows-only at this stage; omit --yes to preview",
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
    let safety_knowledge = rebecca::rules::builtin_safety_knowledge()?;
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
    let plan_result = build_cleanup_plan_with_context(
        &options.request,
        options.rule_source.rules(),
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

    #[cfg(not(windows))]
    {
        let err = rebecca::core::RebeccaError::PlatformUnavailable(
            options.unsupported_execution_message.to_string(),
        )
        .into();
        return finish_stream_with_error(event_writer, err);
    }

    #[cfg(windows)]
    {
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

        let backend = rebecca::windows::WindowsRecycleBinBackend::new();
        let mut execution_policy = ProtectionPolicy::new()
            .with_safety_knowledge(&safety_knowledge)
            .with_protected_storage(&protected_storage);
        if !protected_paths.is_empty() {
            execution_policy = execution_policy.with_protected_paths(&protected_paths);
        }
        let mut execution_report = match execute_cleanup_plan_parallel_with_policy(
            &mut plan,
            &backend,
            execution_policy,
        ) {
            Ok(report) => report,
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
    human_file_progress: HumanFileProgressThrottle,
    scan_cache_summary: ScanCacheProgressSummary,
}

impl PlanProgressReporter {
    fn new(enabled: bool, detail: ProgressDetail, event_writer: Option<NdjsonEventWriter>) -> Self {
        let bar = enabled.then(|| {
            let bar = ProgressBar::new_spinner();
            bar.enable_steady_tick(Duration::from_millis(120));
            bar.set_message("Building cleanup plan");
            bar
        });

        Self {
            bar,
            detail,
            event_writer,
            event_error: None,
            scanned_targets: 0,
            human_file_progress: HumanFileProgressThrottle::new(),
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
                bar.set_message(format!("Scanning {rule_id}: {}", path.display()));
            }
            PlanProgressEvent::TargetFinished {
                status,
                estimated_bytes,
                ..
            } => {
                self.scanned_targets = self.scanned_targets.saturating_add(1);
                bar.set_message(format!(
                    "Scanned {} target(s); last {status:?}, {} bytes",
                    self.scanned_targets, estimated_bytes
                ));
                bar.tick();
            }
            PlanProgressEvent::FileMeasured {
                files_scanned,
                bytes_scanned,
                ..
            } => {
                if self.detail.includes_file_events() && self.human_file_progress.should_refresh() {
                    bar.set_message(format!(
                        "Scanning files: {files_scanned}, {}",
                        format_bytes(bytes_scanned)
                    ));
                }
            }
            PlanProgressEvent::ScanCacheHit {
                rule_id,
                path,
                estimated_bytes,
            } => {
                bar.set_message(format!(
                    "Scan cache hit {rule_id}: {} ({})",
                    path.display(),
                    format_bytes(estimated_bytes)
                ));
                bar.tick();
            }
            PlanProgressEvent::ScanCacheMiss {
                rule_id,
                path,
                reason,
                ..
            } => {
                bar.set_message(format!(
                    "Scan cache miss {rule_id}: {} ({})",
                    path.display(),
                    reason.label()
                ));
                bar.tick();
            }
            PlanProgressEvent::ScanCacheWriteSkipped { rule_id, path } => {
                bar.set_message(format!(
                    "Scan cache write skipped {rule_id}: {}",
                    path.display()
                ));
                bar.tick();
            }
            PlanProgressEvent::ScanCachePruned { report } => {
                bar.set_message(format!(
                    "Scan cache pruned {} record(s) after inspecting {}",
                    report.pruned, report.inspected
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

impl ProgressDetail {
    fn should_emit_machine_event(self, event: PlanProgressEvent<'_>) -> bool {
        !matches!(event, PlanProgressEvent::FileMeasured { .. }) || self.includes_file_events()
    }
}

#[derive(Debug)]
struct HumanFileProgressThrottle {
    events_since_refresh: u64,
    last_refresh: Instant,
}

impl HumanFileProgressThrottle {
    const FILE_INTERVAL: u64 = 64;
    const TIME_INTERVAL: Duration = Duration::from_millis(250);

    fn new() -> Self {
        Self {
            events_since_refresh: 0,
            last_refresh: Instant::now(),
        }
    }

    fn should_refresh(&mut self) -> bool {
        self.events_since_refresh = self.events_since_refresh.saturating_add(1);
        if self.events_since_refresh < Self::FILE_INTERVAL
            && self.last_refresh.elapsed() < Self::TIME_INTERVAL
        {
            return false;
        }

        self.events_since_refresh = 0;
        self.last_refresh = Instant::now();
        true
    }
}

#[cfg(windows)]
fn confirm_cleanup(plan: &CleanupPlan, kind: ConfirmationKind) -> Result<bool> {
    let prompt = match kind {
        ConfirmationKind::Cleanup => format!(
            "Move {} target(s), {} bytes, to the Recycle Bin?",
            plan.summary.allowed_targets, plan.summary.estimated_bytes
        ),
        ConfirmationKind::AppLeftovers => format!(
            "Move {} app leftover target(s), {} bytes, to the Recycle Bin?",
            plan.summary.allowed_targets, plan.summary.estimated_bytes
        ),
        ConfirmationKind::ProjectArtifacts => format!(
            "Move {} project artifact target(s), {} bytes, to the Recycle Bin?",
            plan.summary.allowed_targets, plan.summary.estimated_bytes
        ),
    };

    dialoguer::Confirm::new()
        .with_prompt(prompt)
        .default(false)
        .interact()
        .context("cleanup confirmation failed")
}

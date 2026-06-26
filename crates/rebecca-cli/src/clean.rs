use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use indicatif::ProgressBar;
use rebecca_core::config::load_runtime_config;
use rebecca_core::executor::execute_cleanup_plan_with_policy;
use rebecca_core::history::HistoryStore;
use rebecca_core::plan::CleanupPlan;
use rebecca_core::planner::{PlanBuildContext, PlanProgressEvent, build_cleanup_plan_with_context};
use rebecca_core::protection::ProtectionPolicy;
use rebecca_core::scan::ScanCancellationToken;
use rebecca_core::scan_cache::ScanCacheStore;
use rebecca_core::{DeleteMode, PlanRequest, Platform, RuleDefinition};

use crate::clean_view::ScanCacheProgressSummary;
use crate::output::format_bytes;
use crate::{info, output};
use rebecca_core::environment::SystemEnvironment;

#[derive(Debug)]
pub struct CleanOptions {
    pub dry_run: bool,
    pub json: bool,
    pub yes: bool,
    pub no_progress: bool,
    pub scan_cache: bool,
    pub categories: Vec<String>,
    pub rules: Vec<String>,
    pub exclude_paths: Vec<PathBuf>,
    pub allow_moderate: bool,
    pub allow_risky: bool,
}

pub(crate) struct WorkflowRunOptions<'a> {
    pub(crate) request: PlanRequest,
    pub(crate) rules: &'a [RuleDefinition],
    pub(crate) json: bool,
    pub(crate) yes: bool,
    pub(crate) no_progress: bool,
    pub(crate) scan_cache: bool,
    pub(crate) exclude_paths: Vec<PathBuf>,
    pub(crate) cancellation_message: &'static str,
    #[cfg_attr(windows, allow(dead_code))]
    pub(crate) unsupported_execution_message: &'static str,
    pub(crate) confirmation_kind: ConfirmationKind,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum ConfirmationKind {
    Cleanup,
    AppLeftovers,
    ProjectArtifacts,
}

pub fn run(options: CleanOptions) -> Result<()> {
    let mode = if options.dry_run {
        DeleteMode::DryRun
    } else {
        DeleteMode::RecycleBin
    };

    let mut request = PlanRequest::for_platform(Platform::Windows, mode);
    request.selected_categories = options.categories;
    request.selected_rule_ids = options.rules;
    request.allow_moderate = options.allow_moderate;
    request.allow_risky = options.allow_risky;

    let catalog = rebecca_rules::builtin_rules()?;
    run_workflow(WorkflowRunOptions {
        request,
        rules: &catalog,
        json: options.json,
        yes: options.yes,
        no_progress: options.no_progress,
        scan_cache: options.scan_cache,
        exclude_paths: options.exclude_paths,
        cancellation_message: "Cleanup cancelled.",
        unsupported_execution_message: "cleanup execution is Windows-only at this stage; use --dry-run to preview",
        confirmation_kind: ConfirmationKind::Cleanup,
    })
}

pub(crate) fn run_workflow(options: WorkflowRunOptions<'_>) -> Result<()> {
    let cancellation = ScanCancellationToken::new();
    install_cancellation_handler(cancellation.clone())?;
    let mut progress = PlanProgressReporter::new(!options.json && !options.no_progress);
    let applications = info::application_discovery();
    let runtime_config = load_runtime_config()?;
    let protected_storage = runtime_config.app_paths.storage_entries();
    let protected_paths = merged_protected_paths(
        runtime_config.protected_paths.as_slice(),
        options.exclude_paths.as_slice(),
    )?;
    let scan_cache_store = options
        .scan_cache
        .then(|| ScanCacheStore::from_app_paths(&runtime_config.app_paths));
    let mut context =
        PlanBuildContext::new(&cancellation).with_protected_storage(&protected_storage);
    if !protected_paths.is_empty() {
        context = context.with_protected_paths(&protected_paths);
    }
    if options.scan_cache {
        context = context.with_scan_cache_policy(runtime_config.scan_cache_policy);
        if let Some(store) = &scan_cache_store {
            context = context.with_scan_cache(store);
        }
    }
    let plan_result = build_cleanup_plan_with_context(
        &options.request,
        options.rules,
        &SystemEnvironment,
        applications.as_ref(),
        context,
        |event| progress.on_event(event),
    );
    progress.finish();
    let mut plan = match plan_result {
        Ok(plan) => plan,
        Err(err) => {
            if matches!(&err, rebecca_core::RebeccaError::OperationCancelled(_)) {
                println!("{}", options.cancellation_message);
                return Ok(());
            }

            return Err(err.into());
        }
    };

    let scan_cache_summary = (!options.json).then(|| progress.scan_cache_summary());

    if options.request.mode.is_dry_run() {
        return output::print_plan(&plan, options.json, scan_cache_summary);
    }

    #[cfg(not(windows))]
    {
        return Err(rebecca_core::RebeccaError::PlatformUnavailable(
            options.unsupported_execution_message.to_string(),
        )
        .into());
    }

    #[cfg(windows)]
    {
        if plan.summary.allowed_targets == 0 {
            return output::print_plan(&plan, options.json, scan_cache_summary);
        }

        if !options.yes && !confirm_cleanup(&plan, options.confirmation_kind)? {
            println!("{}", options.cancellation_message);
            return Ok(());
        }

        let backend = rebecca_windows::WindowsRecycleBinBackend::new();
        let mut execution_policy =
            ProtectionPolicy::new().with_protected_storage(&protected_storage);
        if !protected_paths.is_empty() {
            execution_policy = execution_policy.with_protected_paths(&protected_paths);
        }
        execute_cleanup_plan_with_policy(&mut plan, &backend, execution_policy)?;

        HistoryStore::new(runtime_config.app_paths.history_file).append_plan(&plan)?;

        output::print_plan(&plan, options.json, scan_cache_summary)
    }
}

fn merged_protected_paths(config_paths: &[PathBuf], cli_paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut merged = Vec::with_capacity(config_paths.len() + cli_paths.len());
    for path in config_paths.iter().chain(cli_paths) {
        rebecca_core::config::validate_user_protected_path(path)
            .map_err(|message| anyhow!("invalid protected path {}: {message}", path.display()))?;
        if merged.iter().all(|existing| existing != path) {
            merged.push(path.clone());
        }
    }
    Ok(merged)
}

fn install_cancellation_handler(token: ScanCancellationToken) -> Result<()> {
    ctrlc::set_handler(move || token.cancel()).context("failed to install Ctrl+C handler")?;
    Ok(())
}

struct PlanProgressReporter {
    bar: Option<ProgressBar>,
    scanned_targets: u64,
    scan_cache_summary: ScanCacheProgressSummary,
}

impl PlanProgressReporter {
    fn new(enabled: bool) -> Self {
        let bar = enabled.then(|| {
            let bar = ProgressBar::new_spinner();
            bar.enable_steady_tick(Duration::from_millis(120));
            bar.set_message("Building cleanup plan");
            bar
        });

        Self {
            bar,
            scanned_targets: 0,
            scan_cache_summary: ScanCacheProgressSummary::default(),
        }
    }

    fn on_event(&mut self, event: PlanProgressEvent<'_>) {
        self.record_event(event);

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
                bar.set_message(format!(
                    "Scanning files: {files_scanned}, {}",
                    format_bytes(bytes_scanned)
                ));
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
        }
    }

    fn record_event(&mut self, event: PlanProgressEvent<'_>) {
        match event {
            PlanProgressEvent::ScanCacheHit { .. } => {
                self.scan_cache_summary.hits = self.scan_cache_summary.hits.saturating_add(1);
            }
            PlanProgressEvent::ScanCacheMiss { .. } => {
                self.scan_cache_summary.misses = self.scan_cache_summary.misses.saturating_add(1);
            }
            PlanProgressEvent::ScanCacheWriteSkipped { .. } => {
                self.scan_cache_summary.write_skipped =
                    self.scan_cache_summary.write_skipped.saturating_add(1);
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

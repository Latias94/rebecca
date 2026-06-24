use std::time::Duration;

use anyhow::{Context, Result};
use indicatif::ProgressBar;
use rebecca_core::config::load_app_paths;
use rebecca_core::executor::execute_cleanup_plan;
use rebecca_core::history::HistoryStore;
use rebecca_core::plan::CleanupPlan;
use rebecca_core::planner::{PlanBuildContext, PlanProgressEvent, build_cleanup_plan_with_context};
use rebecca_core::scan::ScanCancellationToken;
use rebecca_core::scan_cache::ScanCacheStore;
use rebecca_core::{DeleteMode, PlanRequest, Platform};

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
    pub allow_moderate: bool,
    pub allow_risky: bool,
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
    let cancellation = ScanCancellationToken::new();
    install_cancellation_handler(cancellation.clone())?;
    let mut progress = PlanProgressReporter::new(!options.json && !options.no_progress);
    let applications = info::steam_application_discovery();
    let app_paths = if options.scan_cache || !options.dry_run {
        Some(load_app_paths()?)
    } else {
        None
    };
    let scan_cache_store = app_paths
        .as_ref()
        .filter(|_| options.scan_cache)
        .map(ScanCacheStore::from_app_paths);
    let mut context = PlanBuildContext::new(&cancellation);
    if let Some(store) = &scan_cache_store {
        context = context.with_scan_cache(store);
    }
    let plan_result = build_cleanup_plan_with_context(
        &request,
        &catalog,
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
                println!("Cleanup cancelled.");
                return Ok(());
            }

            return Err(err.into());
        }
    };

    if options.dry_run {
        return output::print_plan(&plan, options.json);
    }

    #[cfg(not(windows))]
    {
        return Err(rebecca_core::RebeccaError::PlatformUnavailable(
            "cleanup execution is Windows-only at this stage; use --dry-run to preview".to_string(),
        )
        .into());
    }

    #[cfg(windows)]
    {
        if plan.summary.allowed_targets == 0 {
            return output::print_plan(&plan, options.json);
        }

        if !options.yes && !confirm_cleanup(&plan)? {
            println!("Cleanup cancelled.");
            return Ok(());
        }

        let backend = rebecca_windows::WindowsRecycleBinBackend::new();
        execute_cleanup_plan(&mut plan, &backend)?;

        let paths = match app_paths {
            Some(paths) => paths,
            None => load_app_paths()?,
        };
        HistoryStore::new(paths.history_file).append_plan(&plan)?;

        output::print_plan(&plan, options.json)
    }
}

fn install_cancellation_handler(token: ScanCancellationToken) -> Result<()> {
    ctrlc::set_handler(move || token.cancel()).context("failed to install Ctrl+C handler")?;
    Ok(())
}

struct PlanProgressReporter {
    bar: Option<ProgressBar>,
    scanned_targets: u64,
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
        }
    }

    fn on_event(&mut self, event: PlanProgressEvent<'_>) {
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

    fn finish(&self) {
        if let Some(bar) = &self.bar {
            bar.finish_and_clear();
        }
    }
}

#[cfg(windows)]
fn confirm_cleanup(plan: &CleanupPlan) -> Result<bool> {
    dialoguer::Confirm::new()
        .with_prompt(format!(
            "Move {} target(s), {} bytes, to the Recycle Bin?",
            plan.summary.allowed_targets, plan.summary.estimated_bytes
        ))
        .default(false)
        .interact()
        .context("cleanup confirmation failed")
}

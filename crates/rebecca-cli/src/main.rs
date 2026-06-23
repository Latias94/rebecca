use std::time::Duration;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use indicatif::ProgressBar;
use rebecca_core::applications::ApplicationDiscovery;
#[cfg(debug_assertions)]
use rebecca_core::applications::{
    NoopApplicationDiscovery, StaticApplicationDiscovery, SteamInstallation,
};
use rebecca_core::config::default_app_paths;
use rebecca_core::environment::SystemEnvironment;
use rebecca_core::executor::execute_cleanup_plan;
use rebecca_core::history::HistoryStore;
use rebecca_core::planner::{
    PlanProgressEvent, build_cleanup_plan_with_environment_applications_progress_and_cancellation,
};
use rebecca_core::scan::ScanCancellationToken;
use rebecca_core::{DeleteMode, PlanRequest, Platform, RuleSelection};

mod info;
mod output;
use crate::output::format_bytes;

#[derive(Debug, Parser)]
#[command(name = "rebecca", version, about = "Windows-first cleanup CLI")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Show the built-in cleanup rules that would be considered.
    Scan {
        /// Render machine-readable JSON.
        #[arg(long)]
        json: bool,
        /// Include a category. Can be repeated.
        #[arg(long = "category")]
        categories: Vec<String>,
        /// Include a specific rule id. Can be repeated.
        #[arg(long = "rule")]
        rules: Vec<String>,
    },
    /// Build or execute a cleanup plan.
    Clean {
        /// Preview the cleanup plan without deleting anything.
        #[arg(short = 'n', long)]
        dry_run: bool,
        /// Render machine-readable JSON.
        #[arg(long)]
        json: bool,
        /// Execute without an interactive confirmation prompt.
        #[arg(long)]
        yes: bool,
        /// Disable human progress output while building the cleanup plan.
        #[arg(long)]
        no_progress: bool,
        /// Include a category. Can be repeated.
        #[arg(long = "category")]
        categories: Vec<String>,
        /// Include a specific rule id. Can be repeated.
        #[arg(long = "rule")]
        rules: Vec<String>,
        /// Include moderate-risk rules.
        #[arg(long)]
        allow_moderate: bool,
        /// Include risky rules.
        #[arg(long)]
        allow_risky: bool,
    },
    /// Show cleanup history.
    History {
        /// Render machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Inspect configuration and local state locations.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Inspect host capabilities and permissions.
    Doctor {
        #[command(subcommand)]
        command: DoctorCommand,
    },
}

#[derive(Debug, Subcommand)]
enum ConfigCommand {
    /// Print config, state, cache, and history paths.
    Paths {
        /// Render machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
enum DoctorCommand {
    /// Print the current Windows privilege level when available.
    Permissions,
    /// Print the Steam installation and library discovery results when available.
    Steam,
}

fn main() -> Result<()> {
    init_tracing();

    let cli = Cli::parse();
    match cli.command.unwrap_or(Command::Scan {
        json: false,
        categories: Vec::new(),
        rules: Vec::new(),
    }) {
        Command::Scan {
            json,
            categories,
            rules,
        } => scan(json, categories, rules),
        Command::Clean {
            dry_run,
            json,
            yes,
            no_progress,
            categories,
            rules,
            allow_moderate,
            allow_risky,
        } => clean(CleanOptions {
            dry_run,
            json,
            yes,
            no_progress,
            categories,
            rules,
            allow_moderate,
            allow_risky,
        }),
        Command::History { json } => info::print_history(json),
        Command::Config { command } => match command {
            ConfigCommand::Paths { json } => info::print_config_paths(json),
        },
        Command::Doctor { command } => match command {
            DoctorCommand::Permissions => info::print_privilege_level(),
            DoctorCommand::Steam => info::print_steam_discovery(&*steam_application_discovery()),
        },
    }
}

fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();
}

fn scan(json: bool, categories: Vec<String>, rules: Vec<String>) -> Result<()> {
    let catalog = rebecca_rules::builtin_rules()?;
    let selection = RuleSelection::new(categories, rules);
    let filtered = catalog
        .iter()
        .filter(|rule| selection.matches_rule(rule))
        .collect::<Vec<_>>();

    if json {
        println!("{}", serde_json::to_string_pretty(&filtered)?);
        return Ok(());
    }

    output::print_rule_catalog(&filtered);

    Ok(())
}

struct CleanOptions {
    dry_run: bool,
    json: bool,
    yes: bool,
    no_progress: bool,
    categories: Vec<String>,
    rules: Vec<String>,
    allow_moderate: bool,
    allow_risky: bool,
}

fn clean(options: CleanOptions) -> Result<()> {
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
    let applications = steam_application_discovery();
    let plan_result = build_cleanup_plan_with_environment_applications_progress_and_cancellation(
        &request,
        &catalog,
        &SystemEnvironment,
        applications.as_ref(),
        &cancellation,
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

        let paths = default_app_paths()?;
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
        }
    }

    fn finish(&self) {
        if let Some(bar) = &self.bar {
            bar.finish_and_clear();
        }
    }
}

#[cfg(windows)]
fn confirm_cleanup(plan: &rebecca_core::plan::CleanupPlan) -> Result<bool> {
    dialoguer::Confirm::new()
        .with_prompt(format!(
            "Move {} target(s), {} bytes, to the Recycle Bin?",
            plan.summary.allowed_targets, plan.summary.estimated_bytes
        ))
        .default(false)
        .interact()
        .context("cleanup confirmation failed")
}

fn steam_application_discovery() -> Box<dyn ApplicationDiscovery> {
    if let Some(applications) = steam_application_discovery_override() {
        return applications;
    }

    #[cfg(windows)]
    {
        Box::new(rebecca_windows::steam::WindowsApplicationDiscovery::new())
    }

    #[cfg(not(windows))]
    {
        Box::new(rebecca_core::applications::NoopApplicationDiscovery::new())
    }
}

#[cfg(debug_assertions)]
fn steam_application_discovery_override() -> Option<Box<dyn ApplicationDiscovery>> {
    let discovery = std::env::var("REBECCA_STEAM_DISCOVERY").ok();
    if discovery.as_deref().is_some_and(|value| {
        value.eq_ignore_ascii_case("none") || value.eq_ignore_ascii_case("disabled")
    }) {
        return Some(Box::new(NoopApplicationDiscovery::new()));
    }

    let path = std::env::var("REBECCA_STEAM_DISCOVERY_PATH").ok()?;
    let path = path.trim();
    if path.is_empty() {
        return Some(Box::new(NoopApplicationDiscovery::new()));
    }

    match SteamInstallation::from_install_path(path) {
        Ok(installation) => Some(Box::new(
            StaticApplicationDiscovery::new().with_steam_installation(installation),
        )),
        Err(_) => Some(Box::new(NoopApplicationDiscovery::new())),
    }
}

#[cfg(not(debug_assertions))]
fn steam_application_discovery_override() -> Option<Box<dyn ApplicationDiscovery>> {
    None
}

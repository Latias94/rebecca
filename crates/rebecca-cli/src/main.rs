use std::collections::BTreeMap;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use indicatif::ProgressBar;
use rebecca_core::config::default_app_paths;
use rebecca_core::environment::SystemEnvironment;
use rebecca_core::executor::execute_cleanup_plan;
use rebecca_core::history::HistoryStore;
use rebecca_core::plan::{CleanupPlan, CleanupTarget};
use rebecca_core::planner::{
    PlanProgressEvent, build_cleanup_plan_with_environment_and_progress_and_cancellation,
};
use rebecca_core::scan::ScanCancellationToken;
use rebecca_core::{
    DeleteMode, PlanRequest, Platform, RuleDefinition, RuleSelection, TargetStatus,
};

const LARGEST_TARGET_LIMIT: usize = 5;

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
        Command::History { json } => history(json),
        Command::Config { command } => match command {
            ConfigCommand::Paths { json } => config_paths(json),
        },
        Command::Doctor { command } => match command {
            DoctorCommand::Permissions => doctor_permissions(),
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

    print_rule_catalog(&filtered);

    Ok(())
}

fn print_rule_catalog(rules: &[&RuleDefinition]) {
    println!("Rebecca rules: {}", rules.len());

    if rules.is_empty() {
        println!("No built-in rules match the current selection.");
        return;
    }

    let mut grouped: BTreeMap<String, Vec<&RuleDefinition>> = BTreeMap::new();
    for rule in rules {
        grouped
            .entry(rule.category.clone())
            .or_default()
            .push(*rule);
    }

    for rules in grouped.values_mut() {
        rules.sort_by(|left, right| left.id.cmp(&right.id));
    }

    for (category, rules) in grouped {
        println!("- {} ({})", category, rules.len());
        for rule in rules {
            println!("  - {} [{:?}] {}", rule.id, rule.safety_level, rule.name);
        }
    }
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
    let plan_result = build_cleanup_plan_with_environment_and_progress_and_cancellation(
        &request,
        &catalog,
        &SystemEnvironment,
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
        return print_plan(&plan, options.json);
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
            return print_plan(&plan, options.json);
        }

        if !options.yes && !confirm_cleanup(&plan)? {
            println!("Cleanup cancelled.");
            return Ok(());
        }

        let backend = rebecca_windows::WindowsRecycleBinBackend::new();
        execute_cleanup_plan(&mut plan, &backend)?;

        let paths = default_app_paths()?;
        HistoryStore::new(paths.history_file).append_plan(&plan)?;

        print_plan(&plan, options.json)
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

fn print_plan(plan: &CleanupPlan, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(plan)?);
        return Ok(());
    }

    println!("Cleanup mode: {:?}", plan.request.mode);
    println!("Targets: {}", plan.summary.total_targets);
    println!("Allowed: {}", plan.summary.allowed_targets);
    println!("Skipped: {}", plan.summary.skipped_targets);
    println!("Blocked: {}", plan.summary.blocked_targets);
    println!("Failed: {}", plan.summary.failed_targets);
    println!("Completed: {}", plan.summary.completed_targets);
    println!(
        "Estimated bytes: {} ({})",
        plan.summary.estimated_bytes,
        format_bytes(plan.summary.estimated_bytes)
    );
    println!(
        "Freed bytes: {} ({})",
        plan.summary.freed_bytes,
        format_bytes(plan.summary.freed_bytes)
    );
    println!(
        "Pending reclaim bytes: {} ({})",
        plan.summary.pending_reclaim_bytes,
        format_bytes(plan.summary.pending_reclaim_bytes)
    );

    print_largest_targets(plan);
    print_targets_by_status(plan);

    Ok(())
}

fn print_largest_targets(plan: &CleanupPlan) {
    let mut targets = plan
        .targets
        .iter()
        .filter(|target| target.estimated_bytes > 0)
        .collect::<Vec<_>>();

    targets.sort_by(|left, right| {
        right
            .estimated_bytes
            .cmp(&left.estimated_bytes)
            .then_with(|| left.rule_id.cmp(&right.rule_id))
            .then_with(|| left.path.cmp(&right.path))
    });

    if targets.is_empty() {
        return;
    }

    println!();
    println!("Largest estimated targets:");
    for target in targets.into_iter().take(LARGEST_TARGET_LIMIT) {
        print_target_line(target, "  -");
    }
}

fn print_targets_by_status(plan: &CleanupPlan) {
    if plan.targets.is_empty() {
        return;
    }

    println!();
    println!("Target details:");

    for status in [
        TargetStatus::Allowed,
        TargetStatus::Completed,
        TargetStatus::Failed,
        TargetStatus::Blocked,
        TargetStatus::Skipped,
    ] {
        let targets = plan
            .targets
            .iter()
            .filter(|target| target.status == status)
            .collect::<Vec<_>>();

        if targets.is_empty() {
            continue;
        }

        println!("{status:?} ({})", targets.len());
        for target in targets {
            print_target_line(target, "  -");
        }
    }
}

fn print_target_line(target: &CleanupTarget, prefix: &str) {
    println!(
        "{prefix} {} [{}] {} bytes ({}){}",
        target.rule_id,
        target.path.display(),
        target.estimated_bytes,
        format_bytes(target.estimated_bytes),
        target
            .reason
            .as_ref()
            .map(|reason| format!(" ({reason})"))
            .unwrap_or_default()
    );
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];

    if bytes < 1024 {
        return format!("{bytes} B");
    }

    let mut value = bytes as f64;
    let mut unit_index = 0;
    while value >= 1024.0 && unit_index < UNITS.len() - 1 {
        value /= 1024.0;
        unit_index += 1;
    }

    format!("{value:.2} {}", UNITS[unit_index])
}

fn history(json: bool) -> Result<()> {
    let paths = default_app_paths()?;
    let store = HistoryStore::new(paths.history_file);
    let entries = store.load()?;

    if json {
        println!("{}", serde_json::to_string_pretty(&entries)?);
        return Ok(());
    }

    if entries.is_empty() {
        println!("No cleanup history found.");
        return Ok(());
    }

    println!("Cleanup history: {} run(s)", entries.len());
    for entry in entries {
        println!(
            "- {}: {} completed, {} failed, {} pending bytes",
            entry.recorded_at_unix_seconds,
            entry.summary.completed_targets,
            entry.summary.failed_targets,
            entry.summary.pending_reclaim_bytes
        );
    }

    Ok(())
}

fn config_paths(json: bool) -> Result<()> {
    let paths = default_app_paths()?;

    if json {
        println!("{}", serde_json::to_string_pretty(&paths)?);
        return Ok(());
    }

    println!("Config file: {}", paths.config_file.display());
    println!("Config dir:  {}", paths.config_dir.display());
    println!("State dir:   {}", paths.state_dir.display());
    println!("Cache dir:   {}", paths.cache_dir.display());
    println!("History:     {}", paths.history_file.display());

    Ok(())
}

fn doctor_permissions() -> Result<()> {
    println!("Privilege level: {}", current_privilege_label());
    Ok(())
}

#[cfg(windows)]
fn current_privilege_label() -> &'static str {
    match rebecca_windows::current_privilege_level() {
        rebecca_windows::PrivilegeLevel::StandardUser => "standard-user",
        rebecca_windows::PrivilegeLevel::Elevated => "elevated",
        rebecca_windows::PrivilegeLevel::Unknown => "unknown",
    }
}

#[cfg(not(windows))]
fn current_privilege_label() -> &'static str {
    "unsupported-platform"
}

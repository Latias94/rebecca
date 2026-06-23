use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use rebecca_core::config::default_app_paths;
use rebecca_core::executor::execute_cleanup_plan;
use rebecca_core::history::HistoryStore;
use rebecca_core::planner::build_cleanup_plan;
use rebecca_core::{DeleteMode, PlanRequest, Platform};

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
    match cli.command.unwrap_or(Command::Scan { json: false }) {
        Command::Scan { json } => scan(json),
        Command::Clean {
            dry_run,
            json,
            yes,
            categories,
            rules,
            allow_moderate,
            allow_risky,
        } => clean(
            dry_run,
            json,
            yes,
            categories,
            rules,
            allow_moderate,
            allow_risky,
        ),
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

fn scan(json: bool) -> Result<()> {
    let rules = rebecca_rules::builtin_rules();

    if json {
        println!("{}", serde_json::to_string_pretty(&rules)?);
        return Ok(());
    }

    println!("Rebecca rules: {}", rules.len());
    for rule in rules {
        println!(
            "- {} [{}/{:?}] {}",
            rule.id, rule.category, rule.safety_level, rule.name
        );
    }

    Ok(())
}

fn clean(
    dry_run: bool,
    json: bool,
    yes: bool,
    categories: Vec<String>,
    rules: Vec<String>,
    allow_moderate: bool,
    allow_risky: bool,
) -> Result<()> {
    let mode = if dry_run {
        DeleteMode::DryRun
    } else {
        DeleteMode::RecycleBin
    };

    let mut request = PlanRequest::for_platform(Platform::Windows, mode);
    request.selected_categories = categories;
    request.selected_rule_ids = rules;
    request.allow_moderate = allow_moderate;
    request.allow_risky = allow_risky;

    let rules = rebecca_rules::builtin_rules();
    let mut plan = build_cleanup_plan(&request, &rules)?;

    if dry_run {
        return print_plan(&plan, json);
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
            return print_plan(&plan, json);
        }

        if !yes && !confirm_cleanup(&plan)? {
            println!("Cleanup cancelled.");
            return Ok(());
        }

        let backend = rebecca_windows::WindowsRecycleBinBackend::new();
        execute_cleanup_plan(&mut plan, &backend)?;

        let paths = default_app_paths()?;
        HistoryStore::new(paths.history_file).append_plan(&plan)?;

        print_plan(&plan, json)
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

fn print_plan(plan: &rebecca_core::plan::CleanupPlan, json: bool) -> Result<()> {
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
    println!("Estimated bytes: {}", plan.summary.estimated_bytes);
    println!("Freed bytes: {}", plan.summary.freed_bytes);
    println!(
        "Pending reclaim bytes: {}",
        plan.summary.pending_reclaim_bytes
    );

    for target in &plan.targets {
        println!(
            "- {:?} {} [{}] {} bytes{}",
            target.status,
            target.rule_id,
            target.path.display(),
            target.estimated_bytes,
            target
                .reason
                .as_ref()
                .map(|reason| format!(" ({reason})"))
                .unwrap_or_default()
        );
    }

    Ok(())
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

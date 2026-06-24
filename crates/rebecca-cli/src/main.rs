use anyhow::Result;
use clap::{Parser, Subcommand};
use std::num::NonZeroUsize;

mod cache;
mod clean;
mod history_view;
mod info;
mod output;
mod scan;

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
        /// Use the rebuildable scan cache for eligible target estimates.
        #[arg(long)]
        scan_cache: bool,
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
        /// Show only the most recent N history entries.
        #[arg(long)]
        limit: Option<NonZeroUsize>,
    },
    /// Inspect or purge Rebecca's own cache directory.
    Cache {
        #[command(subcommand)]
        command: CacheCommand,
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
enum CacheCommand {
    /// Purge Rebecca's rebuildable cache directory.
    Purge {
        /// Preview the purge without deleting anything.
        #[arg(long)]
        dry_run: bool,
        /// Render machine-readable JSON.
        #[arg(long)]
        json: bool,
        /// Delete rebuildable cache entries instead of previewing them.
        #[arg(long)]
        yes: bool,
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
        } => scan::run(json, categories, rules),
        Command::Clean {
            dry_run,
            json,
            yes,
            no_progress,
            scan_cache,
            categories,
            rules,
            allow_moderate,
            allow_risky,
        } => clean::run(clean::CleanOptions {
            dry_run,
            json,
            yes,
            no_progress,
            scan_cache,
            categories,
            rules,
            allow_moderate,
            allow_risky,
        }),
        Command::History { json, limit } => info::print_history(json, limit),
        Command::Cache { command } => match command {
            CacheCommand::Purge { dry_run, json, yes } => {
                cache::purge(cache::CachePurgeOptions { dry_run, json, yes })
            }
        },
        Command::Config { command } => match command {
            ConfigCommand::Paths { json } => info::print_config_paths(json),
        },
        Command::Doctor { command } => match command {
            DoctorCommand::Permissions => info::print_privilege_level(),
            DoctorCommand::Steam => {
                info::print_steam_discovery(&*info::steam_application_discovery())
            }
        },
    }
}

fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();
}

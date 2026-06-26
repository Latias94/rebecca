use anyhow::Result;
use clap::{Parser, Subcommand};
use std::num::NonZeroUsize;
use std::path::PathBuf;

mod apps;
mod cache;
mod cache_view;
mod clean;
mod clean_view;
mod history_view;
mod info;
mod output;
mod purge;
mod purge_view;
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
        /// Exclude a path from cleanup for this run. Can be repeated.
        #[arg(long = "exclude", value_name = "PATH")]
        exclude_paths: Vec<PathBuf>,
        /// Include moderate-risk rules.
        #[arg(long)]
        allow_moderate: bool,
        /// Include risky rules.
        #[arg(long)]
        allow_risky: bool,
    },
    /// Preview or purge project build artifacts such as node_modules and target.
    Purge {
        /// Preview the purge plan without deleting anything.
        #[arg(short = 'n', long)]
        dry_run: bool,
        /// Render machine-readable JSON.
        #[arg(long)]
        json: bool,
        /// Delete project artifacts instead of previewing them.
        #[arg(long)]
        yes: bool,
        /// Disable human progress output while building the purge plan.
        #[arg(long)]
        no_progress: bool,
        /// Use the rebuildable scan cache for eligible target estimates.
        #[arg(long)]
        scan_cache: bool,
        /// List supported project artifact selectors without scanning.
        #[arg(long)]
        list_artifacts: bool,
        /// Directory to scan for project artifacts. Overrides configured purge roots.
        #[arg(long = "root", value_name = "PATH")]
        roots: Vec<PathBuf>,
        /// Maximum directory depth to scan below each root. Defaults to config or 6.
        #[arg(long, value_name = "N")]
        max_depth: Option<usize>,
        /// Skip artifact directories modified more recently than N days. Defaults to config or 7; use 0 to include recent artifacts.
        #[arg(long, value_name = "DAYS")]
        min_age_days: Option<u64>,
        /// Include only a project artifact kind. Accepts directory names or rule ids. Can be repeated.
        #[arg(long = "artifact", value_name = "ARTIFACT")]
        artifacts: Vec<String>,
        /// Exclude a path from project artifact purge for this run. Can be repeated.
        #[arg(long = "exclude", value_name = "PATH")]
        exclude_paths: Vec<PathBuf>,
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
    /// Scan or clean leftover app cache data.
    Apps {
        #[command(subcommand)]
        command: AppsCommand,
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
enum AppsCommand {
    /// Preview leftover app cache data discovered from installed applications.
    Scan {
        /// Render machine-readable JSON.
        #[arg(long)]
        json: bool,
        /// Disable human progress output while building the app leftovers plan.
        #[arg(long)]
        no_progress: bool,
        /// Use the rebuildable scan cache for eligible target estimates.
        #[arg(long)]
        scan_cache: bool,
        /// Exclude a path from app leftovers cleanup for this run. Can be repeated.
        #[arg(long = "exclude", value_name = "PATH")]
        exclude_paths: Vec<PathBuf>,
    },
    /// Preview or move leftover app cache data to the Recycle Bin.
    Clean {
        /// Preview the app leftovers plan without deleting anything.
        #[arg(short = 'n', long)]
        dry_run: bool,
        /// Render machine-readable JSON.
        #[arg(long)]
        json: bool,
        /// Delete leftover app cache data instead of previewing it.
        #[arg(long)]
        yes: bool,
        /// Disable human progress output while building the app leftovers plan.
        #[arg(long)]
        no_progress: bool,
        /// Use the rebuildable scan cache for eligible target estimates.
        #[arg(long)]
        scan_cache: bool,
        /// Exclude a path from app leftovers cleanup for this run. Can be repeated.
        #[arg(long = "exclude", value_name = "PATH")]
        exclude_paths: Vec<PathBuf>,
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
            exclude_paths,
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
            exclude_paths,
            allow_moderate,
            allow_risky,
        }),
        Command::Purge {
            dry_run,
            json,
            yes,
            no_progress,
            scan_cache,
            list_artifacts,
            roots,
            max_depth,
            min_age_days,
            artifacts,
            exclude_paths,
        } => purge::run(purge::PurgeOptions {
            dry_run,
            json,
            yes,
            no_progress,
            scan_cache,
            list_artifacts,
            roots,
            max_depth,
            min_age_days,
            artifacts,
            exclude_paths,
        }),
        Command::History { json, limit } => info::print_history(json, limit),
        Command::Cache { command } => match command {
            CacheCommand::Purge { dry_run, json, yes } => {
                cache::purge(cache::CachePurgeOptions { dry_run, json, yes })
            }
        },
        Command::Apps { command } => match command {
            AppsCommand::Scan {
                json,
                no_progress,
                scan_cache,
                exclude_paths,
            } => apps::scan(apps::AppsScanOptions {
                json,
                no_progress,
                scan_cache,
                exclude_paths,
            }),
            AppsCommand::Clean {
                dry_run,
                json,
                yes,
                no_progress,
                scan_cache,
                exclude_paths,
            } => apps::clean(apps::AppsCleanOptions {
                dry_run,
                json,
                yes,
                no_progress,
                scan_cache,
                exclude_paths,
            }),
        },
        Command::Config { command } => match command {
            ConfigCommand::Paths { json } => info::print_config_paths(json),
        },
        Command::Doctor { command } => match command {
            DoctorCommand::Permissions => info::print_privilege_level(),
        },
    }
}

fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();
}

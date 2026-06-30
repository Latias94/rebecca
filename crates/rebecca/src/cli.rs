use std::num::NonZeroUsize;
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OutputMode {
    Human,
    Json,
    Ndjson,
}

impl OutputMode {
    pub(crate) fn is_human(self) -> bool {
        matches!(self, Self::Human)
    }

    pub(crate) fn is_ndjson(self) -> bool {
        matches!(self, Self::Ndjson)
    }
}

impl std::fmt::Display for OutputMode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let label = match self {
            Self::Human => "human",
            Self::Json => "json",
            Self::Ndjson => "ndjson",
        };
        formatter.write_str(label)
    }
}

#[derive(Debug, Parser)]
#[command(
    name = "rebecca",
    version,
    about = "Windows-first cleanup CLI",
    subcommand_required = true,
    arg_required_else_help = true
)]
pub struct Cli {
    /// Select human text, JSON envelope, or NDJSON event output.
    #[arg(
        long,
        value_enum,
        default_value_t = OutputMode::Human,
        global = true
    )]
    pub format: OutputMode,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Show the built-in cleanup rules that would be considered.
    Scan(ScanArgs),
    /// Build or execute a cleanup plan.
    Clean(CleanArgs),
    /// Preview or purge project build artifacts such as node_modules and target.
    Purge(PurgeArgs),
    /// Show cleanup history.
    History(HistoryArgs),
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
    /// Generate shell completion scripts from the live parser.
    Completion(CompletionArgs),
}

#[derive(Debug, Args)]
pub struct ScanArgs {
    /// Include a category. Can be repeated.
    #[arg(long = "category")]
    pub categories: Vec<String>,
    /// Include a specific rule id. Can be repeated.
    #[arg(long = "rule")]
    pub rules: Vec<String>,
}

#[derive(Debug, Args)]
pub struct CleanupSelectionArgs {
    /// Include a category. Can be repeated.
    #[arg(long = "category")]
    pub categories: Vec<String>,
    /// Include a specific rule id. Can be repeated.
    #[arg(long = "rule")]
    pub rules: Vec<String>,
}

#[derive(Debug, Args)]
pub struct CleanupExecutionArgs {
    /// Disable human progress output while building the cleanup plan.
    #[arg(long)]
    pub no_progress: bool,
    /// Use the rebuildable scan cache for eligible target estimates.
    #[arg(long)]
    pub scan_cache: bool,
    /// Exclude a path from cleanup for this run. Can be repeated.
    #[arg(long = "exclude", value_name = "PATH")]
    pub exclude_paths: Vec<PathBuf>,
}

#[derive(Debug, Args)]
pub struct RiskArgs {
    /// Include moderate-risk rules.
    #[arg(long)]
    pub allow_moderate: bool,
    /// Include risky rules.
    #[arg(long)]
    pub allow_risky: bool,
}

#[derive(Debug, Args)]
pub struct CleanArgs {
    /// Preview the cleanup plan without deleting anything. This is the default unless --yes is set.
    #[arg(short = 'n', long)]
    pub dry_run: bool,
    /// Move allowed targets to the Recycle Bin instead of previewing.
    #[arg(long)]
    pub yes: bool,
    #[command(flatten)]
    pub selection: CleanupSelectionArgs,
    #[command(flatten)]
    pub execution: CleanupExecutionArgs,
    #[command(flatten)]
    pub risk: RiskArgs,
}

#[derive(Debug, Args)]
#[command(args_conflicts_with_subcommands = true)]
pub struct PurgeArgs {
    #[command(subcommand)]
    pub command: Option<PurgeCommand>,
    /// Preview the purge plan without deleting anything.
    #[arg(short = 'n', long)]
    pub dry_run: bool,
    /// Delete project artifacts instead of previewing them.
    #[arg(long)]
    pub yes: bool,
    /// Disable human progress output while building the purge plan.
    #[arg(long)]
    pub no_progress: bool,
    /// Use the rebuildable scan cache for eligible target estimates.
    #[arg(long)]
    pub scan_cache: bool,
    /// List supported project artifact selectors without scanning.
    #[arg(long)]
    pub list_artifacts: bool,
    /// Directory to scan for project artifacts. Overrides configured purge roots.
    #[arg(long = "root", value_name = "PATH")]
    pub roots: Vec<PathBuf>,
    /// Maximum directory depth to scan below each root. Defaults to config or 6.
    #[arg(long, value_name = "N")]
    pub max_depth: Option<usize>,
    /// Skip artifact directories modified more recently than N days. Defaults to config or 7; use 0 to include recent artifacts.
    #[arg(long, value_name = "DAYS")]
    pub min_age_days: Option<u64>,
    /// Include only a project artifact kind. Accepts directory names or rule ids. Can be repeated.
    #[arg(long = "artifact", value_name = "ARTIFACT")]
    pub artifacts: Vec<String>,
    /// Exclude a path from project artifact purge for this run. Can be repeated.
    #[arg(long = "exclude", value_name = "PATH")]
    pub exclude_paths: Vec<PathBuf>,
}

#[derive(Debug, Subcommand)]
pub enum PurgeCommand {
    /// Inspect rebuildable project artifact space without cleanup prompts or history writes.
    Inspect(PurgeInspectArgs),
}

#[derive(Debug, Args)]
pub struct PurgeInspectArgs {
    /// Disable human progress output while building the insight report.
    #[arg(long)]
    pub no_progress: bool,
    /// Use the rebuildable scan cache for eligible target estimates.
    #[arg(long)]
    pub scan_cache: bool,
    /// Directory to scan for project artifacts. Overrides configured purge roots.
    #[arg(long = "root", value_name = "PATH")]
    pub roots: Vec<PathBuf>,
    /// Maximum directory depth to scan below each root. Defaults to config or 6.
    #[arg(long, value_name = "N")]
    pub max_depth: Option<usize>,
    /// Skip artifact directories modified more recently than N days. Defaults to config or 7; use 0 to include recent artifacts.
    #[arg(long, value_name = "DAYS")]
    pub min_age_days: Option<u64>,
    /// Include only a project artifact kind. Accepts directory names or rule ids. Can be repeated.
    #[arg(long = "artifact", value_name = "ARTIFACT")]
    pub artifacts: Vec<String>,
    /// Exclude a path from project artifact insight for this run. Can be repeated.
    #[arg(long = "exclude", value_name = "PATH")]
    pub exclude_paths: Vec<PathBuf>,
}

#[derive(Debug, Args)]
pub struct HistoryArgs {
    /// Show only the most recent N history entries.
    #[arg(long)]
    pub limit: Option<NonZeroUsize>,
}

#[derive(Debug, Subcommand)]
pub enum CacheCommand {
    /// Purge Rebecca's rebuildable cache directory.
    Purge {
        /// Preview the purge without deleting anything.
        #[arg(long)]
        dry_run: bool,
        /// Delete rebuildable cache entries instead of previewing them.
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum AppsCommand {
    /// Preview leftover app cache data discovered from installed applications.
    Scan {
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
pub enum ConfigCommand {
    /// Print config, state, cache, and history paths.
    Paths,
}

#[derive(Debug, Subcommand)]
pub enum DoctorCommand {
    /// Print the current Windows privilege level when available.
    Permissions,
}

#[derive(Debug, Args)]
pub struct CompletionArgs {
    /// Shell to generate completion for. Defaults to the current shell or bash.
    #[arg(value_enum)]
    pub shell: Option<clap_complete::Shell>,
}

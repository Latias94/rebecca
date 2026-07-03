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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub enum ProgressDetail {
    #[default]
    Target,
    File,
}

impl ProgressDetail {
    pub(crate) fn includes_file_events(self) -> bool {
        matches!(self, Self::File)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub enum ScanBackendArg {
    #[default]
    PortableRecursive,
    WindowsNative,
    WindowsNtfsMftExperimental,
}

impl From<ScanBackendArg> for rebecca::core::scan::ScanBackendKind {
    fn from(value: ScanBackendArg) -> Self {
        match value {
            ScanBackendArg::PortableRecursive => Self::PortableRecursive,
            ScanBackendArg::WindowsNative => Self::WindowsNative,
            ScanBackendArg::WindowsNtfsMftExperimental => Self::WindowsNtfsMftExperimental,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum DiskMapGroupKindArg {
    Extension,
    Depth,
    Age,
}

impl From<DiskMapGroupKindArg> for rebecca::core::disk_map::DiskMapGroupKind {
    fn from(value: DiskMapGroupKindArg) -> Self {
        match value {
            DiskMapGroupKindArg::Extension => Self::Extension,
            DiskMapGroupKindArg::Depth => Self::Depth,
            DiskMapGroupKindArg::Age => Self::Age,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub enum DiskMapSortArg {
    #[default]
    Logical,
    Allocated,
    Files,
    Unique,
}

impl From<DiskMapSortArg> for rebecca::core::disk_map::DiskMapSortField {
    fn from(value: DiskMapSortArg) -> Self {
        match value {
            DiskMapSortArg::Logical => Self::Logical,
            DiskMapSortArg::Allocated => Self::Allocated,
            DiskMapSortArg::Files => Self::Files,
            DiskMapSortArg::Unique => Self::Unique,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum InspectMapTableFormatArg {
    Csv,
    Tsv,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum InspectMapTableRowKindArg {
    Total,
    Root,
    Entry,
    Group,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum CatalogKindArg {
    CleanupRule,
    ProjectArtifact,
    Warning,
    SafetyCategory,
    ActionKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Subcommand)]
pub enum CatalogCommand {
    /// Validate the built-in rule and safety catalogs.
    Validate,
}

impl From<CatalogKindArg> for rebecca::core::catalog::CatalogItemKind {
    fn from(kind: CatalogKindArg) -> Self {
        match kind {
            CatalogKindArg::CleanupRule => Self::CleanupRule,
            CatalogKindArg::ProjectArtifact => Self::ProjectArtifact,
            CatalogKindArg::Warning => Self::Warning,
            CatalogKindArg::SafetyCategory => Self::SafetyCategory,
            CatalogKindArg::ActionKind => Self::ActionKind,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum SafetyLevelArg {
    Safe,
    Moderate,
    Risky,
    Dangerous,
}

impl From<SafetyLevelArg> for rebecca::core::SafetyLevel {
    fn from(level: SafetyLevelArg) -> Self {
        match level {
            SafetyLevelArg::Safe => Self::Safe,
            SafetyLevelArg::Moderate => Self::Moderate,
            SafetyLevelArg::Risky => Self::Risky,
            SafetyLevelArg::Dangerous => Self::Dangerous,
        }
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
    /// List or validate cleanup rules, project artifacts, warnings, and safety catalog entries.
    Catalog(CatalogArgs),
    /// Show the built-in cleanup rules that would be considered.
    Scan(ScanArgs),
    /// Build or execute a cleanup plan.
    Clean(CleanArgs),
    /// Run read-only cleanup intelligence inspections.
    Inspect {
        #[command(subcommand)]
        command: InspectCommand,
    },
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

#[derive(Debug, Subcommand)]
pub enum InspectCommand {
    /// Inspect top-level disk usage below one or more roots.
    Space(InspectSpaceArgs),
    /// Inspect ranked disk usage below one or more roots.
    Map(InspectMapArgs),
    /// Inspect rebuildable project artifact space.
    Artifacts(InspectArtifactsArgs),
    /// Report duplicate, large, empty-file, and empty-directory cleanup opportunities.
    Lint(InspectLintArgs),
}

#[derive(Debug, Args)]
pub struct InspectSpaceArgs {
    /// Disable human progress output while building the insight report.
    #[arg(long)]
    pub no_progress: bool,
    /// Use the rebuildable scan cache for eligible entry estimates.
    #[arg(long)]
    pub scan_cache: bool,
    /// Select the scan backend used for inspect space estimates.
    #[arg(long = "scan-backend", value_enum, default_value_t = ScanBackendArg::PortableRecursive)]
    pub scan_backend: ScanBackendArg,
    /// Directory to inspect. Can be repeated. Defaults to the current directory.
    #[arg(long = "root", value_name = "PATH")]
    pub roots: Vec<PathBuf>,
    /// Maximum number of largest entries to include.
    #[arg(long = "top", value_name = "N", default_value_t = 10)]
    pub top_limit: usize,
    /// Maximum number of raw diagnostics to include. Use 0 for summary only.
    #[arg(long = "diagnostic-limit", value_name = "N", default_value_t = rebecca::core::inspect::DEFAULT_SPACE_INSIGHT_DIAGNOSTIC_LIMIT)]
    pub diagnostic_limit: usize,
}

#[derive(Debug, Args)]
pub struct InspectMapArgs {
    /// Select the scan backend used for disk-map inventory.
    #[arg(long = "scan-backend", value_enum, default_value_t = ScanBackendArg::PortableRecursive)]
    pub scan_backend: ScanBackendArg,
    /// Directory or file to inspect. Can be repeated. Defaults to the current directory.
    #[arg(long = "root", value_name = "PATH")]
    pub roots: Vec<PathBuf>,
    /// Maximum number of largest entries to include. Use 0 for totals only.
    #[arg(long = "top", value_name = "N", default_value_t = rebecca::core::disk_map::DEFAULT_DISK_MAP_TOP_LIMIT)]
    pub top_limit: usize,
    /// Sort top entries by logical bytes, allocated bytes, file count, or unique logical bytes.
    #[arg(long = "sort", value_enum, default_value_t = DiskMapSortArg::Logical)]
    pub sort: DiskMapSortArg,
    /// Add a file grouping section. Can be repeated: extension, depth, age.
    #[arg(long = "group-by", value_enum)]
    pub group_kinds: Vec<DiskMapGroupKindArg>,
    /// Maximum number of groups to include across all requested group kinds.
    #[arg(long = "group-limit", value_name = "N", default_value_t = rebecca::core::disk_map::DEFAULT_DISK_MAP_GROUP_LIMIT)]
    pub group_limit: usize,
    /// Sort groups by logical bytes, allocated bytes, file count, or unique logical bytes.
    #[arg(long = "group-sort", value_enum, default_value_t = DiskMapSortArg::Logical)]
    pub group_sort: DiskMapSortArg,
    /// Export the flat map table as CSV or TSV. Cannot be combined with --format json/ndjson.
    #[arg(long = "table", value_enum, value_name = "FORMAT")]
    pub table_format: Option<InspectMapTableFormatArg>,
    /// Limit table output to selected row kinds. Can be repeated: total, root, entry, group.
    #[arg(long = "table-row", value_enum, value_name = "KIND")]
    pub table_row_kinds: Vec<InspectMapTableRowKindArg>,
    /// Maximum number of raw diagnostics to include. Use 0 for summary only.
    #[arg(long = "diagnostic-limit", value_name = "N", default_value_t = rebecca::core::disk_map::DEFAULT_DISK_MAP_DIAGNOSTIC_LIMIT)]
    pub diagnostic_limit: usize,
    /// Maximum rendered depth below each root. Direct children are depth 1.
    #[arg(long = "max-depth", value_name = "N")]
    pub max_depth: Option<usize>,
}

#[derive(Debug, Args)]
pub struct InspectArtifactsArgs {
    /// Disable human progress output while building the insight report.
    #[arg(long)]
    pub no_progress: bool,
    /// Select progress detail for supported NDJSON and human plan-building updates.
    #[arg(long, value_enum, default_value_t = ProgressDetail::Target)]
    pub progress_detail: ProgressDetail,
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
    #[arg(long, alias = "older-than-days", value_name = "DAYS")]
    pub min_age_days: Option<u64>,
    /// Measure ranked eligible artifacts until at least this many bytes would be reclaimed.
    #[arg(long, value_name = "BYTES")]
    pub reclaim_limit_bytes: Option<u64>,
    /// Include only a project artifact kind. Accepts directory names or rule ids. Can be repeated.
    #[arg(long = "artifact", value_name = "ARTIFACT")]
    pub artifacts: Vec<String>,
    /// Exclude a path from project artifact insight for this run. Can be repeated.
    #[arg(long = "exclude", value_name = "PATH")]
    pub exclude_paths: Vec<PathBuf>,
}

#[derive(Debug, Args)]
pub struct InspectLintArgs {
    /// Directory to inspect. Can be repeated. Defaults to the current directory.
    #[arg(long = "root", value_name = "PATH")]
    pub roots: Vec<PathBuf>,
    /// Directory whose files should be treated as keep candidates in duplicate groups.
    #[arg(long = "reference", value_name = "PATH")]
    pub reference_roots: Vec<PathBuf>,
    /// Exclude a path from lint inventory for this run. Can be repeated.
    #[arg(long = "exclude", value_name = "PATH")]
    pub exclude_paths: Vec<PathBuf>,
    /// Include files at or above this size in the large-file report.
    #[arg(long, value_name = "BYTES", default_value_t = rebecca::core::lint::DEFAULT_LARGE_FILE_THRESHOLD_BYTES)]
    pub large_file_threshold_bytes: u64,
    /// Maximum number of groups or entries to include per lint report section.
    #[arg(long = "top", value_name = "N", default_value_t = rebecca::core::lint::DEFAULT_LINT_TOP_LIMIT)]
    pub top_limit: usize,
}

#[derive(Debug, Args)]
pub struct CatalogArgs {
    #[command(subcommand)]
    pub command: Option<CatalogCommand>,
    /// Include only one catalog item kind.
    #[arg(long, value_enum)]
    pub kind: Option<CatalogKindArg>,
    /// Include a cleanup or safety category. Can be repeated.
    #[arg(long = "category")]
    pub categories: Vec<String>,
    /// Include a cleanup rule, artifact rule, warning id, safety category, or action id. Can be repeated.
    #[arg(long = "rule")]
    pub rules: Vec<String>,
    /// Include a project artifact selector. Can be repeated.
    #[arg(long = "artifact")]
    pub artifacts: Vec<String>,
    /// Include a warning kind. Can be repeated.
    #[arg(long = "warning")]
    pub warnings: Vec<String>,
    /// Include cleanup rules at a safety level.
    #[arg(long = "safety-level", value_enum)]
    pub safety_level: Option<SafetyLevelArg>,
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
    /// Select progress detail for supported NDJSON and human plan-building updates.
    #[arg(long, value_enum, default_value_t = ProgressDetail::Target)]
    pub progress_detail: ProgressDetail,
    /// Use the rebuildable scan cache for eligible target estimates.
    #[arg(long)]
    pub scan_cache: bool,
    /// Disable the rebuildable scan cache for preview estimates.
    #[arg(long, conflicts_with = "scan_cache")]
    pub no_scan_cache: bool,
    /// Select the scan backend used for cleanup plan estimates.
    #[arg(long = "scan-backend", value_enum, default_value_t = ScanBackendArg::PortableRecursive)]
    pub scan_backend: ScanBackendArg,
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
    /// Include targets that carry a named warning gate. Can be repeated.
    #[arg(long = "allow-warning", value_name = "WARNING")]
    pub allow_warnings: Vec<String>,
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
    /// Select progress detail for supported NDJSON and human plan-building updates.
    #[arg(long, value_enum, default_value_t = ProgressDetail::Target)]
    pub progress_detail: ProgressDetail,
    /// Use the rebuildable scan cache for eligible target estimates.
    #[arg(long)]
    pub scan_cache: bool,
    /// Disable the rebuildable scan cache for preview estimates.
    #[arg(long, conflicts_with = "scan_cache")]
    pub no_scan_cache: bool,
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
    #[arg(long, alias = "older-than-days", value_name = "DAYS")]
    pub min_age_days: Option<u64>,
    /// Measure ranked eligible artifacts until at least this many bytes would be reclaimed.
    #[arg(long, value_name = "BYTES")]
    pub reclaim_limit_bytes: Option<u64>,
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
    /// Select progress detail for supported NDJSON and human plan-building updates.
    #[arg(long, value_enum, default_value_t = ProgressDetail::Target)]
    pub progress_detail: ProgressDetail,
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
    #[arg(long, alias = "older-than-days", value_name = "DAYS")]
    pub min_age_days: Option<u64>,
    /// Measure ranked eligible artifacts until at least this many bytes would be reclaimed.
    #[arg(long, value_name = "BYTES")]
    pub reclaim_limit_bytes: Option<u64>,
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
        /// Move rebuildable cache entries to the Recycle Bin instead of previewing them.
        #[arg(long)]
        yes: bool,
        /// Permanently delete rebuildable cache entries. Requires --yes and conflicts with --dry-run.
        #[arg(long, requires = "yes", conflicts_with = "dry_run")]
        permanent: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum AppsCommand {
    /// Preview leftover app cache data discovered from installed applications.
    Scan {
        /// Disable human progress output while building the app leftovers plan.
        #[arg(long)]
        no_progress: bool,
        /// Select progress detail for supported NDJSON and human plan-building updates.
        #[arg(long, value_enum, default_value_t = ProgressDetail::Target)]
        progress_detail: ProgressDetail,
        /// Use the rebuildable scan cache for eligible target estimates.
        #[arg(long)]
        scan_cache: bool,
        /// Disable the rebuildable scan cache for preview estimates.
        #[arg(long, conflicts_with = "scan_cache")]
        no_scan_cache: bool,
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
        /// Select progress detail for supported NDJSON and human plan-building updates.
        #[arg(long, value_enum, default_value_t = ProgressDetail::Target)]
        progress_detail: ProgressDetail,
        /// Use the rebuildable scan cache for eligible target estimates.
        #[arg(long)]
        scan_cache: bool,
        /// Disable the rebuildable scan cache for preview estimates.
        #[arg(long, conflicts_with = "scan_cache")]
        no_scan_cache: bool,
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
    /// Report warning-bearing cleanup rules whose applications appear to be running.
    ActiveProcesses,
}

#[derive(Debug, Args)]
pub struct CompletionArgs {
    /// Shell to generate completion for. Defaults to the current shell or bash.
    #[arg(value_enum)]
    pub shell: Option<clap_complete::Shell>,
}

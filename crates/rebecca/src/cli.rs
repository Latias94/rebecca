use std::num::NonZeroUsize;
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum, ValueHint};

pub const DEFAULT_RULE_VALIDATE_MAX_DEPTH: usize = 8;
pub const DEFAULT_RULE_VALIDATE_MAX_FILES: usize = 512;

const ROOT_AFTER_LONG_HELP: &str = "\
Common tasks:
  rebecca inspect map --root . --top 20 --cleanup-advice
  rebecca clean --dry-run --category browser
  rebecca clean --dry-run --category browser --save-plan cleanup-plan.json
  rebecca plan run cleanup-plan.json --yes
  rebecca clean --yes --category browser --receipt cleanup-receipt.json
  rebecca clean --yes --category browser
  rebecca purge --dry-run --root . --artifact target
  rebecca trash empty
  rebecca tui --root .

Normal cleanup moves allowed targets to the system Trash or Windows Recycle Bin.
Use --permanent only when you want irreversible deletion.";

const CLEAN_AFTER_LONG_HELP: &str = "\
Examples:
  rebecca clean --dry-run
  rebecca clean --dry-run --category browser
  rebecca clean --dry-run --category browser --save-plan cleanup-plan.json
  rebecca clean --dry-run --rule windows.user-temp
  rebecca clean --yes --category browser
  rebecca clean --yes --category browser --receipt cleanup-receipt.json
  rebecca clean --yes --permanent --category browser
  rebecca plan inspect cleanup-plan.json
  rebecca plan run cleanup-plan.json --yes

Without --yes, clean only previews. With --yes, allowed targets move to the
system Trash or Windows Recycle Bin. Add --permanent only when you want to
bypass trash.";

const INSPECT_MAP_AFTER_LONG_HELP: &str = "\
Examples:
  rebecca inspect map --root . --top 20
  rebecca inspect map --root . --top 20 --cleanup-advice
  rebecca inspect map --root . --group-by extension
  rebecca inspect map --root . --table csv --table-row entry

Use inspect map when you want to understand where space went. Use clean or
purge for deletion.";

const INSPECT_DRIVE_AFTER_LONG_HELP: &str = "\
Examples:
  rebecca inspect drive E:
  rebecca inspect drive / --scan-backend portable-recursive
  rebecca inspect drive . --top 40 --no-cleanup-advice
  rebecca inspect drive C:\\ --scan-backend windows-ntfs-mft-experimental

Use inspect drive when you want the safest first answer to \"what is filling this
disk?\" It is read-only, enables cleanup advice by default, separates Rebecca
commands from manual-review findings, and keeps deletion in clean or purge.";

const TUI_AFTER_LONG_HELP: &str = "\
Examples:
  rebecca tui
  rebecca tui --root .
  rebecca i --root C:\\

The TUI is an interactive workbench for browsing disk usage, staging cleanup
rules, previewing targets, and executing only after typed confirmation.";

const PURGE_AFTER_LONG_HELP: &str = "\
Examples:
  rebecca purge --dry-run --root .
  rebecca purge --dry-run --root . --artifact target
  rebecca purge --dry-run --root . --artifact target --save-plan purge-plan.json
  rebecca purge --dry-run --root . --min-age-days 0
  rebecca purge --yes --root . --artifact target
  rebecca purge --yes --permanent --root . --artifact target
  rebecca plan run purge-plan.json --yes

Purge is for rebuildable project output such as target, node_modules, build,
dist, and similar artifact directories.";

const PLAN_AFTER_LONG_HELP: &str = "\
Examples:
  rebecca clean --dry-run --category browser --save-plan cleanup-plan.json
  rebecca purge --dry-run --root . --artifact target --save-plan purge-plan.json
  rebecca plan inspect cleanup-plan.json
  rebecca plan run cleanup-plan.json
  rebecca plan run cleanup-plan.json --yes
  rebecca plan run cleanup-plan.json --yes --receipt cleanup-receipt.json
  rebecca plan run cleanup-plan.json --yes --permanent

Saved plans are review artifacts, not blind delete scripts. Rebecca validates
the current platform and target metadata before execution. Without --yes, plan
run only reports what is still executable.";

const TRASH_EMPTY_AFTER_LONG_HELP: &str = "\
Examples:
  rebecca trash empty
  rebecca trash empty --yes
  rebecca trash empty --drive E --yes

Normal cleanup moves data to trash first. Run this command without --yes to
preview the pending space, then add --yes when you are ready to empty it.";

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
    /// Target-level progress events. This is the default for compact terminal output.
    #[default]
    Target,
    /// Include throttled file-level scan progress for long-running scans.
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

impl From<ScanBackendArg> for rebecca_core::scan::ScanBackendKind {
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
    Type,
    Extension,
    Depth,
    Age,
}

impl From<DiskMapGroupKindArg> for rebecca_core::disk_map::DiskMapGroupKind {
    fn from(value: DiskMapGroupKindArg) -> Self {
        match value {
            DiskMapGroupKindArg::Type => Self::Type,
            DiskMapGroupKindArg::Extension => Self::Extension,
            DiskMapGroupKindArg::Depth => Self::Depth,
            DiskMapGroupKindArg::Age => Self::Age,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum DiskMapEntryKindArg {
    File,
    Directory,
    Other,
}

impl From<DiskMapEntryKindArg> for rebecca_core::disk_map::DiskMapEntryKind {
    fn from(value: DiskMapEntryKindArg) -> Self {
        match value {
            DiskMapEntryKindArg::File => Self::File,
            DiskMapEntryKindArg::Directory => Self::Directory,
            DiskMapEntryKindArg::Other => Self::Other,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub enum DiskMapMetadataProfileArg {
    /// Fastest disk map: logical bytes only, without allocated or unique-file evidence.
    LogicalOnly,
    /// Include allocated bytes, but skip unique-file and modified-time evidence.
    Allocated,
    /// Include allocated bytes and unique-file identity, but skip modified-time evidence.
    Unique,
    /// Include allocated bytes, unique-file identity, and modified time for grouping.
    AgeAndGrouping,
    /// Include all available disk-map evidence. This is the default.
    #[default]
    FullEvidence,
}

impl From<DiskMapMetadataProfileArg> for rebecca_core::disk_map::DiskMapMetadataProfile {
    fn from(value: DiskMapMetadataProfileArg) -> Self {
        match value {
            DiskMapMetadataProfileArg::LogicalOnly => Self::LogicalOnly,
            DiskMapMetadataProfileArg::Allocated => Self::Allocated,
            DiskMapMetadataProfileArg::Unique => Self::Unique,
            DiskMapMetadataProfileArg::AgeAndGrouping => Self::AgeAndGrouping,
            DiskMapMetadataProfileArg::FullEvidence => Self::FullEvidence,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum CleanupAdviceStatusArg {
    Cleanable,
    MaybeCleanable,
    ReviewOnly,
    ContainsCleanable,
    Protected,
    Unknown,
}

impl From<CleanupAdviceStatusArg> for rebecca_core::cleanup_advice::CleanupAdviceStatus {
    fn from(value: CleanupAdviceStatusArg) -> Self {
        match value {
            CleanupAdviceStatusArg::Cleanable => Self::Cleanable,
            CleanupAdviceStatusArg::MaybeCleanable => Self::MaybeCleanable,
            CleanupAdviceStatusArg::ReviewOnly => Self::ReviewOnly,
            CleanupAdviceStatusArg::ContainsCleanable => Self::ContainsCleanable,
            CleanupAdviceStatusArg::Protected => Self::Protected,
            CleanupAdviceStatusArg::Unknown => Self::Unknown,
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

impl From<DiskMapSortArg> for rebecca_core::disk_map::DiskMapSortField {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum PlatformArg {
    Windows,
    Linux,
    Macos,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub enum SkillAgentArg {
    #[default]
    Agents,
    Codex,
}

impl SkillAgentArg {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Agents => "agents",
            Self::Codex => "codex",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Subcommand)]
pub enum CatalogCommand {
    /// Validate the built-in rule and safety catalogs.
    Validate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum SchemaDocumentArg {
    Envelope,
    Event,
    Error,
    Payloads,
    Config,
    CleanerManifestV1,
}

impl From<CatalogKindArg> for rebecca_core::catalog::CatalogItemKind {
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

impl From<PlatformArg> for rebecca_core::Platform {
    fn from(platform: PlatformArg) -> Self {
        match platform {
            PlatformArg::Windows => Self::Windows,
            PlatformArg::Linux => Self::Linux,
            PlatformArg::Macos => Self::Macos,
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

impl From<SafetyLevelArg> for rebecca_core::SafetyLevel {
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
    about = "Cross-platform cleanup CLI",
    after_long_help = ROOT_AFTER_LONG_HELP,
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
    /// Report machine-readable CLI capabilities for GUI wrappers.
    Capabilities,
    /// List or validate cleanup rules, project artifacts, warnings, and safety catalog entries.
    Catalog(CatalogArgs),
    /// Validate external cleanup rule manifests before import.
    Rules {
        #[command(subcommand)]
        command: RulesCommand,
    },
    /// Show the built-in cleanup rules that would be considered.
    Scan(ScanArgs),
    /// Build or execute a cleanup plan.
    Clean(CleanArgs),
    /// Inspect or execute a saved dry-run cleanup plan.
    Plan {
        #[command(subcommand)]
        command: PlanCommand,
    },
    /// Open an interactive terminal workbench for disk usage and safe cleanup.
    #[command(visible_alias = "i")]
    Tui(TuiArgs),
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
    /// Export Rebecca CLI API schemas.
    Schema {
        #[command(subcommand)]
        command: SchemaCommand,
    },
    /// Install, locate, or remove Rebecca agent skills.
    Skills {
        #[command(subcommand)]
        command: SkillsCommand,
    },
    /// Inspect or empty the system trash or Windows Recycle Bin.
    Trash {
        #[command(subcommand)]
        command: TrashCommand,
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
    /// Explain what is filling a drive or large root with safe follow-up commands.
    Drive(InspectDriveArgs),
    /// Inspect rebuildable project artifact space.
    Artifacts(InspectArtifactsArgs),
    /// Report duplicate, large, empty-file, and empty-directory cleanup opportunities.
    Lint(InspectLintArgs),
}

#[derive(Debug, Args)]
pub struct InspectSpaceArgs {
    /// Disable the stderr progress spinner; useful for scripts and captured logs.
    #[arg(long)]
    pub no_progress: bool,
    /// Select target-level or throttled file-level progress detail.
    #[arg(long, value_enum, default_value_t = ProgressDetail::Target)]
    pub progress_detail: ProgressDetail,
    /// Use the rebuildable scan cache for eligible entry estimates.
    #[arg(long)]
    pub scan_cache: bool,
    /// Select the scan backend used for inspect space estimates.
    #[arg(long = "scan-backend", value_enum, default_value_t = ScanBackendArg::PortableRecursive)]
    pub scan_backend: ScanBackendArg,
    /// Directory to inspect. Can be repeated. Defaults to the current directory.
    #[arg(long = "root", value_name = "PATH", value_hint = ValueHint::DirPath)]
    pub roots: Vec<PathBuf>,
    /// Maximum number of largest entries to include.
    #[arg(long = "top", value_name = "N", default_value_t = 10)]
    pub top_limit: usize,
    /// Maximum number of raw diagnostics to include. Use 0 for summary only.
    #[arg(long = "diagnostic-limit", value_name = "N", default_value_t = rebecca_core::inspect::DEFAULT_SPACE_INSIGHT_DIAGNOSTIC_LIMIT)]
    pub diagnostic_limit: usize,
}

#[derive(Debug, Args)]
#[command(after_long_help = INSPECT_MAP_AFTER_LONG_HELP)]
pub struct InspectMapArgs {
    /// Disable the stderr progress spinner; useful for scripts and captured logs.
    #[arg(long)]
    pub no_progress: bool,
    /// Select target-level or throttled file-level progress detail.
    #[arg(long, value_enum, default_value_t = ProgressDetail::Target)]
    pub progress_detail: ProgressDetail,
    /// Select the scan backend used for disk-map inventory.
    #[arg(long = "scan-backend", value_enum, default_value_t = ScanBackendArg::PortableRecursive)]
    pub scan_backend: ScanBackendArg,
    /// Select how much metadata disk-map inventory collects.
    #[arg(long = "metadata-profile", value_enum, default_value_t = DiskMapMetadataProfileArg::FullEvidence)]
    pub metadata_profile: DiskMapMetadataProfileArg,
    /// Directory or file to inspect. Can be repeated. Defaults to the current directory.
    #[arg(long = "root", value_name = "PATH", value_hint = ValueHint::AnyPath)]
    pub roots: Vec<PathBuf>,
    /// Maximum number of largest entries to include. Use 0 for totals only.
    #[arg(long = "top", value_name = "N", default_value_t = rebecca_core::disk_map::DEFAULT_DISK_MAP_TOP_LIMIT)]
    pub top_limit: usize,
    /// Sort top entries by logical bytes, allocated bytes, file count, or unique logical bytes.
    #[arg(long = "sort", value_enum, default_value_t = DiskMapSortArg::Logical)]
    pub sort: DiskMapSortArg,
    /// Keep only ranked entries with at least this many logical bytes. Totals are unchanged.
    #[arg(long = "min-logical-bytes", value_name = "BYTES")]
    pub min_logical_bytes: Option<u64>,
    /// Keep only ranked entries of this kind: file, directory, or other. Totals are unchanged.
    #[arg(long = "entry-kind", value_enum, value_name = "KIND")]
    pub entry_kind: Option<DiskMapEntryKindArg>,
    /// Keep only ranked entries whose path contains this text, case-insensitively.
    #[arg(long = "path-contains", value_name = "TEXT")]
    pub path_contains: Option<String>,
    /// Add read-only cleanup advice to ranked entries.
    #[arg(long = "cleanup-advice")]
    pub cleanup_advice: bool,
    /// Render ranked entries without visual bars, optimized for screen readers and logs.
    #[arg(long = "screen-reader")]
    pub screen_reader: bool,
    /// Print full paths in human ranked output instead of compacting long paths.
    #[arg(long = "full-path")]
    pub full_path: bool,
    /// Hide visual usage bars in human ranked output.
    #[arg(long = "no-bars")]
    pub no_bars: bool,
    /// Set the visual usage bar width for human ranked output.
    #[arg(long = "bar-width", value_name = "COLUMNS")]
    pub bar_width: Option<usize>,
    /// Keep only ranked entries with this cleanup advice status. Implies --cleanup-advice.
    #[arg(long = "advice-status", value_enum, value_name = "STATUS")]
    pub advice_status: Option<CleanupAdviceStatusArg>,
    /// Add a file grouping section. Can be repeated: type, extension, depth, age.
    #[arg(long = "group-by", value_enum)]
    pub group_kinds: Vec<DiskMapGroupKindArg>,
    /// Maximum number of groups to include across all requested group kinds.
    #[arg(long = "group-limit", value_name = "N", default_value_t = rebecca_core::disk_map::DEFAULT_DISK_MAP_GROUP_LIMIT)]
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
    #[arg(long = "diagnostic-limit", value_name = "N", default_value_t = rebecca_core::disk_map::DEFAULT_DISK_MAP_DIAGNOSTIC_LIMIT)]
    pub diagnostic_limit: usize,
    /// Maximum rendered depth below each root. Direct children are depth 1.
    #[arg(long = "max-depth", value_name = "N")]
    pub max_depth: Option<usize>,
}

#[derive(Debug, Args)]
#[command(after_long_help = INSPECT_DRIVE_AFTER_LONG_HELP)]
pub struct InspectDriveArgs {
    /// Root, mount point, or drive to inspect.
    #[arg(value_name = "ROOT", value_hint = ValueHint::AnyPath)]
    pub root: PathBuf,
    /// Disable the stderr progress spinner; useful for scripts and captured logs.
    #[arg(long)]
    pub no_progress: bool,
    /// Select target-level or throttled file-level progress detail.
    #[arg(long, value_enum, default_value_t = ProgressDetail::Target)]
    pub progress_detail: ProgressDetail,
    /// Select the scan backend used for disk-map inventory. Defaults to NTFS/MFT on Windows and portable elsewhere.
    #[arg(long = "scan-backend", value_enum)]
    pub scan_backend: Option<ScanBackendArg>,
    /// Select how much metadata disk-map inventory collects; logical-only keeps large first-pass scans faster.
    #[arg(long = "metadata-profile", value_enum, default_value_t = DiskMapMetadataProfileArg::LogicalOnly)]
    pub metadata_profile: DiskMapMetadataProfileArg,
    /// Maximum number of largest entries to include.
    #[arg(long = "top", value_name = "N", default_value_t = 40)]
    pub top_limit: usize,
    /// Disable read-only cleanup and manual-review advice.
    #[arg(long = "no-cleanup-advice")]
    pub no_cleanup_advice: bool,
    /// Render ranked entries without visual bars, optimized for screen readers and logs.
    #[arg(long = "screen-reader")]
    pub screen_reader: bool,
    /// Print full paths in human ranked output instead of compacting long paths.
    #[arg(long = "full-path")]
    pub full_path: bool,
    /// Hide visual usage bars in human ranked output.
    #[arg(long = "no-bars")]
    pub no_bars: bool,
    /// Set the visual usage bar width for human ranked output.
    #[arg(long = "bar-width", value_name = "COLUMNS")]
    pub bar_width: Option<usize>,
    /// Maximum number of raw diagnostics to include. Use 0 for summary only.
    #[arg(long = "diagnostic-limit", value_name = "N", default_value_t = rebecca_core::disk_map::DEFAULT_DISK_MAP_DIAGNOSTIC_LIMIT)]
    pub diagnostic_limit: usize,
    /// Maximum rendered depth below the root. Direct children are depth 1.
    #[arg(long = "max-depth", value_name = "N")]
    pub max_depth: Option<usize>,
}

#[derive(Debug, Args)]
pub struct InspectArtifactsArgs {
    /// Disable the stderr progress spinner; useful for scripts and captured logs.
    #[arg(long)]
    pub no_progress: bool,
    /// Select target-level or throttled file-level progress detail.
    #[arg(long, value_enum, default_value_t = ProgressDetail::Target)]
    pub progress_detail: ProgressDetail,
    /// Use the rebuildable scan cache for eligible target estimates.
    #[arg(long)]
    pub scan_cache: bool,
    /// Directory to scan for project artifacts. Overrides configured purge roots.
    #[arg(long = "root", value_name = "PATH", value_hint = ValueHint::DirPath)]
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
    #[arg(long = "exclude", value_name = "PATH", value_hint = ValueHint::AnyPath)]
    pub exclude_paths: Vec<PathBuf>,
}

#[derive(Debug, Args)]
pub struct InspectLintArgs {
    /// Directory to inspect. Can be repeated. Defaults to the current directory.
    #[arg(long = "root", value_name = "PATH", value_hint = ValueHint::DirPath)]
    pub roots: Vec<PathBuf>,
    /// Directory whose files should be treated as keep candidates in duplicate groups.
    #[arg(long = "reference", value_name = "PATH", value_hint = ValueHint::DirPath)]
    pub reference_roots: Vec<PathBuf>,
    /// Exclude a path from lint inventory for this run. Can be repeated.
    #[arg(long = "exclude", value_name = "PATH", value_hint = ValueHint::AnyPath)]
    pub exclude_paths: Vec<PathBuf>,
    /// Include files at or above this size in the large-file report.
    #[arg(long, value_name = "BYTES", default_value_t = rebecca_core::lint::DEFAULT_LARGE_FILE_THRESHOLD_BYTES)]
    pub large_file_threshold_bytes: u64,
    /// Maximum number of groups or entries to include per lint report section.
    #[arg(long = "top", value_name = "N", default_value_t = rebecca_core::lint::DEFAULT_LINT_TOP_LIMIT)]
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
    /// Include cleanup rules for a platform.
    #[arg(long, value_enum)]
    pub platform: Option<PlatformArg>,
}

#[derive(Debug, Subcommand)]
pub enum RulesCommand {
    /// Validate external Cleaner Manifest v1 files or directories without enabling them.
    Validate(RulesValidateArgs),
    /// Import an external Cleaner Manifest v1 file into Rebecca-owned storage, disabled by default.
    Import(RulesImportArgs),
    /// List imported external rule manifests.
    List,
    /// Enable an imported external rule manifest after revalidation.
    Enable(RulesImportIdArgs),
    /// Disable an imported external rule manifest.
    Disable(RulesImportIdArgs),
    /// Remove an imported external rule manifest from Rebecca-owned storage.
    Remove(RulesImportIdArgs),
}

#[derive(Debug, Args)]
pub struct RulesValidateArgs {
    /// External Cleaner Manifest v1 TOML file. Can be repeated.
    #[arg(long = "file", value_name = "PATH", value_hint = ValueHint::FilePath)]
    pub files: Vec<PathBuf>,
    /// Directory containing external Cleaner Manifest v1 TOML files. Can be repeated.
    #[arg(long = "dir", value_name = "PATH", value_hint = ValueHint::DirPath)]
    pub dirs: Vec<PathBuf>,
    /// Maximum directory depth below each --dir to inspect.
    #[arg(long = "max-depth", value_name = "N", default_value_t = DEFAULT_RULE_VALIDATE_MAX_DEPTH)]
    pub max_depth: usize,
    /// Maximum number of manifest files accepted across all inputs.
    #[arg(long = "max-files", value_name = "N", default_value_t = DEFAULT_RULE_VALIDATE_MAX_FILES)]
    pub max_files: usize,
}

#[derive(Debug, Args)]
pub struct RulesImportArgs {
    /// External Cleaner Manifest v1 TOML file to import.
    #[arg(long = "file", value_name = "PATH", value_hint = ValueHint::FilePath)]
    pub file: PathBuf,
}

#[derive(Debug, Args)]
pub struct RulesImportIdArgs {
    /// Imported external rule id from rules import/list.
    #[arg(value_name = "IMPORT_ID")]
    pub import_id: String,
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
    /// Disable the stderr progress spinner; useful for scripts and captured logs.
    #[arg(long)]
    pub no_progress: bool,
    /// Select target-level or throttled file-level progress detail.
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
    #[arg(long = "exclude", value_name = "PATH", value_hint = ValueHint::AnyPath)]
    pub exclude_paths: Vec<PathBuf>,
    /// Write the preview plan to a JSON file for later review and execution.
    #[arg(long = "save-plan", value_name = "FILE", value_hint = ValueHint::FilePath, conflicts_with = "yes")]
    pub save_plan: Option<PathBuf>,
    /// Write a cleanup receipt JSON file after executing with --yes.
    #[arg(long = "receipt", value_name = "FILE", value_hint = ValueHint::FilePath, requires = "yes", conflicts_with = "dry_run")]
    pub receipt: Option<PathBuf>,
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
#[command(after_long_help = CLEAN_AFTER_LONG_HELP)]
pub struct CleanArgs {
    /// Preview the cleanup plan without deleting anything. This is the default unless --yes is set.
    #[arg(short = 'n', long)]
    pub dry_run: bool,
    /// Move allowed targets to the system trash or Recycle Bin instead of previewing.
    #[arg(long)]
    pub yes: bool,
    /// Permanently delete allowed targets. Requires --yes and bypasses the system trash or Recycle Bin.
    #[arg(long, requires = "yes", conflicts_with = "dry_run")]
    pub permanent: bool,
    #[command(flatten)]
    pub selection: CleanupSelectionArgs,
    #[command(flatten)]
    pub execution: CleanupExecutionArgs,
    #[command(flatten)]
    pub risk: RiskArgs,
}

#[derive(Debug, Args)]
#[command(after_long_help = TUI_AFTER_LONG_HELP)]
pub struct TuiArgs {
    /// Directory or file to inspect. Can be repeated. Without roots, the TUI opens a root picker.
    #[arg(long = "root", value_name = "PATH", value_hint = ValueHint::AnyPath)]
    pub roots: Vec<PathBuf>,
    /// Select the scan backend used for disk-map inventory.
    #[arg(long = "scan-backend", value_enum)]
    pub scan_backend: Option<ScanBackendArg>,
    /// Maximum ranked entries loaded into the initial interactive session.
    #[arg(long = "entry-limit", value_name = "N")]
    pub entry_limit: Option<usize>,
    /// Prefer plain text cues and omit visual bars for screen readers.
    #[arg(long = "screen-reader", conflicts_with = "visual_bars")]
    pub screen_reader: bool,
    /// Show visual bars even when saved preferences default to screen-reader mode.
    #[arg(long = "visual-bars", conflicts_with = "screen_reader")]
    pub visual_bars: bool,
    /// Disable color styling in the interactive terminal UI.
    #[arg(long = "no-color", conflicts_with = "color")]
    pub no_color: bool,
    /// Enable color styling even when saved preferences default to no-color mode.
    #[arg(long = "color", conflicts_with = "no_color")]
    pub color: bool,
    /// Render one deterministic frame and exit. Intended for CI and automated smoke tests.
    #[arg(long, hide = true)]
    pub once: bool,
    /// Apply a whitespace-separated key script before rendering --once or entering the TUI.
    #[arg(long = "replay-keys", value_name = "KEYS", hide = true)]
    pub replay_keys: Option<String>,
    /// Width used by the hidden deterministic one-frame renderer.
    #[arg(
        long = "terminal-width",
        value_name = "COLUMNS",
        default_value_t = 120,
        hide = true
    )]
    pub terminal_width: usize,
}

#[derive(Debug, Args)]
#[command(after_long_help = PURGE_AFTER_LONG_HELP)]
pub struct PurgeArgs {
    /// Preview the purge plan without deleting anything.
    #[arg(short = 'n', long)]
    pub dry_run: bool,
    /// Move project artifacts to the system trash or Recycle Bin instead of previewing.
    #[arg(long)]
    pub yes: bool,
    /// Permanently delete project artifacts. Requires --yes and bypasses the system trash or Recycle Bin.
    #[arg(long, requires = "yes", conflicts_with = "dry_run")]
    pub permanent: bool,
    /// Disable the stderr progress spinner; useful for scripts and captured logs.
    #[arg(long)]
    pub no_progress: bool,
    /// Select target-level or throttled file-level progress detail.
    #[arg(long, value_enum, default_value_t = ProgressDetail::Target)]
    pub progress_detail: ProgressDetail,
    /// Use the rebuildable scan cache for eligible target estimates.
    #[arg(long)]
    pub scan_cache: bool,
    /// Disable the rebuildable scan cache for preview estimates.
    #[arg(long, conflicts_with = "scan_cache")]
    pub no_scan_cache: bool,
    /// Directory to scan for project artifacts. Overrides configured purge roots.
    #[arg(long = "root", value_name = "PATH", value_hint = ValueHint::DirPath)]
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
    #[arg(long = "exclude", value_name = "PATH", value_hint = ValueHint::AnyPath)]
    pub exclude_paths: Vec<PathBuf>,
    /// Write the preview plan to a JSON file for later review and execution.
    #[arg(long = "save-plan", value_name = "FILE", value_hint = ValueHint::FilePath, conflicts_with = "yes")]
    pub save_plan: Option<PathBuf>,
    /// Write a cleanup receipt JSON file after executing with --yes.
    #[arg(long = "receipt", value_name = "FILE", value_hint = ValueHint::FilePath, requires = "yes", conflicts_with = "dry_run")]
    pub receipt: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
pub enum PlanCommand {
    /// Read a saved cleanup plan without executing it.
    #[command(after_long_help = PLAN_AFTER_LONG_HELP)]
    Inspect(SavedPlanInspectArgs),
    /// Revalidate a saved cleanup plan, then execute it only with --yes.
    #[command(after_long_help = PLAN_AFTER_LONG_HELP)]
    Run(SavedPlanRunArgs),
}

#[derive(Debug, Args)]
pub struct SavedPlanInspectArgs {
    /// Saved plan JSON produced by --save-plan.
    #[arg(value_name = "FILE", value_hint = ValueHint::FilePath)]
    pub file: PathBuf,
}

#[derive(Debug, Args)]
pub struct SavedPlanRunArgs {
    /// Saved plan JSON produced by --save-plan.
    #[arg(value_name = "FILE", value_hint = ValueHint::FilePath)]
    pub file: PathBuf,
    /// Execute still-valid allowed targets. Without --yes, Rebecca only revalidates the plan.
    #[arg(long)]
    pub yes: bool,
    /// Permanently delete still-valid allowed targets. Requires --yes and bypasses trash.
    #[arg(long, requires = "yes")]
    pub permanent: bool,
    /// Write a cleanup receipt JSON file after executing with --yes.
    #[arg(long = "receipt", value_name = "FILE", value_hint = ValueHint::FilePath, requires = "yes")]
    pub receipt: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct HistoryArgs {
    /// Show only the most recent N history entries.
    #[arg(long)]
    pub limit: Option<NonZeroUsize>,
}

#[derive(Debug, Subcommand)]
pub enum CacheCommand {
    /// Inspect Rebecca cache records without deleting anything.
    Inspect {
        /// Cache namespace to inspect.
        #[arg(long, value_enum, default_value_t = CacheNamespaceArg::All)]
        namespace: CacheNamespaceArg,
    },
    /// Diagnose Rebecca cache health and print prune recommendations.
    Doctor,
    /// Prune Rebecca cache metadata records. Previews by default.
    Prune {
        /// Cache namespace to prune.
        #[arg(long, value_enum, default_value_t = CacheNamespaceArg::All)]
        namespace: CacheNamespaceArg,
        /// Select only stale, corrupt, or orphaned cache records.
        #[arg(long)]
        stale_only: bool,
        /// Maximum number of records to prune.
        #[arg(long, value_name = "N")]
        limit: Option<NonZeroUsize>,
        /// Preview the prune without deleting anything.
        #[arg(long)]
        dry_run: bool,
        /// Delete selected cache metadata records instead of previewing.
        #[arg(long)]
        yes: bool,
    },
    /// Purge Rebecca's rebuildable cache directory.
    Purge {
        /// Preview the purge without deleting anything.
        #[arg(long)]
        dry_run: bool,
        /// Move rebuildable cache entries to the system trash or Recycle Bin instead of previewing them.
        #[arg(long)]
        yes: bool,
        /// Permanently delete rebuildable cache entries. Requires --yes and conflicts with --dry-run.
        #[arg(long, requires = "yes", conflicts_with = "dry_run")]
        permanent: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum TrashCommand {
    /// Preview or empty the system trash. On Windows this uses the Recycle Bin.
    #[command(after_long_help = TRASH_EMPTY_AFTER_LONG_HELP)]
    Empty {
        /// Empty the trash. Without --yes, Rebecca only reports what would be freed.
        #[arg(long)]
        yes: bool,
        /// Windows only: limit the Recycle Bin operation to one drive, such as C or E:. Can be repeated.
        #[arg(long = "drive", value_name = "DRIVE")]
        drives: Vec<String>,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub enum CacheNamespaceArg {
    #[default]
    All,
    ScanCache,
    NtfsVolumeIndex,
}

impl From<CacheNamespaceArg> for rebecca_core::cache::CacheNamespace {
    fn from(namespace: CacheNamespaceArg) -> Self {
        match namespace {
            CacheNamespaceArg::All => Self::All,
            CacheNamespaceArg::ScanCache => Self::ScanCache,
            CacheNamespaceArg::NtfsVolumeIndex => Self::NtfsVolumeIndex,
        }
    }
}

#[derive(Debug, Subcommand)]
pub enum AppsCommand {
    /// Preview leftover app cache data discovered from installed applications.
    Scan {
        /// Disable the stderr progress spinner; useful for scripts and captured logs.
        #[arg(long)]
        no_progress: bool,
        /// Select target-level or throttled file-level progress detail.
        #[arg(long, value_enum, default_value_t = ProgressDetail::Target)]
        progress_detail: ProgressDetail,
        /// Use the rebuildable scan cache for eligible target estimates.
        #[arg(long)]
        scan_cache: bool,
        /// Disable the rebuildable scan cache for preview estimates.
        #[arg(long, conflicts_with = "scan_cache")]
        no_scan_cache: bool,
        /// Exclude a path from app leftovers cleanup for this run. Can be repeated.
        #[arg(long = "exclude", value_name = "PATH", value_hint = ValueHint::AnyPath)]
        exclude_paths: Vec<PathBuf>,
    },
    /// Preview or move leftover app cache data to the system trash or Recycle Bin.
    Clean {
        /// Preview the app leftovers plan without deleting anything.
        #[arg(short = 'n', long)]
        dry_run: bool,
        /// Move leftover app cache data to the system trash or Recycle Bin instead of previewing.
        #[arg(long)]
        yes: bool,
        /// Permanently delete leftover app cache data. Requires --yes and bypasses the system trash or Recycle Bin.
        #[arg(long, requires = "yes", conflicts_with = "dry_run")]
        permanent: bool,
        /// Disable the stderr progress spinner; useful for scripts and captured logs.
        #[arg(long)]
        no_progress: bool,
        /// Select target-level or throttled file-level progress detail.
        #[arg(long, value_enum, default_value_t = ProgressDetail::Target)]
        progress_detail: ProgressDetail,
        /// Use the rebuildable scan cache for eligible target estimates.
        #[arg(long)]
        scan_cache: bool,
        /// Disable the rebuildable scan cache for preview estimates.
        #[arg(long, conflicts_with = "scan_cache")]
        no_scan_cache: bool,
        /// Write the preview plan to a JSON file for later review and execution.
        #[arg(long = "save-plan", value_name = "FILE", value_hint = ValueHint::FilePath, conflicts_with = "yes")]
        save_plan: Option<PathBuf>,
        /// Write a cleanup receipt JSON file after executing with --yes.
        #[arg(long = "receipt", value_name = "FILE", value_hint = ValueHint::FilePath, requires = "yes", conflicts_with = "dry_run")]
        receipt: Option<PathBuf>,
        /// Exclude a path from app leftovers cleanup for this run. Can be repeated.
        #[arg(long = "exclude", value_name = "PATH", value_hint = ValueHint::AnyPath)]
        exclude_paths: Vec<PathBuf>,
    },
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Print config, state, cache, and history paths.
    Paths,
    /// Print the loaded config and effective runtime config.
    Show(ConfigFileArgs),
    /// Validate the current or supplied config file.
    Validate(ConfigFileArgs),
}

#[derive(Debug, Args)]
pub struct ConfigFileArgs {
    /// Config file to read instead of the default Rebecca config.toml.
    #[arg(long = "file", value_name = "PATH", value_hint = ValueHint::FilePath)]
    pub file: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
pub enum DoctorCommand {
    /// Print the current Windows privilege level when available.
    Permissions,
    /// Report warning-bearing cleanup rules whose applications appear to be running.
    ActiveProcesses,
}

#[derive(Debug, Subcommand)]
pub enum SchemaCommand {
    /// Export one CLI API v1 JSON schema document.
    Export(SchemaExportArgs),
}

#[derive(Debug, Subcommand)]
pub enum SkillsCommand {
    /// Install the Rebecca disk-cleaner skill into an agent skills directory.
    Install(SkillsInstallArgs),
    /// Remove the Rebecca disk-cleaner skill from an agent skills directory.
    #[command(alias = "delete", alias = "uninstall")]
    Remove(SkillsRemoveArgs),
    /// Print the resolved Rebecca skill install path.
    Path(SkillsPathArgs),
}

#[derive(Debug, Args)]
pub struct SkillsTargetArgs {
    /// Agent path preset. Defaults to ~/.agents/skills.
    #[arg(long, value_enum, default_value_t = SkillAgentArg::Agents)]
    pub agent: SkillAgentArg,
    /// Skills root directory. Overrides --agent and should be the parent skills directory.
    #[arg(long = "destination", value_name = "SKILLS_DIR", value_hint = ValueHint::DirPath)]
    pub destination: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct SkillsInstallArgs {
    #[command(flatten)]
    pub target: SkillsTargetArgs,
    /// Show the planned install without writing files.
    #[arg(long)]
    pub dry_run: bool,
    /// Replace an existing different Rebecca skill directory.
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, Args)]
pub struct SkillsRemoveArgs {
    #[command(flatten)]
    pub target: SkillsTargetArgs,
    /// Show the planned removal without deleting files.
    #[arg(long)]
    pub dry_run: bool,
    /// Remove the selected skill directory even when Rebecca cannot verify its marker.
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, Args)]
pub struct SkillsPathArgs {
    #[command(flatten)]
    pub target: SkillsTargetArgs,
}

#[derive(Debug, Args)]
pub struct SchemaExportArgs {
    /// Schema document to export.
    #[arg(long = "document", value_enum, default_value_t = SchemaDocumentArg::Payloads)]
    pub document: SchemaDocumentArg,
}

#[derive(Debug, Args)]
pub struct CompletionArgs {
    /// Shell to generate completion for. Defaults to the current shell or bash.
    #[arg(value_enum)]
    pub shell: Option<clap_complete::Shell>,
}

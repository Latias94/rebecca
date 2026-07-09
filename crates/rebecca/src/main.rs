use anyhow::Result;
use clap::error::ErrorKind;
use clap::{CommandFactory, Parser};
use clap_complete::{Shell, generate};
use std::io;

mod apps;
mod cache;
mod cache_view;
mod capabilities;
mod catalog;
mod clean;
mod clean_view;
mod cleanup_receipt;
mod cli;
mod config_cmd;
mod history_view;
mod info;
mod inspect;
mod output;
mod progress;
mod purge;
mod purge_view;
mod render;
mod rules_cmd;
mod runtime;
mod saved_plan;
mod scan;
mod schema;
mod skills;
mod text;
mod trash;
mod trash_backend;
mod tui;
mod workbench;
mod workflow_artifacts;
mod workflow_execution;
mod workflow_planner;

use cli::{
    AppsCommand, CacheCommand, CatalogArgs, CatalogCommand, CleanArgs, Cli, Command,
    CompletionArgs, ConfigCommand, DoctorCommand, HistoryArgs, InspectCommand, OutputMode,
    PlanCommand, PurgeArgs, RulesCommand, ScanArgs, SchemaCommand, SkillsCommand, TrashCommand,
    TuiArgs,
};
use runtime::CliRuntime;

fn main() {
    init_tracing();

    if let Err(err) = run() {
        if output::is_broken_pipe_error(&err) {
            std::process::exit(0);
        }

        if err.downcast_ref::<output::MachineErrorRendered>().is_some() {
            std::process::exit(1);
        }

        let cli = Cli::try_parse();
        let (contract, mode) = cli
            .as_ref()
            .ok()
            .map(|cli| (command_api_contract(&cli.command), command_output_mode(cli)))
            .unwrap_or((
                output::CliApiContract::v1("rebecca", "command-error"),
                OutputMode::Human,
            ));
        output::render_error(contract, mode, &err);
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = if std::env::args_os().len() <= 1 {
        let mut cmd = Cli::command();
        cmd.print_help()?;
        println!();
        return Ok(());
    } else {
        match Cli::try_parse() {
            Ok(cli) => cli,
            Err(err) => {
                handle_cli_parse_error(err)?;
                unreachable!("CLI parse error handler should exit or return an error");
            }
        }
    };

    let runtime = CliRuntime::with_ctrlc_handler()?;

    match cli.command {
        Command::Capabilities => capabilities::run(cli.format),
        Command::Catalog(args) => run_catalog(args, cli.format),
        Command::Rules { command } => match command {
            RulesCommand::Validate(args) => rules_cmd::validate(
                cli.format,
                args.files,
                args.dirs,
                rules_cmd::RuleDiscoveryOptions {
                    max_depth: args.max_depth,
                    max_files: args.max_files,
                },
            ),
            RulesCommand::Import(args) => rules_cmd::import(cli.format, args.file),
            RulesCommand::List => rules_cmd::list(cli.format),
            RulesCommand::Enable(args) => rules_cmd::enable(cli.format, args.import_id),
            RulesCommand::Disable(args) => rules_cmd::disable(cli.format, args.import_id),
            RulesCommand::Remove(args) => rules_cmd::remove(cli.format, args.import_id),
        },
        Command::Scan(args) => run_scan(args, cli.format),
        Command::Clean(args) => run_clean(args, cli.format, &runtime),
        Command::Plan { command } => match command {
            PlanCommand::Inspect(args) => {
                saved_plan::inspect(saved_plan::SavedPlanInspectOptions {
                    output_mode: cli.format,
                    file: args.file,
                })
            }
            PlanCommand::Run(args) => saved_plan::run_with_runtime(
                saved_plan::SavedPlanRunOptions {
                    output_mode: cli.format,
                    file: args.file,
                    yes: args.yes,
                    permanent: args.permanent,
                    receipt: args.receipt,
                },
                &runtime,
            ),
        },
        Command::Tui(args) => run_tui(args, cli.format, &runtime),
        Command::Inspect { command } => run_inspect(command, cli.format, &runtime),
        Command::Purge(args) => run_purge(args, cli.format, &runtime),
        Command::History(args) => run_history(args, cli.format),
        Command::Cache { command } => match command {
            CacheCommand::Inspect { namespace } => cache::inspect(cache::CacheInspectOptions {
                output_mode: cli.format,
                namespace: namespace.into(),
            }),
            CacheCommand::Doctor => cache::doctor(cache::CacheDoctorOptions {
                output_mode: cli.format,
            }),
            CacheCommand::Prune {
                namespace,
                stale_only,
                limit,
                dry_run,
                yes,
            } => cache::prune(cache::CachePruneOptions {
                output_mode: cli.format,
                namespace: namespace.into(),
                stale_only,
                limit,
                dry_run,
                yes,
            }),
            CacheCommand::Purge {
                dry_run,
                yes,
                permanent,
            } => cache::purge(cache::CachePurgeOptions {
                dry_run,
                output_mode: cli.format,
                yes,
                permanent,
            }),
        },
        Command::Apps { command } => match command {
            AppsCommand::Scan {
                no_progress,
                progress_detail,
                scan_cache,
                no_scan_cache,
                exclude_paths,
            } => apps::scan_with_runtime(
                apps::AppsScanOptions {
                    output_mode: cli.format,
                    no_progress,
                    progress_detail,
                    scan_cache: effective_scan_cache(true, scan_cache, no_scan_cache),
                    exclude_paths,
                },
                &runtime,
            ),
            AppsCommand::Clean {
                dry_run,
                yes,
                permanent,
                no_progress,
                progress_detail,
                scan_cache,
                no_scan_cache,
                save_plan,
                receipt,
                exclude_paths,
            } => apps::clean_with_runtime(
                apps::AppsCleanOptions {
                    dry_run,
                    output_mode: cli.format,
                    yes,
                    permanent,
                    no_progress,
                    progress_detail,
                    scan_cache: effective_scan_cache(
                        workflow_is_dry_run(dry_run, yes),
                        scan_cache,
                        no_scan_cache,
                    ),
                    save_plan,
                    receipt,
                    exclude_paths,
                },
                &runtime,
            ),
        },
        Command::Config { command } => match command {
            ConfigCommand::Paths => info::print_config_paths(cli.format),
            ConfigCommand::Show(args) => config_cmd::show(cli.format, args.file),
            ConfigCommand::Validate(args) => config_cmd::validate(cli.format, args.file),
        },
        Command::Doctor { command } => match command {
            DoctorCommand::Permissions => info::print_privilege_level(cli.format),
            DoctorCommand::ActiveProcesses => info::print_active_processes(cli.format),
        },
        Command::Schema { command } => match command {
            SchemaCommand::Export(args) => schema::export(cli.format, args.document),
        },
        Command::Skills { command } => match command {
            SkillsCommand::Install(args) => skills::install(cli.format, args),
            SkillsCommand::Remove(args) => skills::remove(cli.format, args),
            SkillsCommand::Path(args) => skills::path(cli.format, args),
        },
        Command::Trash { command } => match command {
            TrashCommand::Empty { yes, drives } => trash::empty(trash::TrashEmptyOptions {
                output_mode: cli.format,
                yes,
                drives,
            }),
        },
        Command::Completion(args) => run_completion(args),
    }
}

fn handle_cli_parse_error(err: clap::Error) -> Result<()> {
    if matches!(
        err.kind(),
        ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
    ) {
        err.print()?;
        std::process::exit(0);
    }

    if let Some(mode) = detect_machine_output_mode_from_args() {
        let contract = infer_command_api_contract_from_args();
        let err = anyhow::Error::new(err);
        output::render_error(contract, mode, &err);
        return Err(output::MachineErrorRendered.into());
    }

    let exit_code = err.exit_code();
    err.print()?;
    std::process::exit(exit_code);
}

fn detect_machine_output_mode_from_args() -> Option<OutputMode> {
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        let value = arg.to_string_lossy();
        if value == "--format" {
            let format = args.next()?;
            return output_mode_from_raw(&format.to_string_lossy());
        }
        if let Some(format) = value.strip_prefix("--format=") {
            return output_mode_from_raw(format);
        }
    }

    None
}

fn output_mode_from_raw(raw: &str) -> Option<OutputMode> {
    match raw {
        "json" => Some(OutputMode::Json),
        "ndjson" => Some(OutputMode::Ndjson),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy)]
struct CommandApiSpec {
    path: &'static [&'static str],
    command: &'static str,
    payload_kind: &'static str,
}

impl CommandApiSpec {
    const fn new(
        path: &'static [&'static str],
        command: &'static str,
        payload_kind: &'static str,
    ) -> Self {
        Self {
            path,
            command,
            payload_kind,
        }
    }

    fn contract(self) -> output::CliApiContract {
        output::CliApiContract::v1(self.command, self.payload_kind)
    }

    fn matches_tokens(self, tokens: &[String]) -> bool {
        tokens.len() >= self.path.len()
            && self
                .path
                .iter()
                .zip(tokens.iter())
                .all(|(expected, actual)| actual == expected)
    }

    fn matches_path(self, path: &[&str]) -> bool {
        self.path == path
    }
}

const COMMAND_ERROR_PAYLOAD_KIND: &str = "command-error";

const COMMAND_API_SPECS: &[CommandApiSpec] = &[
    CommandApiSpec::new(&["capabilities"], "capabilities", "capabilities"),
    CommandApiSpec::new(
        &["catalog", "validate"],
        "catalog validate",
        "catalog-validation",
    ),
    CommandApiSpec::new(&["catalog"], "catalog", "catalog"),
    CommandApiSpec::new(&["rules", "validate"], "rules validate", "rule-validation"),
    CommandApiSpec::new(&["rules", "import"], "rules import", "rule-import"),
    CommandApiSpec::new(&["rules", "list"], "rules list", "rule-import-list"),
    CommandApiSpec::new(&["rules", "enable"], "rules enable", "rule-import-mutation"),
    CommandApiSpec::new(
        &["rules", "disable"],
        "rules disable",
        "rule-import-mutation",
    ),
    CommandApiSpec::new(&["rules", "remove"], "rules remove", "rule-import-mutation"),
    CommandApiSpec::new(&["rules"], "rules", COMMAND_ERROR_PAYLOAD_KIND),
    CommandApiSpec::new(&["scan"], "scan", "rule-catalog"),
    CommandApiSpec::new(&["clean"], "clean", "cleanup-plan"),
    CommandApiSpec::new(&["plan", "inspect"], "plan inspect", "saved-cleanup-plan"),
    CommandApiSpec::new(&["plan", "run"], "plan run", "cleanup-plan"),
    CommandApiSpec::new(&["plan"], "plan", COMMAND_ERROR_PAYLOAD_KIND),
    CommandApiSpec::new(&["tui"], "tui", COMMAND_ERROR_PAYLOAD_KIND),
    CommandApiSpec::new(&["i"], "tui", COMMAND_ERROR_PAYLOAD_KIND),
    CommandApiSpec::new(&["inspect", "space"], "inspect space", "inspect-space"),
    CommandApiSpec::new(&["inspect", "map"], "inspect map", "inspect-map"),
    CommandApiSpec::new(
        &["inspect", "artifacts"],
        "inspect artifacts",
        "inspect-artifacts",
    ),
    CommandApiSpec::new(&["inspect", "lint"], "inspect lint", "inspect-lint"),
    CommandApiSpec::new(&["inspect"], "inspect", COMMAND_ERROR_PAYLOAD_KIND),
    CommandApiSpec::new(&["purge"], "purge", "project-artifact-cleanup-plan"),
    CommandApiSpec::new(&["history"], "history", "history-list"),
    CommandApiSpec::new(&["cache", "inspect"], "cache inspect", "cache-inventory"),
    CommandApiSpec::new(&["cache", "doctor"], "cache doctor", "cache-doctor"),
    CommandApiSpec::new(&["cache", "prune"], "cache prune", "cache-prune-report"),
    CommandApiSpec::new(&["cache", "purge"], "cache purge", "cache-purge-report"),
    CommandApiSpec::new(&["cache"], "cache", COMMAND_ERROR_PAYLOAD_KIND),
    CommandApiSpec::new(&["apps", "scan"], "apps scan", "app-leftovers-cleanup-plan"),
    CommandApiSpec::new(
        &["apps", "clean"],
        "apps clean",
        "app-leftovers-cleanup-plan",
    ),
    CommandApiSpec::new(&["apps"], "apps", COMMAND_ERROR_PAYLOAD_KIND),
    CommandApiSpec::new(&["config", "paths"], "config paths", "config-paths"),
    CommandApiSpec::new(&["config", "show"], "config show", "config-view"),
    CommandApiSpec::new(
        &["config", "validate"],
        "config validate",
        "config-validation",
    ),
    CommandApiSpec::new(&["config"], "config", COMMAND_ERROR_PAYLOAD_KIND),
    CommandApiSpec::new(
        &["doctor", "permissions"],
        "doctor permissions",
        "permissions-diagnostic",
    ),
    CommandApiSpec::new(
        &["doctor", "active-processes"],
        "doctor active-processes",
        "active-process-diagnostic",
    ),
    CommandApiSpec::new(&["doctor"], "doctor", COMMAND_ERROR_PAYLOAD_KIND),
    CommandApiSpec::new(&["schema", "export"], "schema export", "cli-schema"),
    CommandApiSpec::new(&["schema"], "schema", COMMAND_ERROR_PAYLOAD_KIND),
    CommandApiSpec::new(&["skills", "install"], "skills install", "skill-management"),
    CommandApiSpec::new(&["skills", "remove"], "skills remove", "skill-management"),
    CommandApiSpec::new(&["skills", "delete"], "skills remove", "skill-management"),
    CommandApiSpec::new(
        &["skills", "uninstall"],
        "skills remove",
        "skill-management",
    ),
    CommandApiSpec::new(&["skills", "path"], "skills path", "skill-management"),
    CommandApiSpec::new(&["skills"], "skills", COMMAND_ERROR_PAYLOAD_KIND),
    CommandApiSpec::new(&["trash", "empty"], "trash empty", "trash-report"),
    CommandApiSpec::new(&["trash"], "trash", COMMAND_ERROR_PAYLOAD_KIND),
    CommandApiSpec::new(&["completion"], "completion", COMMAND_ERROR_PAYLOAD_KIND),
];

fn infer_command_api_contract_from_args() -> output::CliApiContract {
    let tokens = positional_tokens_for_command_contract();
    command_api_contract_for_tokens(&tokens)
        .unwrap_or_else(|| output::CliApiContract::v1("rebecca", COMMAND_ERROR_PAYLOAD_KIND))
}

fn command_api_contract_for_tokens(tokens: &[String]) -> Option<output::CliApiContract> {
    COMMAND_API_SPECS
        .iter()
        .copied()
        .filter(|spec| spec.matches_tokens(tokens))
        .max_by_key(|spec| spec.path.len())
        .map(CommandApiSpec::contract)
}

fn command_api_contract_for_path(path: &[&str]) -> output::CliApiContract {
    COMMAND_API_SPECS
        .iter()
        .copied()
        .find(|spec| spec.matches_path(path))
        .unwrap_or_else(|| panic!("missing CLI API contract for command path: {path:?}"))
        .contract()
}

fn positional_tokens_for_command_contract() -> Vec<String> {
    let mut tokens = Vec::new();
    let mut skip_next = false;
    for arg in std::env::args_os().skip(1) {
        if skip_next {
            skip_next = false;
            continue;
        }

        let value = arg.to_string_lossy();
        if value == "--format" {
            skip_next = true;
            continue;
        }
        if value.starts_with("--format=") || value.starts_with('-') {
            continue;
        }
        tokens.push(value.into_owned());
    }

    tokens
}

fn run_tui(args: TuiArgs, global_mode: OutputMode, runtime: &CliRuntime) -> Result<()> {
    tui::run_with_runtime(
        tui::TuiOptions {
            output_mode: global_mode,
            roots: args.roots,
            scan_backend: args.scan_backend.map(Into::into),
            entry_limit: args.entry_limit,
            screen_reader: if args.screen_reader {
                Some(true)
            } else if args.visual_bars {
                Some(false)
            } else {
                None
            },
            no_color: if args.no_color {
                Some(true)
            } else if args.color {
                Some(false)
            } else {
                None
            },
            once: args.once,
            replay_keys: args.replay_keys,
            terminal_width: args.terminal_width,
        },
        runtime,
    )
}

fn run_inspect(
    command: InspectCommand,
    global_mode: OutputMode,
    runtime: &CliRuntime,
) -> Result<()> {
    match command {
        InspectCommand::Space(args) => inspect::space_with_runtime(
            inspect::InspectSpaceOptions {
                output_mode: global_mode,
                no_progress: args.no_progress,
                progress_detail: args.progress_detail,
                scan_cache: args.scan_cache,
                scan_backend: args.scan_backend,
                roots: args.roots,
                top_limit: args.top_limit,
                diagnostic_limit: args.diagnostic_limit,
            },
            runtime,
        ),
        InspectCommand::Map(args) => inspect::map_with_runtime(
            inspect::InspectMapOptions {
                output_mode: global_mode,
                no_progress: args.no_progress,
                progress_detail: args.progress_detail,
                scan_backend: args.scan_backend,
                metadata_profile: args.metadata_profile.into(),
                roots: args.roots,
                top_limit: args.top_limit,
                sort: args.sort.into(),
                min_logical_bytes: args.min_logical_bytes,
                entry_kind: args.entry_kind.map(Into::into),
                path_contains: args.path_contains,
                cleanup_advice: args.cleanup_advice,
                screen_reader: args.screen_reader,
                full_path: args.full_path,
                no_bars: args.no_bars,
                bar_width: args.bar_width,
                advice_status: args.advice_status.map(Into::into),
                group_kinds: args.group_kinds.into_iter().map(Into::into).collect(),
                group_limit: args.group_limit,
                group_sort: args.group_sort.into(),
                table_format: args.table_format.map(|format| match format {
                    cli::InspectMapTableFormatArg::Csv => inspect::InspectMapTableFormat::Csv,
                    cli::InspectMapTableFormatArg::Tsv => inspect::InspectMapTableFormat::Tsv,
                }),
                table_row_kinds: args
                    .table_row_kinds
                    .into_iter()
                    .map(|kind| match kind {
                        cli::InspectMapTableRowKindArg::Total => {
                            inspect::InspectMapTableRowKind::Total
                        }
                        cli::InspectMapTableRowKindArg::Root => {
                            inspect::InspectMapTableRowKind::Root
                        }
                        cli::InspectMapTableRowKindArg::Entry => {
                            inspect::InspectMapTableRowKind::Entry
                        }
                        cli::InspectMapTableRowKindArg::Group => {
                            inspect::InspectMapTableRowKind::Group
                        }
                    })
                    .collect(),
                diagnostic_limit: args.diagnostic_limit,
                max_depth: args.max_depth,
            },
            runtime,
        ),
        InspectCommand::Artifacts(args) => inspect::artifacts_with_runtime(
            inspect::InspectArtifactsOptions {
                output_mode: global_mode,
                no_progress: args.no_progress,
                progress_detail: args.progress_detail,
                scan_cache: args.scan_cache,
                roots: args.roots,
                max_depth: args.max_depth,
                min_age_days: args.min_age_days,
                reclaim_limit_bytes: args.reclaim_limit_bytes,
                artifacts: args.artifacts,
                exclude_paths: args.exclude_paths,
                command: "inspect artifacts",
            },
            runtime,
        ),
        InspectCommand::Lint(args) => inspect::lint_with_runtime(
            inspect::InspectLintOptions {
                output_mode: global_mode,
                roots: args.roots,
                reference_roots: args.reference_roots,
                exclude_paths: args.exclude_paths,
                large_file_threshold_bytes: args.large_file_threshold_bytes,
                top_limit: args.top_limit,
            },
            runtime,
        ),
    }
}

fn run_catalog(args: CatalogArgs, global_mode: OutputMode) -> Result<()> {
    let CatalogArgs {
        command,
        kind,
        categories,
        rules,
        artifacts,
        warnings,
        safety_level,
        platform,
    } = args;

    if let Some(CatalogCommand::Validate) = command {
        return catalog::validate(global_mode);
    }

    catalog::run(catalog::CatalogOptions {
        output_mode: global_mode,
        kind: kind.map(Into::into),
        categories,
        rules,
        artifacts,
        warnings,
        safety_level: safety_level.map(Into::into),
        platform: platform.map(Into::into),
    })
}

fn run_scan(args: ScanArgs, global_mode: OutputMode) -> Result<()> {
    scan::run(global_mode, args.categories, args.rules)
}

fn run_clean(args: CleanArgs, global_mode: OutputMode, runtime: &CliRuntime) -> Result<()> {
    let CleanArgs {
        dry_run,
        yes,
        permanent,
        selection,
        execution,
        risk,
    } = args;
    let is_dry_run = workflow_is_dry_run(dry_run, yes);
    clean::run_with_runtime(
        clean::CleanOptions {
            dry_run,
            output_mode: global_mode,
            yes,
            permanent,
            no_progress: execution.no_progress,
            progress_detail: execution.progress_detail,
            scan_cache: effective_scan_cache(
                is_dry_run,
                execution.scan_cache,
                execution.no_scan_cache,
            ),
            scan_backend: execution.scan_backend.into(),
            categories: selection.categories,
            rules: selection.rules,
            exclude_paths: execution.exclude_paths,
            save_plan: execution.save_plan,
            receipt: execution.receipt,
            allow_moderate: risk.allow_moderate,
            allow_risky: risk.allow_risky,
            allow_warnings: risk.allow_warnings,
        },
        runtime,
    )
}

fn run_purge(args: PurgeArgs, global_mode: OutputMode, runtime: &CliRuntime) -> Result<()> {
    let PurgeArgs {
        dry_run,
        yes,
        permanent,
        no_progress,
        progress_detail,
        scan_cache,
        no_scan_cache,
        roots,
        max_depth,
        min_age_days,
        reclaim_limit_bytes,
        artifacts,
        exclude_paths,
        save_plan,
        receipt,
    } = args;

    purge::run_with_runtime(
        purge::PurgeOptions {
            dry_run,
            output_mode: global_mode,
            yes,
            permanent,
            no_progress,
            progress_detail,
            scan_cache: effective_scan_cache(
                workflow_is_dry_run(dry_run, yes),
                scan_cache,
                no_scan_cache,
            ),
            roots,
            max_depth,
            min_age_days,
            reclaim_limit_bytes,
            artifacts,
            exclude_paths,
            save_plan,
            receipt,
        },
        runtime,
    )
}

fn workflow_is_dry_run(dry_run: bool, yes: bool) -> bool {
    dry_run || !yes
}

fn effective_scan_cache(is_dry_run: bool, scan_cache: bool, no_scan_cache: bool) -> bool {
    !no_scan_cache && (scan_cache || is_dry_run)
}

fn run_history(args: HistoryArgs, global_mode: OutputMode) -> Result<()> {
    info::print_history(global_mode, args.limit)
}

fn run_completion(args: CompletionArgs) -> Result<()> {
    let shell = args.shell.unwrap_or_else(default_completion_shell);
    let mut cmd = Cli::command();
    let bin_name = cmd.get_name().to_owned();
    generate(shell, &mut cmd, bin_name, &mut io::stdout());
    Ok(())
}

fn default_completion_shell() -> Shell {
    Shell::from_env().unwrap_or(Shell::Bash)
}

fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();
}

fn command_output_mode(cli: &Cli) -> OutputMode {
    cli.format
}

fn command_api_contract(command: &Command) -> output::CliApiContract {
    match command {
        Command::Capabilities => command_api_contract_for_path(&["capabilities"]),
        Command::Catalog(args) => {
            if matches!(args.command.as_ref(), Some(CatalogCommand::Validate)) {
                command_api_contract_for_path(&["catalog", "validate"])
            } else {
                command_api_contract_for_path(&["catalog"])
            }
        }
        Command::Rules { command } => match command {
            RulesCommand::Validate(_) => command_api_contract_for_path(&["rules", "validate"]),
            RulesCommand::Import(_) => command_api_contract_for_path(&["rules", "import"]),
            RulesCommand::List => command_api_contract_for_path(&["rules", "list"]),
            RulesCommand::Enable(_) => command_api_contract_for_path(&["rules", "enable"]),
            RulesCommand::Disable(_) => command_api_contract_for_path(&["rules", "disable"]),
            RulesCommand::Remove(_) => command_api_contract_for_path(&["rules", "remove"]),
        },
        Command::Scan(_) => command_api_contract_for_path(&["scan"]),
        Command::Clean(_) => command_api_contract_for_path(&["clean"]),
        Command::Plan { command } => match command {
            PlanCommand::Inspect(_) => command_api_contract_for_path(&["plan", "inspect"]),
            PlanCommand::Run(_) => command_api_contract_for_path(&["plan", "run"]),
        },
        Command::Tui(_) => command_api_contract_for_path(&["tui"]),
        Command::Inspect { command } => match command {
            InspectCommand::Space(_) => command_api_contract_for_path(&["inspect", "space"]),
            InspectCommand::Map(_) => command_api_contract_for_path(&["inspect", "map"]),
            InspectCommand::Artifacts(_) => {
                command_api_contract_for_path(&["inspect", "artifacts"])
            }
            InspectCommand::Lint(_) => command_api_contract_for_path(&["inspect", "lint"]),
        },
        Command::Purge(_) => command_api_contract_for_path(&["purge"]),
        Command::History(_) => command_api_contract_for_path(&["history"]),
        Command::Cache { command } => match command {
            CacheCommand::Inspect { .. } => command_api_contract_for_path(&["cache", "inspect"]),
            CacheCommand::Doctor => command_api_contract_for_path(&["cache", "doctor"]),
            CacheCommand::Prune { .. } => command_api_contract_for_path(&["cache", "prune"]),
            CacheCommand::Purge { .. } => command_api_contract_for_path(&["cache", "purge"]),
        },
        Command::Apps { command } => match command {
            AppsCommand::Scan { .. } => command_api_contract_for_path(&["apps", "scan"]),
            AppsCommand::Clean { .. } => command_api_contract_for_path(&["apps", "clean"]),
        },
        Command::Config { command } => match command {
            ConfigCommand::Paths => command_api_contract_for_path(&["config", "paths"]),
            ConfigCommand::Show(_) => command_api_contract_for_path(&["config", "show"]),
            ConfigCommand::Validate(_) => command_api_contract_for_path(&["config", "validate"]),
        },
        Command::Doctor { command } => match command {
            DoctorCommand::Permissions => command_api_contract_for_path(&["doctor", "permissions"]),
            DoctorCommand::ActiveProcesses => {
                command_api_contract_for_path(&["doctor", "active-processes"])
            }
        },
        Command::Schema { command } => match command {
            SchemaCommand::Export(_) => command_api_contract_for_path(&["schema", "export"]),
        },
        Command::Skills { command } => match command {
            SkillsCommand::Install(_) => command_api_contract_for_path(&["skills", "install"]),
            SkillsCommand::Remove(_) => command_api_contract_for_path(&["skills", "remove"]),
            SkillsCommand::Path(_) => command_api_contract_for_path(&["skills", "path"]),
        },
        Command::Trash { command } => match command {
            TrashCommand::Empty { .. } => command_api_contract_for_path(&["trash", "empty"]),
        },
        Command::Completion(_) => command_api_contract_for_path(&["completion"]),
    }
}

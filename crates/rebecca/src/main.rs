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

fn infer_command_api_contract_from_args() -> output::CliApiContract {
    let tokens = positional_tokens_for_command_contract();
    let command = tokens.first().map(String::as_str);
    match command {
        Some("catalog") => {
            if matches!(tokens.get(1).map(String::as_str), Some("validate")) {
                output::CliApiContract::v1("catalog validate", "catalog-validation")
            } else {
                output::CliApiContract::v1("catalog", "catalog")
            }
        }
        Some("rules") => match tokens.get(1).map(String::as_str) {
            Some("validate") => output::CliApiContract::v1("rules validate", "rule-validation"),
            Some("import") => output::CliApiContract::v1("rules import", "rule-import"),
            Some("list") => output::CliApiContract::v1("rules list", "rule-import-list"),
            Some("enable") => output::CliApiContract::v1("rules enable", "rule-import-mutation"),
            Some("disable") => output::CliApiContract::v1("rules disable", "rule-import-mutation"),
            Some("remove") => output::CliApiContract::v1("rules remove", "rule-import-mutation"),
            _ => output::CliApiContract::v1("rules", "command-error"),
        },
        Some("scan") => output::CliApiContract::v1("scan", "rule-catalog"),
        Some("clean") => output::CliApiContract::v1("clean", "cleanup-plan"),
        Some("plan") => match tokens.get(1).map(String::as_str) {
            Some("inspect") => output::CliApiContract::v1("plan inspect", "saved-cleanup-plan"),
            Some("run") => output::CliApiContract::v1("plan run", "cleanup-plan"),
            _ => output::CliApiContract::v1("plan", "command-error"),
        },
        Some("tui") | Some("i") => output::CliApiContract::v1("tui", "terminal-workbench"),
        Some("inspect") => match tokens.get(1).map(String::as_str) {
            Some("space") => output::CliApiContract::v1("inspect space", "inspect-space"),
            Some("map") => output::CliApiContract::v1("inspect map", "inspect-map"),
            Some("artifacts") => {
                output::CliApiContract::v1("inspect artifacts", "inspect-artifacts")
            }
            Some("lint") => output::CliApiContract::v1("inspect lint", "inspect-lint"),
            _ => output::CliApiContract::v1("inspect", "command-error"),
        },
        Some("purge") => output::CliApiContract::v1("purge", "project-artifact-cleanup-plan"),
        Some("history") => output::CliApiContract::v1("history", "history-list"),
        Some("cache") => match tokens.get(1).map(String::as_str) {
            Some("inspect") => output::CliApiContract::v1("cache inspect", "cache-inventory"),
            Some("doctor") => output::CliApiContract::v1("cache doctor", "cache-doctor"),
            Some("prune") => output::CliApiContract::v1("cache prune", "cache-prune-report"),
            Some("purge") => output::CliApiContract::v1("cache purge", "cache-purge-report"),
            _ => output::CliApiContract::v1("cache", "command-error"),
        },
        Some("apps") => match tokens.get(1).map(String::as_str) {
            Some("scan") => output::CliApiContract::v1("apps scan", "app-leftovers-cleanup-plan"),
            Some("clean") => output::CliApiContract::v1("apps clean", "app-leftovers-cleanup-plan"),
            _ => output::CliApiContract::v1("apps", "command-error"),
        },
        Some("config") => match tokens.get(1).map(String::as_str) {
            Some("paths") => output::CliApiContract::v1("config paths", "config-paths"),
            _ => output::CliApiContract::v1("config", "command-error"),
        },
        Some("doctor") => match tokens.get(1).map(String::as_str) {
            Some("permissions") => {
                output::CliApiContract::v1("doctor permissions", "permissions-diagnostic")
            }
            Some("active-processes") => {
                output::CliApiContract::v1("doctor active-processes", "active-process-diagnostic")
            }
            _ => output::CliApiContract::v1("doctor", "command-error"),
        },
        Some("schema") => output::CliApiContract::v1("schema export", "cli-schema"),
        Some("skills") => match tokens.get(1).map(String::as_str) {
            Some("install") => output::CliApiContract::v1("skills install", "skill-management"),
            Some("remove") | Some("delete") | Some("uninstall") => {
                output::CliApiContract::v1("skills remove", "skill-management")
            }
            Some("path") => output::CliApiContract::v1("skills path", "skill-management"),
            _ => output::CliApiContract::v1("skills", "command-error"),
        },
        Some("trash") => match tokens.get(1).map(String::as_str) {
            Some("empty") => output::CliApiContract::v1("trash empty", "trash-report"),
            _ => output::CliApiContract::v1("trash", "command-error"),
        },
        Some("completion") => output::CliApiContract::v1("completion", "completion-script"),
        _ => output::CliApiContract::v1("rebecca", "command-error"),
    }
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
        Command::Capabilities => output::CliApiContract::v1("capabilities", "capabilities"),
        Command::Catalog(args) => {
            if matches!(args.command.as_ref(), Some(CatalogCommand::Validate)) {
                output::CliApiContract::v1("catalog validate", "catalog-validation")
            } else {
                output::CliApiContract::v1("catalog", "catalog")
            }
        }
        Command::Rules { command } => match command {
            RulesCommand::Validate(_) => {
                output::CliApiContract::v1("rules validate", "rule-validation")
            }
            RulesCommand::Import(_) => output::CliApiContract::v1("rules import", "rule-import"),
            RulesCommand::List => output::CliApiContract::v1("rules list", "rule-import-list"),
            RulesCommand::Enable(_) => {
                output::CliApiContract::v1("rules enable", "rule-import-mutation")
            }
            RulesCommand::Disable(_) => {
                output::CliApiContract::v1("rules disable", "rule-import-mutation")
            }
            RulesCommand::Remove(_) => {
                output::CliApiContract::v1("rules remove", "rule-import-mutation")
            }
        },
        Command::Scan(_) => output::CliApiContract::v1("scan", "rule-catalog"),
        Command::Clean(_) => output::CliApiContract::v1("clean", "cleanup-plan"),
        Command::Plan { command } => match command {
            PlanCommand::Inspect(_) => {
                output::CliApiContract::v1("plan inspect", "saved-cleanup-plan")
            }
            PlanCommand::Run(_) => output::CliApiContract::v1("plan run", "cleanup-plan"),
        },
        Command::Tui(_) => output::CliApiContract::v1("tui", "terminal-workbench"),
        Command::Inspect { command } => match command {
            InspectCommand::Space(_) => {
                output::CliApiContract::v1("inspect space", "inspect-space")
            }
            InspectCommand::Map(_) => output::CliApiContract::v1("inspect map", "inspect-map"),
            InspectCommand::Artifacts(_) => {
                output::CliApiContract::v1("inspect artifacts", "inspect-artifacts")
            }
            InspectCommand::Lint(_) => output::CliApiContract::v1("inspect lint", "inspect-lint"),
        },
        Command::Purge(_) => output::CliApiContract::v1("purge", "project-artifact-cleanup-plan"),
        Command::History(_) => output::CliApiContract::v1("history", "history-list"),
        Command::Cache { command } => match command {
            CacheCommand::Inspect { .. } => {
                output::CliApiContract::v1("cache inspect", "cache-inventory")
            }
            CacheCommand::Doctor => output::CliApiContract::v1("cache doctor", "cache-doctor"),
            CacheCommand::Prune { .. } => {
                output::CliApiContract::v1("cache prune", "cache-prune-report")
            }
            CacheCommand::Purge { .. } => {
                output::CliApiContract::v1("cache purge", "cache-purge-report")
            }
        },
        Command::Apps { command } => match command {
            AppsCommand::Scan { .. } => {
                output::CliApiContract::v1("apps scan", "app-leftovers-cleanup-plan")
            }
            AppsCommand::Clean { .. } => {
                output::CliApiContract::v1("apps clean", "app-leftovers-cleanup-plan")
            }
        },
        Command::Config { command } => match command {
            ConfigCommand::Paths => output::CliApiContract::v1("config paths", "config-paths"),
            ConfigCommand::Show(_) => output::CliApiContract::v1("config show", "config-view"),
            ConfigCommand::Validate(_) => {
                output::CliApiContract::v1("config validate", "config-validation")
            }
        },
        Command::Doctor { command } => match command {
            DoctorCommand::Permissions => {
                output::CliApiContract::v1("doctor permissions", "permissions-diagnostic")
            }
            DoctorCommand::ActiveProcesses => {
                output::CliApiContract::v1("doctor active-processes", "active-process-diagnostic")
            }
        },
        Command::Schema { command } => match command {
            SchemaCommand::Export(_) => output::CliApiContract::v1("schema export", "cli-schema"),
        },
        Command::Skills { command } => match command {
            SkillsCommand::Install(_) => {
                output::CliApiContract::v1("skills install", "skill-management")
            }
            SkillsCommand::Remove(_) => {
                output::CliApiContract::v1("skills remove", "skill-management")
            }
            SkillsCommand::Path(_) => output::CliApiContract::v1("skills path", "skill-management"),
        },
        Command::Trash { command } => match command {
            TrashCommand::Empty { .. } => output::CliApiContract::v1("trash empty", "trash-report"),
        },
        Command::Completion(_) => output::CliApiContract::v1("completion", "completion-script"),
    }
}

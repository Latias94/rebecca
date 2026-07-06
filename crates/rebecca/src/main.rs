use anyhow::Result;
use clap::{CommandFactory, Parser};
use clap_complete::{Shell, generate};
use std::io;

mod apps;
mod cache;
mod cache_view;
mod catalog;
mod clean;
mod clean_view;
mod cli;
mod history_view;
mod info;
mod inspect;
mod output;
mod purge;
mod purge_view;
mod render;
mod runtime;
mod scan;
mod text;

use cli::{
    AppsCommand, CacheCommand, CatalogArgs, CatalogCommand, CleanArgs, Cli, Command,
    CompletionArgs, ConfigCommand, DoctorCommand, HistoryArgs, InspectCommand, OutputMode,
    PurgeArgs, PurgeCommand, ScanArgs,
};
use runtime::CliRuntime;

fn main() {
    init_tracing();

    if let Err(err) = run() {
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
        Cli::parse()
    };

    let runtime = CliRuntime::with_ctrlc_handler()?;

    match cli.command {
        Command::Catalog(args) => run_catalog(args, cli.format),
        Command::Scan(args) => run_scan(args, cli.format),
        Command::Clean(args) => run_clean(args, cli.format, &runtime),
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
                no_progress,
                progress_detail,
                scan_cache,
                no_scan_cache,
                exclude_paths,
            } => apps::clean_with_runtime(
                apps::AppsCleanOptions {
                    dry_run,
                    output_mode: cli.format,
                    yes,
                    no_progress,
                    progress_detail,
                    scan_cache: effective_scan_cache(
                        workflow_is_dry_run(dry_run, yes),
                        scan_cache,
                        no_scan_cache,
                    ),
                    exclude_paths,
                },
                &runtime,
            ),
        },
        Command::Config { command } => match command {
            ConfigCommand::Paths => info::print_config_paths(cli.format),
        },
        Command::Doctor { command } => match command {
            DoctorCommand::Permissions => info::print_privilege_level(cli.format),
            DoctorCommand::ActiveProcesses => info::print_active_processes(cli.format),
        },
        Command::Completion(args) => run_completion(args),
    }
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
    })
}

fn run_scan(args: ScanArgs, global_mode: OutputMode) -> Result<()> {
    scan::run(global_mode, args.categories, args.rules)
}

fn run_clean(args: CleanArgs, global_mode: OutputMode, runtime: &CliRuntime) -> Result<()> {
    let CleanArgs {
        dry_run,
        yes,
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
            allow_moderate: risk.allow_moderate,
            allow_risky: risk.allow_risky,
            allow_warnings: risk.allow_warnings,
        },
        runtime,
    )
}

fn run_purge(args: PurgeArgs, global_mode: OutputMode, runtime: &CliRuntime) -> Result<()> {
    let PurgeArgs {
        command,
        dry_run,
        yes,
        no_progress,
        progress_detail,
        scan_cache,
        no_scan_cache,
        list_artifacts,
        roots,
        max_depth,
        min_age_days,
        reclaim_limit_bytes,
        artifacts,
        exclude_paths,
    } = args;

    if let Some(PurgeCommand::Inspect(args)) = command {
        return purge::inspect_with_runtime(
            purge::PurgeInspectOptions {
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
            },
            runtime,
        );
    }

    purge::run_with_runtime(
        purge::PurgeOptions {
            dry_run,
            output_mode: global_mode,
            yes,
            no_progress,
            progress_detail,
            scan_cache: effective_scan_cache(
                workflow_is_dry_run(dry_run, yes),
                scan_cache,
                no_scan_cache,
            ),
            list_artifacts,
            roots,
            max_depth,
            min_age_days,
            reclaim_limit_bytes,
            artifacts,
            exclude_paths,
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
        Command::Catalog(args) => {
            if matches!(args.command.as_ref(), Some(CatalogCommand::Validate)) {
                output::CliApiContract::v1("catalog validate", "catalog-validation")
            } else {
                output::CliApiContract::v1("catalog", "catalog")
            }
        }
        Command::Scan(_) => output::CliApiContract::v1("scan", "rule-catalog"),
        Command::Clean(_) => output::CliApiContract::v1("clean", "cleanup-plan"),
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
        Command::Purge(args) => {
            if matches!(args.command, Some(PurgeCommand::Inspect(_))) {
                output::CliApiContract::v1("purge inspect", "inspect-artifacts")
            } else if args.list_artifacts {
                output::CliApiContract::v1("purge", "project-artifact-catalog")
            } else {
                output::CliApiContract::v1("purge", "project-artifact-cleanup-plan")
            }
        }
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
        },
        Command::Doctor { command } => match command {
            DoctorCommand::Permissions => {
                output::CliApiContract::v1("doctor permissions", "permissions-diagnostic")
            }
            DoctorCommand::ActiveProcesses => {
                output::CliApiContract::v1("doctor active-processes", "active-process-diagnostic")
            }
        },
        Command::Completion(_) => output::CliApiContract::v1("completion", "completion-script"),
    }
}

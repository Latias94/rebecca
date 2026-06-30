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
    AppsCommand, CacheCommand, CatalogArgs, CleanArgs, Cli, Command, CompletionArgs, ConfigCommand,
    DoctorCommand, HistoryArgs, InspectCommand, OutputMode, PurgeArgs, PurgeCommand, ScanArgs,
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
            CacheCommand::Purge { dry_run, yes } => cache::purge(cache::CachePurgeOptions {
                dry_run,
                output_mode: cli.format,
                yes,
            }),
        },
        Command::Apps { command } => match command {
            AppsCommand::Scan {
                no_progress,
                scan_cache,
                exclude_paths,
            } => apps::scan_with_runtime(
                apps::AppsScanOptions {
                    output_mode: cli.format,
                    no_progress,
                    scan_cache,
                    exclude_paths,
                },
                &runtime,
            ),
            AppsCommand::Clean {
                dry_run,
                yes,
                no_progress,
                scan_cache,
                exclude_paths,
            } => apps::clean_with_runtime(
                apps::AppsCleanOptions {
                    dry_run,
                    output_mode: cli.format,
                    yes,
                    no_progress,
                    scan_cache,
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
                roots: args.roots,
                top_limit: args.top_limit,
            },
            runtime,
        ),
        InspectCommand::Artifacts(args) => inspect::artifacts_with_runtime(
            inspect::InspectArtifactsOptions {
                output_mode: global_mode,
                no_progress: args.no_progress,
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
    catalog::run(catalog::CatalogOptions {
        output_mode: global_mode,
        kind: args.kind.map(Into::into),
        categories: args.categories,
        rules: args.rules,
        artifacts: args.artifacts,
        warnings: args.warnings,
        safety_level: args.safety_level.map(Into::into),
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
    clean::run_with_runtime(
        clean::CleanOptions {
            dry_run,
            output_mode: global_mode,
            yes,
            no_progress: execution.no_progress,
            scan_cache: execution.scan_cache,
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
        scan_cache,
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
            scan_cache,
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
        Command::Catalog(_) => output::CliApiContract::v2("catalog", "catalog"),
        Command::Scan(_) => output::CliApiContract::v1("scan", "rule-catalog"),
        Command::Clean(_) => output::CliApiContract::v1("clean", "cleanup-plan"),
        Command::Inspect { command } => match command {
            InspectCommand::Space(_) => {
                output::CliApiContract::v2("inspect space", "inspect-space")
            }
            InspectCommand::Artifacts(_) => {
                output::CliApiContract::v2("inspect artifacts", "inspect-artifacts")
            }
            InspectCommand::Lint(_) => output::CliApiContract::v2("inspect lint", "inspect-lint"),
        },
        Command::Purge(args) => {
            if matches!(args.command, Some(PurgeCommand::Inspect(_))) {
                output::CliApiContract::v2("purge inspect", "inspect-artifacts")
            } else if args.list_artifacts {
                output::CliApiContract::v1("purge", "project-artifact-catalog")
            } else {
                output::CliApiContract::v1("purge", "project-artifact-cleanup-plan")
            }
        }
        Command::History(_) => output::CliApiContract::v1("history", "history-list"),
        Command::Cache { command } => match command {
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

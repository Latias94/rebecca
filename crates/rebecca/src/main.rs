use anyhow::Result;
use clap::{CommandFactory, Parser};
use clap_complete::{Shell, generate};
use std::io;

mod apps;
mod cache;
mod cache_view;
mod clean;
mod clean_view;
mod cli;
mod history_view;
mod info;
mod output;
mod purge;
mod purge_view;
mod render;
mod runtime;
mod scan;
mod text;

use cli::{
    AppsCommand, CacheCommand, CleanArgs, Cli, Command, CompletionArgs, ConfigCommand,
    DoctorCommand, HistoryArgs, OutputMode, PurgeArgs, PurgeCommand, ScanArgs,
};
use runtime::CliRuntime;

fn main() {
    init_tracing();

    if let Err(err) = run() {
        if err.downcast_ref::<output::MachineErrorRendered>().is_some() {
            std::process::exit(1);
        }

        let cli = Cli::try_parse();
        let (command, mode) = cli
            .as_ref()
            .ok()
            .map(|cli| (command_name(&cli.command), command_output_mode(cli)))
            .unwrap_or(("rebecca", OutputMode::Human));
        output::render_error(command, mode, &err);
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
        Command::Scan(args) => run_scan(args, cli.format),
        Command::Clean(args) => run_clean(args, cli.format, &runtime),
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
        },
        Command::Completion(args) => run_completion(args),
    }
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

fn command_name(command: &Command) -> &'static str {
    match command {
        Command::Scan(_) => "scan",
        Command::Clean(_) => "clean",
        Command::Purge(args) => {
            if matches!(args.command, Some(PurgeCommand::Inspect(_))) {
                "purge inspect"
            } else {
                "purge"
            }
        }
        Command::History(_) => "history",
        Command::Cache { command } => match command {
            CacheCommand::Purge { .. } => "cache purge",
        },
        Command::Apps { command } => match command {
            AppsCommand::Scan { .. } => "apps scan",
            AppsCommand::Clean { .. } => "apps clean",
        },
        Command::Config { command } => match command {
            ConfigCommand::Paths => "config paths",
        },
        Command::Doctor { command } => match command {
            DoctorCommand::Permissions => "doctor permissions",
        },
        Command::Completion(_) => "completion",
    }
}

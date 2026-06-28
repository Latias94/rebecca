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
mod scan;

use cli::{
    AppsCommand, CacheCommand, CleanArgs, Cli, Command, CompletionArgs, ConfigCommand,
    DoctorCommand, HistoryArgs, PurgeArgs, ScanArgs,
};

fn main() -> Result<()> {
    init_tracing();

    let cli = if std::env::args_os().len() <= 1 {
        let mut cmd = Cli::command();
        cmd.print_help()?;
        println!();
        return Ok(());
    } else {
        Cli::parse()
    };

    match cli.command {
        Command::Scan(args) => run_scan(args),
        Command::Clean(args) => run_clean(args),
        Command::Purge(args) => run_purge(args),
        Command::History(args) => run_history(args),
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
        Command::Completion(args) => run_completion(args),
    }
}

fn run_scan(args: ScanArgs) -> Result<()> {
    scan::run(args.json, args.categories, args.rules)
}

fn run_clean(args: CleanArgs) -> Result<()> {
    let CleanArgs {
        dry_run,
        json,
        yes,
        selection,
        execution,
        risk,
    } = args;
    clean::run(clean::CleanOptions {
        dry_run,
        json,
        yes,
        no_progress: execution.no_progress,
        scan_cache: execution.scan_cache,
        categories: selection.categories,
        rules: selection.rules,
        exclude_paths: execution.exclude_paths,
        allow_moderate: risk.allow_moderate,
        allow_risky: risk.allow_risky,
    })
}

fn run_purge(args: PurgeArgs) -> Result<()> {
    purge::run(purge::PurgeOptions {
        dry_run: args.dry_run,
        json: args.json,
        yes: args.yes,
        no_progress: args.no_progress,
        scan_cache: args.scan_cache,
        list_artifacts: args.list_artifacts,
        roots: args.roots,
        max_depth: args.max_depth,
        min_age_days: args.min_age_days,
        artifacts: args.artifacts,
        exclude_paths: args.exclude_paths,
    })
}

fn run_history(args: HistoryArgs) -> Result<()> {
    info::print_history(args.json, args.limit)
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

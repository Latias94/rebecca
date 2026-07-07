use std::io::{self, IsTerminal};
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use rebecca::core::config::{AppRuntimeConfig, load_runtime_config};
use rebecca::core::history::{HistoryEntry, HistoryStore};
use rebecca::core::scan::ScanBackendKind;

use crate::cli::OutputMode;
use crate::runtime::CliRuntime;
use crate::tui::app::{RootChoice, TuiApp, TuiEffect};

mod app;
mod task;
mod terminal;
mod view;

#[derive(Debug)]
pub(crate) struct TuiOptions {
    pub(crate) output_mode: OutputMode,
    pub(crate) roots: Vec<PathBuf>,
    pub(crate) scan_backend: ScanBackendKind,
    pub(crate) entry_limit: usize,
    pub(crate) screen_reader: bool,
    pub(crate) no_color: bool,
    pub(crate) once: bool,
    pub(crate) replay_keys: Option<String>,
    pub(crate) terminal_width: usize,
}

pub(crate) fn run_with_runtime(options: TuiOptions, runtime: &CliRuntime) -> Result<()> {
    if !options.output_mode.is_human() {
        bail!("rebecca tui requires --format human because it owns the terminal screen");
    }
    if !options.once && !io::stdout().is_terminal() {
        bail!("rebecca tui requires an interactive terminal; use inspect map for scripts");
    }

    let runtime_config = load_runtime_config()?;
    let view_options = view::ViewOptions {
        width: options.terminal_width,
        visual_bars: !options.screen_reader,
        color: !options.no_color,
    };

    if options.once || options.replay_keys.is_some() {
        let mut app = initial_app(&options, &runtime_config, runtime)?;
        app.set_history(load_recent_history(&runtime_config)?);
        if let Some(keys) = options.replay_keys.as_deref() {
            drive_replay(&mut app, keys, &runtime_config, runtime)?;
        }
        if !options.once {
            run_interactive(app, None, view_options, &runtime_config, runtime)?;
            return Ok(());
        }
        println!("{}", view::snapshot(&app, view_options));
        return Ok(());
    }

    let (mut app, startup_effect) = interactive_initial_app(&options)?;
    app.set_history(load_recent_history(&runtime_config)?);
    run_interactive(app, startup_effect, view_options, &runtime_config, runtime)
}

fn run_interactive(
    mut app: TuiApp,
    startup_effect: Option<TuiEffect>,
    view_options: view::ViewOptions,
    runtime_config: &AppRuntimeConfig,
    runtime: &CliRuntime,
) -> Result<()> {
    let mut terminal = terminal::TerminalGuard::enter()?;
    let mut active_task = match startup_effect {
        Some(effect) => task::start(&mut app, effect, runtime_config, runtime)?,
        None => None,
    };
    while !app.should_quit() {
        task::poll(&mut app, &mut active_task, runtime_config)?;
        terminal
            .terminal_mut()
            .draw(|frame| view::render(frame, &app, view_options))?;
        if let Some(key) = terminal::poll_key(Duration::from_millis(120))? {
            let effect = app.handle_key(key);
            if matches!(effect, TuiEffect::CancelTask) {
                if let Some(task) = active_task.as_ref() {
                    task.cancel();
                    app.apply_cancel_requested();
                } else {
                    app.apply_error("no active background task to cancel");
                }
            } else if active_task.is_some() && starts_background_task(&effect) {
                app.apply_task_already_running();
            } else if let Some(task) = task::start(&mut app, effect, runtime_config, runtime)? {
                active_task = Some(task);
            }
        }
    }

    if let Some(task) = active_task.take() {
        task.cancel_and_join();
    }

    Ok(())
}

fn interactive_initial_app(options: &TuiOptions) -> Result<(TuiApp, Option<TuiEffect>)> {
    if options.roots.is_empty() {
        return Ok((
            TuiApp::root_picker(root_choices()?, options.scan_backend, options.entry_limit),
            None,
        ));
    }

    Ok((
        TuiApp::root_picker(Vec::new(), options.scan_backend, options.entry_limit),
        Some(TuiEffect::Scan(options.roots.clone())),
    ))
}

fn initial_app(
    options: &TuiOptions,
    runtime_config: &AppRuntimeConfig,
    runtime: &CliRuntime,
) -> Result<TuiApp> {
    if options.roots.is_empty() {
        return Ok(TuiApp::root_picker(
            root_choices()?,
            options.scan_backend,
            options.entry_limit,
        ));
    }

    let session = task::scan_session(
        options.roots.clone(),
        options.entry_limit,
        options.scan_backend,
        runtime_config,
        runtime,
    )?;
    Ok(TuiApp::from_session(
        session,
        options.scan_backend,
        options.entry_limit,
    ))
}

fn drive_replay(
    app: &mut TuiApp,
    keys: &str,
    runtime_config: &AppRuntimeConfig,
    runtime: &CliRuntime,
) -> Result<()> {
    for token in keys.split_whitespace() {
        let Some(key) = terminal::replay_token_to_key(token) else {
            bail!("unknown tui replay key token: {token}");
        };
        let effect = app.handle_key(key);
        handle_effect(app, effect, runtime_config, runtime)?;
        if app.should_quit() {
            break;
        }
    }
    Ok(())
}

fn handle_effect(
    app: &mut TuiApp,
    effect: TuiEffect,
    runtime_config: &AppRuntimeConfig,
    runtime: &CliRuntime,
) -> Result<()> {
    match effect {
        TuiEffect::None | TuiEffect::CancelTask | TuiEffect::Quit => {}
        TuiEffect::Scan(roots) => match task::scan_session(
            roots,
            app.entry_limit,
            app.scan_backend,
            runtime_config,
            runtime,
        ) {
            Ok(session) => app.apply_scan_result(session),
            Err(err) => app.apply_error(err.to_string()),
        },
        TuiEffect::Refresh(roots) => {
            let retry = TuiEffect::Refresh(roots.clone());
            app.prepare_refresh();
            match task::scan_session(
                roots,
                app.entry_limit,
                app.scan_backend,
                runtime_config,
                runtime,
            ) {
                Ok(session) => app.apply_refresh_result(session),
                Err(err) => app.apply_task_error(err.to_string(), retry),
            }
        }
        TuiEffect::Preview(request) => {
            match crate::workbench::preview_cleanup_plan(&request, runtime_config, runtime) {
                Ok(plan) => app.apply_preview(plan),
                Err(err) => app.apply_error(err.to_string()),
            }
        }
        TuiEffect::Execute(request) => {
            match crate::workbench::execute_recoverable_cleanup(&request, runtime_config, runtime) {
                Ok(plan) => {
                    app.apply_execution(plan);
                    app.set_history(load_recent_history(runtime_config)?);
                }
                Err(err) => app.apply_error(err.to_string()),
            }
        }
    }
    Ok(())
}

fn starts_background_task(effect: &TuiEffect) -> bool {
    matches!(
        effect,
        TuiEffect::Scan(_) | TuiEffect::Refresh(_) | TuiEffect::Preview(_) | TuiEffect::Execute(_)
    )
}

fn root_choices() -> Result<Vec<RootChoice>> {
    let mut choices = vec![RootChoice {
        label: "current".to_string(),
        path: std::env::current_dir().context("failed to resolve current directory")?,
    }];

    if let Some(home) = home_dir()
        && !choices.iter().any(|choice| choice.path == home)
    {
        choices.push(RootChoice {
            label: "home".to_string(),
            path: home,
        });
    }

    #[cfg(windows)]
    {
        for letter in 'C'..='Z' {
            let path = PathBuf::from(format!("{letter}:\\"));
            if path.exists() && !choices.iter().any(|choice| choice.path == path) {
                choices.push(RootChoice {
                    label: format!("drive {letter}"),
                    path,
                });
            }
        }
    }

    #[cfg(unix)]
    {
        let path = PathBuf::from("/");
        if !choices.iter().any(|choice| choice.path == path) {
            choices.push(RootChoice {
                label: "filesystem".to_string(),
                path,
            });
        }
    }

    Ok(choices)
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

fn load_recent_history(runtime_config: &AppRuntimeConfig) -> Result<Vec<HistoryEntry>> {
    let Some(limit) = NonZeroUsize::new(5) else {
        return Ok(Vec::new());
    };
    let report = HistoryStore::new(runtime_config.app_paths.history_file.clone())
        .load_tail_report(limit)
        .context("failed to load cleanup history")?;
    Ok(report.entries)
}

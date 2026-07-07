use std::io::{self, IsTerminal};
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use rebecca::core::config::{AppRuntimeConfig, load_runtime_config};
use rebecca::core::disk_map::DiskMapSortField;
use rebecca::core::history::{HistoryEntry, HistoryStore};
use rebecca::core::scan::ScanBackendKind;

use crate::cli::OutputMode;
use crate::runtime::CliRuntime;
use crate::tui::app::TuiApp;
use crate::tui::effect::TuiEffect;
use crate::tui::input::TuiInput;
use crate::tui::navigation::RootChoice;
use crate::tui::preferences::{TuiPreferences, preferences_path};

mod app;
mod basket;
mod effect;
mod hit_test;
mod input;
mod layout;
mod model;
mod navigation;
mod preferences;
mod progress;
mod projection;
mod replay;
mod snapshot;
mod task;
mod terminal;
mod treemap;
mod view;

const DEFAULT_ENTRY_LIMIT: usize = 2_000;
const DEFAULT_SCAN_BACKEND: ScanBackendKind = ScanBackendKind::PortableRecursive;
const DEFAULT_SORT: DiskMapSortField = DiskMapSortField::Logical;

#[derive(Debug)]
pub(crate) struct TuiOptions {
    pub(crate) output_mode: OutputMode,
    pub(crate) roots: Vec<PathBuf>,
    pub(crate) scan_backend: Option<ScanBackendKind>,
    pub(crate) entry_limit: Option<usize>,
    pub(crate) screen_reader: Option<bool>,
    pub(crate) no_color: Option<bool>,
    pub(crate) once: bool,
    pub(crate) replay_keys: Option<String>,
    pub(crate) terminal_width: usize,
}

#[derive(Debug, Clone, Copy)]
struct ResolvedTuiOptions {
    scan_backend: ScanBackendKind,
    entry_limit: usize,
    sort: DiskMapSortField,
    screen_reader: bool,
    no_color: bool,
    preferred_screen: Option<crate::tui::model::TuiScreen>,
}

impl TuiOptions {
    fn resolve(&self, preferences: &TuiPreferences) -> ResolvedTuiOptions {
        ResolvedTuiOptions {
            scan_backend: self
                .scan_backend
                .or(preferences.scan_backend)
                .unwrap_or(DEFAULT_SCAN_BACKEND),
            entry_limit: self
                .entry_limit
                .or(preferences.entry_limit)
                .unwrap_or(DEFAULT_ENTRY_LIMIT),
            sort: preferences.sort.unwrap_or(DEFAULT_SORT),
            screen_reader: self
                .screen_reader
                .or(preferences.screen_reader)
                .unwrap_or(false),
            no_color: self.no_color.or(preferences.no_color).unwrap_or(false),
            preferred_screen: preferences.last_screen,
        }
    }

    fn should_save_preferences(&self) -> bool {
        !self.once && self.replay_keys.is_none()
    }
}

pub(crate) fn run_with_runtime(options: TuiOptions, runtime: &CliRuntime) -> Result<()> {
    if !options.output_mode.is_human() {
        bail!("rebecca tui requires --format human because it owns the terminal screen");
    }
    if !options.once && !io::stdout().is_terminal() {
        bail!("rebecca tui requires an interactive terminal; use inspect map for scripts");
    }

    let runtime_config = load_runtime_config()?;
    let preference_path = preferences_path(&runtime_config);
    let preference_load = TuiPreferences::load(&preference_path);
    let resolved_options = options.resolve(&preference_load.preferences);
    let view_options = view::ViewOptions {
        width: options.terminal_width,
        visual_bars: !resolved_options.screen_reader,
        color: !resolved_options.no_color,
    };

    if options.once || options.replay_keys.is_some() {
        let mut app = initial_app(&options, resolved_options, &runtime_config, runtime)?;
        if let Some(warning) = preference_load.warning.clone() {
            app.message = warning;
        }
        app.set_history(load_recent_history(&runtime_config)?);
        if let Some(keys) = options.replay_keys.as_deref() {
            replay::drive(
                &mut app,
                keys,
                options.terminal_width,
                &runtime_config,
                runtime,
            )?;
        }
        if !options.once {
            run_interactive(app, None, view_options, &runtime_config, runtime, None)?;
            return Ok(());
        }
        println!("{}", snapshot::snapshot(&app, view_options));
        return Ok(());
    }

    let (mut app, startup_effect) = interactive_initial_app(&options, resolved_options)?;
    if let Some(warning) = preference_load.warning {
        app.message = warning;
    }
    app.set_history(load_recent_history(&runtime_config)?);
    let preference_save_path = options.should_save_preferences().then_some(preference_path);
    run_interactive(
        app,
        startup_effect,
        view_options,
        &runtime_config,
        runtime,
        preference_save_path,
    )
}

fn run_interactive(
    mut app: TuiApp,
    startup_effect: Option<TuiEffect>,
    view_options: view::ViewOptions,
    runtime_config: &AppRuntimeConfig,
    runtime: &CliRuntime,
    preference_save_path: Option<PathBuf>,
) -> Result<()> {
    let mut terminal = terminal::TerminalGuard::enter()?;
    let mut task_manager = task::TuiTaskManager::new();
    if let Some(effect) = startup_effect {
        task_manager.handle_effect(&mut app, effect, runtime_config, runtime)?;
    }
    while !app.should_quit() {
        task_manager.poll(&mut app, runtime_config)?;
        let mut draw_area = ratatui::layout::Rect::new(0, 0, 0, 0);
        terminal.terminal_mut().draw(|frame| {
            draw_area = frame.area();
            view::render(frame, &app, view_options);
        })?;
        if let Some(input) = terminal::poll_input(Duration::from_millis(120))? {
            let effect = match input {
                TuiInput::Key(key) => app.handle_key(key),
                TuiInput::Mouse(mouse) => hit_test::hit_test(&app, view_options, draw_area, mouse)
                    .map(|action| app.handle_mouse_action(action))
                    .unwrap_or(TuiEffect::None),
            };
            task_manager.handle_effect(&mut app, effect, runtime_config, runtime)?;
        }
    }

    task_manager.shutdown();
    if let Some(path) = preference_save_path {
        TuiPreferences::from_app(&app, view_options).save(&path)?;
    }

    Ok(())
}

fn interactive_initial_app(
    options: &TuiOptions,
    resolved_options: ResolvedTuiOptions,
) -> Result<(TuiApp, Option<TuiEffect>)> {
    if options.roots.is_empty() {
        let mut app = TuiApp::root_picker(
            root_choices()?,
            resolved_options.scan_backend,
            resolved_options.entry_limit,
        );
        app.set_pending_initial_screen(resolved_options.preferred_screen);
        app.sort = resolved_options.sort;
        return Ok((app, None));
    }

    let mut app = TuiApp::root_picker(
        Vec::new(),
        resolved_options.scan_backend,
        resolved_options.entry_limit,
    );
    app.set_pending_initial_screen(resolved_options.preferred_screen);
    app.sort = resolved_options.sort;
    Ok((app, Some(TuiEffect::Scan(options.roots.clone()))))
}

fn initial_app(
    options: &TuiOptions,
    resolved_options: ResolvedTuiOptions,
    runtime_config: &AppRuntimeConfig,
    runtime: &CliRuntime,
) -> Result<TuiApp> {
    if options.roots.is_empty() {
        let mut app = TuiApp::root_picker(
            root_choices()?,
            resolved_options.scan_backend,
            resolved_options.entry_limit,
        );
        app.sort = resolved_options.sort;
        app.set_pending_initial_screen(resolved_options.preferred_screen);
        return Ok(app);
    }

    let session = task::scan_session(
        options.roots.clone(),
        resolved_options.entry_limit,
        resolved_options.scan_backend,
        runtime_config,
        runtime,
    )?;
    let mut app = TuiApp::from_session(
        session,
        resolved_options.scan_backend,
        resolved_options.entry_limit,
    );
    app.sort = resolved_options.sort;
    app.set_pending_initial_screen(resolved_options.preferred_screen);
    Ok(app)
}

pub(super) fn handle_effect(
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
        TuiEffect::Refresh { anchor } => {
            let retry = TuiEffect::Refresh {
                anchor: anchor.clone(),
            };
            match task::scan_session(
                vec![anchor.clone()],
                app.entry_limit,
                app.scan_backend,
                runtime_config,
                runtime,
            ) {
                Ok(session) => app.apply_refresh_result(anchor, session),
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

#[cfg(test)]
mod tests {
    use rebecca::core::disk_map::DiskMapSortField;

    use super::*;
    use crate::tui::model::TuiScreen;

    fn options() -> TuiOptions {
        TuiOptions {
            output_mode: OutputMode::Human,
            roots: Vec::new(),
            scan_backend: None,
            entry_limit: None,
            screen_reader: None,
            no_color: None,
            once: false,
            replay_keys: None,
            terminal_width: 120,
        }
    }

    #[test]
    fn preferences_fill_omitted_tui_options() {
        let preferences = TuiPreferences {
            version: 1,
            last_screen: Some(TuiScreen::Treemap),
            sort: Some(DiskMapSortField::Files),
            entry_limit: Some(500),
            scan_backend: Some(ScanBackendKind::WindowsNative),
            screen_reader: Some(true),
            no_color: Some(true),
        };

        let resolved = options().resolve(&preferences);

        assert_eq!(resolved.scan_backend, ScanBackendKind::WindowsNative);
        assert_eq!(resolved.entry_limit, 500);
        assert_eq!(resolved.sort, DiskMapSortField::Files);
        assert!(resolved.screen_reader);
        assert!(resolved.no_color);
        assert_eq!(resolved.preferred_screen, Some(TuiScreen::Treemap));
    }

    #[test]
    fn explicit_tui_options_override_preferences_without_mutating_them() {
        let mut options = options();
        options.scan_backend = Some(ScanBackendKind::PortableRecursive);
        options.entry_limit = Some(100);
        options.screen_reader = Some(false);
        options.no_color = Some(false);
        let preferences = TuiPreferences {
            version: 1,
            last_screen: Some(TuiScreen::Types),
            sort: Some(DiskMapSortField::Files),
            entry_limit: Some(500),
            scan_backend: Some(ScanBackendKind::WindowsNative),
            screen_reader: Some(true),
            no_color: Some(true),
        };

        let resolved = options.resolve(&preferences);

        assert_eq!(resolved.scan_backend, ScanBackendKind::PortableRecursive);
        assert_eq!(resolved.entry_limit, 100);
        assert!(!resolved.screen_reader);
        assert!(!resolved.no_color);
        assert_eq!(preferences.entry_limit, Some(500));
    }
}

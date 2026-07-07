use std::io::{self, Stdout};
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use crate::tui::app::TuiKey;

pub(crate) type TuiTerminal = Terminal<CrosstermBackend<Stdout>>;

pub(crate) struct TerminalGuard {
    terminal: TuiTerminal,
}

impl TerminalGuard {
    pub(crate) fn enter() -> Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let terminal = Terminal::new(CrosstermBackend::new(stdout))?;
        Ok(Self { terminal })
    }

    pub(crate) fn terminal_mut(&mut self) -> &mut TuiTerminal {
        &mut self.terminal
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(self.terminal.backend_mut(), LeaveAlternateScreen);
        let _ = self.terminal.show_cursor();
    }
}

pub(crate) fn poll_key(timeout: Duration) -> Result<Option<TuiKey>> {
    if !event::poll(timeout)? {
        return Ok(None);
    }

    let Event::Key(key) = event::read()? else {
        return Ok(None);
    };
    if key.kind != KeyEventKind::Press {
        return Ok(None);
    }
    Ok(key_event_to_tui_key(key))
}

fn key_event_to_tui_key(key: KeyEvent) -> Option<TuiKey> {
    if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('c')) {
        return Some(TuiKey::Char('q'));
    }

    match key.code {
        KeyCode::Up => Some(TuiKey::Up),
        KeyCode::Down => Some(TuiKey::Down),
        KeyCode::Left => Some(TuiKey::Left),
        KeyCode::Right => Some(TuiKey::Right),
        KeyCode::Enter => Some(TuiKey::Enter),
        KeyCode::Backspace => Some(TuiKey::Backspace),
        KeyCode::Esc => Some(TuiKey::Esc),
        KeyCode::Tab => Some(TuiKey::Tab),
        KeyCode::Char(' ') => Some(TuiKey::Space),
        KeyCode::Char(ch) => Some(TuiKey::Char(ch)),
        _ => None,
    }
}

pub(crate) fn replay_token_to_key(token: &str) -> Option<TuiKey> {
    match token {
        "up" | "k" => Some(TuiKey::Up),
        "down" | "j" => Some(TuiKey::Down),
        "left" | "h" | "back" => Some(TuiKey::Left),
        "right" | "l" | "open" => Some(TuiKey::Right),
        "enter" => Some(TuiKey::Enter),
        "esc" => Some(TuiKey::Esc),
        "tab" => Some(TuiKey::Tab),
        "space" => Some(TuiKey::Space),
        "backspace" => Some(TuiKey::Backspace),
        token if token.len() == 1 => token.chars().next().map(TuiKey::Char),
        _ => None,
    }
}

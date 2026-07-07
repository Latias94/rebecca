use std::io::{self, Stdout};
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use crate::tui::input::{TuiInput, TuiKey, TuiMouseEvent, TuiMouseEventKind};

pub(crate) type TuiTerminal = Terminal<CrosstermBackend<Stdout>>;

pub(crate) struct TerminalGuard {
    terminal: TuiTerminal,
    _mouse_capture: MouseCaptureGuard,
    _alternate_screen: AlternateScreenGuard,
    _raw_mode: RawModeGuard,
}

impl TerminalGuard {
    pub(crate) fn enter() -> Result<Self> {
        let raw_mode = RawModeGuard::enter()?;
        let mut stdout = io::stdout();
        let alternate_screen = AlternateScreenGuard::enter(&mut stdout)?;
        let mouse_capture = MouseCaptureGuard::enter(&mut stdout)?;
        let terminal = Terminal::new(CrosstermBackend::new(stdout))?;
        Ok(Self {
            terminal,
            _mouse_capture: mouse_capture,
            _alternate_screen: alternate_screen,
            _raw_mode: raw_mode,
        })
    }

    pub(crate) fn terminal_mut(&mut self) -> &mut TuiTerminal {
        &mut self.terminal
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = self.terminal.show_cursor();
    }
}

struct RawModeGuard;

impl RawModeGuard {
    fn enter() -> Result<Self> {
        enable_raw_mode()?;
        Ok(Self)
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
    }
}

struct AlternateScreenGuard;

impl AlternateScreenGuard {
    fn enter(stdout: &mut Stdout) -> Result<Self> {
        execute!(stdout, EnterAlternateScreen)?;
        Ok(Self)
    }
}

impl Drop for AlternateScreenGuard {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
    }
}

struct MouseCaptureGuard;

impl MouseCaptureGuard {
    fn enter(stdout: &mut Stdout) -> Result<Self> {
        execute!(stdout, EnableMouseCapture)?;
        Ok(Self)
    }
}

impl Drop for MouseCaptureGuard {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), DisableMouseCapture);
    }
}

pub(crate) fn poll_input(timeout: Duration) -> Result<Option<TuiInput>> {
    if !event::poll(timeout)? {
        return Ok(None);
    }

    match event::read()? {
        Event::Key(key) if key.kind == KeyEventKind::Press => {
            Ok(key_event_to_tui_key(key).map(TuiInput::Key))
        }
        Event::Mouse(mouse) => Ok(mouse_event_to_tui_mouse_event(mouse).map(TuiInput::Mouse)),
        _ => Ok(None),
    }
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

fn mouse_event_to_tui_mouse_event(mouse: MouseEvent) -> Option<TuiMouseEvent> {
    let kind = match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => TuiMouseEventKind::LeftDown,
        MouseEventKind::ScrollUp => TuiMouseEventKind::ScrollUp,
        MouseEventKind::ScrollDown => TuiMouseEventKind::ScrollDown,
        _ => return None,
    };
    Some(TuiMouseEvent {
        column: mouse.column,
        row: mouse.row,
        kind,
    })
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

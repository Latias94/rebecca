use std::io::IsTerminal;
use std::path::Path;
use std::time::{Duration, Instant};

use indicatif::{ProgressBar, ProgressStyle};

use crate::output::format_bytes;

pub(crate) const PROGRESS_PATH_MAX_CHARS: usize = 72;

pub(crate) fn stderr_spinner(enabled: bool, initial_message: &'static str) -> Option<ProgressBar> {
    (enabled && std::io::stderr().is_terminal()).then(|| {
        let bar = ProgressBar::new_spinner();
        apply_progress_style(&bar);
        bar.enable_steady_tick(Duration::from_millis(120));
        bar.set_message(initial_message);
        bar
    })
}

pub(crate) fn apply_progress_style(bar: &ProgressBar) {
    if let Ok(style) = ProgressStyle::with_template("{spinner} {msg}") {
        bar.set_style(style.tick_strings(&["-", "\\", "|", "/"]));
    }
}

pub(crate) fn compact_progress_path(path: &Path, max_chars: usize) -> String {
    compact_progress_text(&path.display().to_string(), max_chars)
}

pub(crate) fn compact_progress_text(text: &str, max_chars: usize) -> String {
    let char_count = text.chars().count();
    if char_count <= max_chars {
        return text.to_string();
    }

    if max_chars <= 3 {
        return ".".repeat(max_chars);
    }

    let tail_len = max_chars - 3;
    let tail = text
        .chars()
        .rev()
        .take(tail_len)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();
    format!("...{tail}")
}

pub(crate) fn format_file_rate(files_scanned: u64, elapsed: Duration) -> String {
    let seconds = elapsed.as_secs_f64();
    if files_scanned == 0 || seconds <= f64::EPSILON {
        return "0.0 files/s".to_string();
    }

    format!("{:.1} files/s", files_scanned as f64 / seconds)
}

pub(crate) fn format_byte_rate(bytes_scanned: u64, elapsed: Duration) -> String {
    let seconds = elapsed.as_secs_f64();
    if bytes_scanned == 0 || seconds <= f64::EPSILON {
        return "0 B/s".to_string();
    }

    let bytes_per_second = (bytes_scanned as f64 / seconds).round() as u64;
    format!("{}/s", format_bytes(bytes_per_second))
}

#[derive(Debug)]
pub(crate) struct HumanProgressThrottle {
    events_since_refresh: u64,
    last_refresh: Instant,
}

impl HumanProgressThrottle {
    const EVENT_INTERVAL: u64 = 64;
    const TIME_INTERVAL: Duration = Duration::from_millis(250);

    pub(crate) fn new() -> Self {
        Self {
            events_since_refresh: 0,
            last_refresh: Instant::now(),
        }
    }

    pub(crate) fn should_refresh(&mut self) -> bool {
        self.events_since_refresh = self.events_since_refresh.saturating_add(1);
        if self.events_since_refresh < Self::EVENT_INTERVAL
            && self.last_refresh.elapsed() < Self::TIME_INTERVAL
        {
            return false;
        }

        self.events_since_refresh = 0;
        self.last_refresh = Instant::now();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_progress_text_handles_tiny_widths() {
        assert_eq!(compact_progress_text("abcdef", 2), "..");
        assert_eq!(compact_progress_text("abcdef", 4), "...f");
    }

    #[test]
    fn rate_formatters_handle_zero_elapsed_inputs() {
        assert_eq!(format_file_rate(0, Duration::ZERO), "0.0 files/s");
        assert_eq!(format_byte_rate(0, Duration::ZERO), "0 B/s");
    }
}

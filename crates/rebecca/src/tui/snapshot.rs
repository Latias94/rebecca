use ratatui::text::Line;
use rebecca::core::cleanup_advice::CleanupAdviceStatus;
use rebecca::core::disk_session::{DiskMapDistributionRow, DiskMapVisibleRow};
use rebecca::core::plan::CleanupPlan;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::output::format_bytes;
use crate::text::format_count;
use crate::tui::app::TuiApp;
use crate::tui::model::TuiScreen;
use crate::tui::navigation::RootChoice;
use crate::tui::progress::TuiTaskStatus;
use crate::tui::view::ViewOptions;

const BAR_WIDTH: usize = 12;

pub(crate) fn snapshot(app: &TuiApp, options: ViewOptions) -> String {
    let mut lines = Vec::new();
    let width = options.width;
    lines.push(trim_to_width(
        format!(
            "Rebecca TUI | {} | basket {} | sort {}{}",
            screen_label(app.screen),
            app.basket.len(),
            app.sort.label(),
            group_filter_suffix(app)
        ),
        width,
    ));
    match app.screen {
        TuiScreen::RootPicker => snapshot_root_picker(app, width, &mut lines),
        TuiScreen::Map => snapshot_map(app, options, &mut lines),
        TuiScreen::Treemap => snapshot_treemap(app, &mut lines, width),
        TuiScreen::Types | TuiScreen::Extensions => snapshot_distribution(app, options, &mut lines),
        TuiScreen::Busy => {
            lines.push(trim_to_width(format!("Busy: {}", app.message), width));
            for line in task_status_plain_lines(app.task_status.as_ref()) {
                lines.push(trim_to_width(line, width));
            }
        }
        TuiScreen::Preview => {
            snapshot_plan("Cleanup preview", app.preview.as_ref(), width, &mut lines)
        }
        TuiScreen::Confirm => {
            lines.push(trim_to_width(
                format!("Type {} to execute", app.confirmation_phrase()),
                width,
            ));
            lines.push(trim_to_width(format!("Input: {}", app.message), width));
        }
        TuiScreen::Executed => {
            snapshot_plan("Cleanup result", app.executed.as_ref(), width, &mut lines)
        }
        TuiScreen::History => snapshot_history(app, width, &mut lines),
        TuiScreen::Help => snapshot_help(width, &mut lines),
        TuiScreen::Error => lines.push(trim_to_width(format!("Error: {}", app.message), width)),
    }
    lines.push(trim_to_width(format!("Status: {}", app.message), width));
    lines.join("\n")
}

fn snapshot_root_picker(app: &TuiApp, width: usize, lines: &mut Vec<String>) {
    lines.push(trim_to_width("Roots".to_string(), width));
    for (index, choice) in app.root_choices.iter().enumerate() {
        lines.push(snapshot_root_choice(index, choice, app.selected, width));
    }
}

fn snapshot_root_choice(
    index: usize,
    choice: &RootChoice,
    selected: usize,
    width: usize,
) -> String {
    trim_to_width(
        format!(
            "{} {} {}",
            if index == selected { ">" } else { " " },
            choice.label,
            choice.path.display()
        ),
        width,
    )
}

fn snapshot_map(app: &TuiApp, options: ViewOptions, lines: &mut Vec<String>) {
    let width = options.width;
    let rows = app.visible_rows();
    lines.push(trim_to_width(map_title(app, "Map"), width));
    for (index, row) in rows.iter().enumerate().take(20) {
        let staged = row
            .cleanup_advice
            .as_ref()
            .and_then(|advice| advice.rule_id.as_ref())
            .is_some_and(|rule_id| app.basket.contains_key(rule_id));
        lines.push(trim_to_width(
            format!(
                "{}{} {:>10}{} {} {}",
                if index == app.selected { ">" } else { " " },
                if staged { "*" } else { " " },
                format_bytes(row.metrics.logical_bytes),
                if options.visual_bars {
                    format!(
                        " {}",
                        byte_bar(row.metrics.logical_bytes, max_logical(&rows), BAR_WIDTH)
                    )
                } else {
                    String::new()
                },
                row.name,
                advice_label(row)
            ),
            width,
        ));
    }
}

fn snapshot_treemap(app: &TuiApp, lines: &mut Vec<String>, width: usize) {
    let rows = app.visible_rows();
    let total = rows
        .iter()
        .map(|row| row.metrics.logical_bytes)
        .sum::<u64>()
        .max(1);
    lines.push(trim_to_width(map_title(app, "Treemap"), width));
    for line in treemap_context_lines(app) {
        lines.push(trim_to_width(line, width));
    }
    if rows.is_empty() {
        lines.push(trim_to_width(treemap_empty_message(app), width));
        return;
    }
    for (index, row) in rows.iter().enumerate().take(20) {
        lines.push(trim_to_width(
            format!(
                "{} {:>10} {:>6.1}% {} {}",
                if index == app.selected { ">" } else { " " },
                format_bytes(row.metrics.logical_bytes),
                (row.metrics.logical_bytes as f64 / total as f64) * 100.0,
                row.name,
                advice_label(row)
            ),
            width,
        ));
    }
}

fn snapshot_distribution(app: &TuiApp, options: ViewOptions, lines: &mut Vec<String>) {
    let width = options.width;
    let rows = app.distribution_rows();
    let max = max_distribution_logical(&rows);
    let selected = app.selected_distribution_index();
    lines.push(trim_to_width(
        distribution_title(app.screen).to_string(),
        width,
    ));
    if rows.is_empty() {
        lines.push(trim_to_width(
            distribution_empty_label(app.screen).to_string(),
            width,
        ));
        return;
    }
    for (index, row) in rows.iter().enumerate().take(20) {
        lines.push(trim_to_width(
            format!(
                "{} {:>10} {:>7} {:>14}{} {}",
                if index == selected { ">" } else { " " },
                format_bytes(row.metrics.logical_bytes),
                distribution_share_label(row),
                distribution_count_label(row),
                if options.visual_bars {
                    format!(" {}", byte_bar(row.metrics.logical_bytes, max, BAR_WIDTH))
                } else {
                    String::new()
                },
                row.label,
            ),
            width,
        ));
    }
}

fn snapshot_plan(
    title: &'static str,
    plan: Option<&CleanupPlan>,
    width: usize,
    lines: &mut Vec<String>,
) {
    lines.push(trim_to_width(title.to_string(), width));
    for line in plan_lines(plan) {
        lines.push(trim_to_width(line_to_plain(&line), width));
    }
}

fn snapshot_help(width: usize, lines: &mut Vec<String>) {
    for line in help_lines() {
        lines.push(trim_to_width(line_to_plain(&line), width));
    }
}

fn snapshot_history(app: &TuiApp, width: usize, lines: &mut Vec<String>) {
    for line in history_lines(app) {
        lines.push(trim_to_width(line_to_plain(&line), width));
    }
}

fn plan_lines(plan: Option<&CleanupPlan>) -> Vec<Line<'static>> {
    let Some(plan) = plan else {
        return vec![Line::from("No plan available.")];
    };
    vec![
        Line::from(format!(
            "Targets: {} total, {} allowed, {} blocked, {} skipped, {} failed",
            plan.summary.total_targets,
            plan.summary.allowed_targets,
            plan.summary.blocked_targets,
            plan.summary.skipped_targets,
            plan.summary.failed_targets
        )),
        Line::from(format!(
            "Estimated: {} ({})",
            plan.summary.estimated_bytes,
            format_bytes(plan.summary.estimated_bytes)
        )),
        Line::from(format!(
            "Freed: {} ({})",
            plan.summary.freed_bytes,
            format_bytes(plan.summary.freed_bytes)
        )),
        Line::from("e execute  Esc return  q quit"),
    ]
}

fn help_lines() -> Vec<Line<'static>> {
    vec![
        Line::from("j/k or arrows move"),
        Line::from("Enter/l opens a directory or treemap tile, h/Esc moves back"),
        Line::from("1 map, 4/w treemap, 2/t types, 3/x extensions, Tab cycles views"),
        Line::from("Enter filters by selected type/extension; Backspace clears group filter"),
        Line::from("/ filters the active view, s cycles sort"),
        Line::from("r refreshes the active directory, R refreshes the root, b restores a scan"),
        Line::from(
            "Mouse: click tabs, map rows, treemap tiles, or distribution rows; click selects only",
        ),
        Line::from("Space stages the cleanup rule; preview includes all matching targets"),
        Line::from("e executes only after typed confirmation"),
        Line::from("g shows recent cleanup history"),
        Line::from("q quits"),
    ]
}

fn history_lines(app: &TuiApp) -> Vec<Line<'static>> {
    if app.history.is_empty() {
        return vec![Line::from("No cleanup history entries yet.")];
    }

    let mut lines = Vec::with_capacity(app.history.len() + 2);
    lines.push(Line::from("Recent cleanup history"));
    for (index, entry) in app.history.iter().rev().enumerate() {
        let bytes = entry
            .summary
            .freed_bytes
            .saturating_add(entry.summary.pending_reclaim_bytes);
        lines.push(Line::from(format!(
            "#{:>2} {} | targets {} done, {} blocked, {} skipped | {}",
            index + 1,
            entry.recorded_at_unix_seconds,
            entry.summary.completed_targets,
            entry.summary.blocked_targets,
            entry.summary.skipped_targets,
            format_bytes(bytes),
        )));
    }
    lines.push(Line::from("Esc returns to map, q quits."));
    lines
}

fn task_status_plain_lines(status: Option<&TuiTaskStatus>) -> Vec<String> {
    let Some(status) = status else {
        return Vec::new();
    };

    let mut lines = Vec::new();
    lines.push(format!("Task: {} | {}", status.label, status.phase));
    if status.cancel_requested {
        lines.push("Cancel requested; waiting for cooperative checkpoint.".to_string());
    }
    if let Some(backend) = &status.backend {
        lines.push(format!("Backend: {backend}"));
    }
    if status.root_count > 0 {
        lines.push(format!(
            "Roots: {}/{}",
            status.roots_finished, status.root_count
        ));
    }
    if status.files > 0 || status.directories > 0 || status.logical_bytes > 0 {
        lines.push(format!(
            "Scanned: {} files, {} directories, {}",
            status.files,
            status.directories,
            format_bytes(status.logical_bytes)
        ));
    }
    if status.targets_started > 0 || status.targets_finished > 0 {
        lines.push(format!(
            "Targets: {} started, {} finished, {} estimated",
            status.targets_started,
            status.targets_finished,
            format_bytes(status.estimated_bytes)
        ));
    }
    if status.cache_hits > 0 || status.cache_misses > 0 || status.cache_write_skipped > 0 {
        lines.push(format!(
            "Scan cache: {} hits, {} misses, {} skipped writes, {} pruned",
            status.cache_hits, status.cache_misses, status.cache_write_skipped, status.cache_pruned
        ));
    }
    if let Some(rule_id) = &status.current_rule_id {
        lines.push(format!("Rule: {rule_id}"));
    }
    if let Some(path) = &status.current_path {
        lines.push(format!("Current: {}", path.display()));
    }
    if !status.last_event.is_empty() {
        lines.push(format!("Last: {}", status.last_event));
    }
    lines
}

fn group_filter_suffix(app: &TuiApp) -> String {
    app.active_group_filter_label()
        .map(|label| format!(" | filter {label}"))
        .unwrap_or_default()
}

fn map_title(app: &TuiApp, prefix: &str) -> String {
    match app.active_group_filter_label() {
        Some(label) => format!("{prefix}: {} [{label}]", app.current_node_name()),
        None => format!("{prefix}: {}", app.current_node_name()),
    }
}

fn treemap_context_lines(app: &TuiApp) -> Vec<String> {
    let mut lines = Vec::new();
    lines.push(format!("Scope: {}", app.current_node_name()));
    lines.push(format!("Breadcrumb: {}", app.current_scope_breadcrumb()));
    lines.push(format!("Zoom depth: {}", app.zoom_depth()));
    lines.push(format!(
        "Filter: {}",
        app.active_scope_filter_summary()
            .unwrap_or_else(|| "none".to_string())
    ));
    if let Some(summary) = app.treemap_selection_summary() {
        lines.push(format!("Selected tile: {}", summary.name));
        lines.push(format!("Kind: {}", summary.kind));
        lines.push(format!(
            "Drillable: {}",
            if summary.drillable { "yes" } else { "no" }
        ));
        if let Some(reason) = summary.non_drillable_reason {
            lines.push(format!("Reason: {reason}"));
        }
        lines.push(format!("Action: {}", summary.primary_action));
    } else {
        lines.push("Selected tile: none".to_string());
        lines.push("Drillable: no".to_string());
        lines.push("Action: select a directory tile".to_string());
    }
    lines
}

fn treemap_empty_message(app: &TuiApp) -> String {
    if app.active_scope_filter_summary().is_some() {
        "No entries match the active filters in this scope.".to_string()
    } else {
        "No entries for this scope.".to_string()
    }
}

fn screen_label(screen: TuiScreen) -> &'static str {
    match screen {
        TuiScreen::RootPicker => "root-picker",
        TuiScreen::Map => "map",
        TuiScreen::Treemap => "treemap",
        TuiScreen::Types => "types",
        TuiScreen::Extensions => "extensions",
        TuiScreen::Busy => "working",
        TuiScreen::Preview => "preview",
        TuiScreen::Confirm => "confirm",
        TuiScreen::Executed => "executed",
        TuiScreen::History => "history",
        TuiScreen::Help => "help",
        TuiScreen::Error => "error",
    }
}

fn advice_label(row: &DiskMapVisibleRow) -> String {
    row.cleanup_advice
        .as_ref()
        .map(|advice| advice.status.label().to_string())
        .unwrap_or_else(|| CleanupAdviceStatus::Unknown.label().to_string())
}

fn max_logical(rows: &[DiskMapVisibleRow]) -> u64 {
    rows.iter()
        .map(|row| row.metrics.logical_bytes)
        .max()
        .unwrap_or(0)
}

fn max_distribution_logical(rows: &[DiskMapDistributionRow]) -> u64 {
    rows.iter()
        .map(|row| row.metrics.logical_bytes)
        .max()
        .unwrap_or(0)
}

fn distribution_title(screen: TuiScreen) -> &'static str {
    match screen {
        TuiScreen::Types => "Types: file kind distribution",
        TuiScreen::Extensions => "Extensions: suffix distribution",
        _ => "Distribution",
    }
}

fn distribution_empty_label(screen: TuiScreen) -> &'static str {
    match screen {
        TuiScreen::Types => "No type distribution for this scan.",
        TuiScreen::Extensions => "No extension distribution for this scan.",
        _ => "No distribution rows for this scan.",
    }
}

fn distribution_count_label(row: &DiskMapDistributionRow) -> String {
    match (row.metrics.files, row.metrics.directories) {
        (files, 0) => format_count(files, "file", "files"),
        (0, directories) => format_count(directories, "directory", "directories"),
        (files, directories) => format!(
            "{}, {}",
            format_count(files, "file", "files"),
            format_count(directories, "directory", "directories")
        ),
    }
}

fn distribution_share_label(row: &DiskMapDistributionRow) -> String {
    if row.scope_logical_bytes == 0 {
        return "0.0%".to_string();
    }
    format!(
        "{:.1}%",
        (row.metrics.logical_bytes as f64 / row.scope_logical_bytes as f64) * 100.0
    )
}

fn byte_bar(value: u64, max: u64, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let filled = if max == 0 {
        0
    } else {
        ((value as f64 / max as f64) * width as f64).round() as usize
    }
    .min(width);
    format!("{}{}", "#".repeat(filled), ".".repeat(width - filled))
}

fn trim_to_width(value: String, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    if UnicodeWidthStr::width(value.as_str()) <= width {
        return value;
    }

    let suffix = if width > 3 { "..." } else { "" };
    let target_width = width.saturating_sub(UnicodeWidthStr::width(suffix));
    let mut rendered_width = 0;
    let mut trimmed = String::new();
    for ch in value.chars() {
        let char_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if rendered_width + char_width > target_width {
            break;
        }
        trimmed.push(ch);
        rendered_width += char_width;
    }
    trimmed.push_str(suffix);
    trimmed
}

fn line_to_plain(line: &Line<'_>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>()
}

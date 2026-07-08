use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table, Wrap};
use rebecca::core::cleanup_advice::CleanupAdviceStatus;
use rebecca::core::disk_session::{DiskMapDistributionRow, DiskMapVisibleRow};
use rebecca::core::plan::CleanupPlan;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::output::format_bytes;
use crate::text::format_count;
use crate::tui::app::TuiApp;
use crate::tui::basket;
use crate::tui::layout;
use crate::tui::model::TuiScreen;
use crate::tui::progress::TuiTaskStatus;
use crate::tui::treemap::TreemapTile;

const BAR_WIDTH: usize = 12;

#[derive(Debug, Clone, Copy)]
pub(crate) struct ViewOptions {
    pub(crate) width: usize,
    pub(crate) visual_bars: bool,
    pub(crate) color: bool,
}

pub(crate) fn render(frame: &mut Frame<'_>, app: &TuiApp, options: ViewOptions) {
    let layout = layout::frame(frame.area());

    render_header(frame, app, layout.header, options.color);
    match app.screen {
        TuiScreen::RootPicker => render_root_picker(frame, app, layout.body, options.color),
        TuiScreen::Map => render_map(frame, app, layout.body, options),
        TuiScreen::Treemap => render_treemap(frame, app, layout.body, options),
        TuiScreen::Types | TuiScreen::Extensions => {
            render_distribution(frame, app, layout.body, options);
        }
        TuiScreen::Busy => render_busy(frame, app, layout.body),
        TuiScreen::Preview => {
            render_plan(frame, app.preview.as_ref(), "Cleanup preview", layout.body)
        }
        TuiScreen::Confirm => render_confirm(frame, app, layout.body),
        TuiScreen::Executed => {
            render_plan(frame, app.executed.as_ref(), "Cleanup result", layout.body)
        }
        TuiScreen::History => render_history(frame, app, layout.body),
        TuiScreen::Help => render_help(frame, layout.body),
        TuiScreen::Error => render_error(frame, app, layout.body),
    }
    render_status(frame, app, layout.status);
}

fn render_header(frame: &mut Frame<'_>, app: &TuiApp, area: Rect, color: bool) {
    let mut spans = vec![Span::styled("Rebecca ", header_style(color))];
    for (label, screen) in layout::header_tab_specs() {
        spans.push(Span::styled(
            format!("[{label}]"),
            selected_style(app.screen == screen, color),
        ));
        spans.push(Span::raw(" "));
    }
    spans.push(Span::raw(format!(
        " basket:{}  sort:{}{}",
        app.basket.len(),
        app.sort.label(),
        group_filter_suffix(app)
    )));
    let title = Line::from(spans);
    frame.render_widget(Paragraph::new(title), area);
}

fn render_root_picker(frame: &mut Frame<'_>, app: &TuiApp, area: Rect, color: bool) {
    let rows = app.root_choices.iter().enumerate().map(|(index, choice)| {
        let marker = if index == app.selected { ">" } else { " " };
        Row::new(vec![
            Cell::from(marker),
            Cell::from(choice.label.clone()),
            Cell::from(choice.path.display().to_string()),
        ])
        .style(selected_style(index == app.selected, color))
    });
    let table = Table::new(
        rows,
        [
            Constraint::Length(2),
            Constraint::Length(18),
            Constraint::Min(20),
        ],
    )
    .block(Block::default().borders(Borders::ALL).title("Roots"))
    .column_spacing(1);
    frame.render_widget(table, area);
}

fn render_map(frame: &mut Frame<'_>, app: &TuiApp, area: Rect, options: ViewOptions) {
    let chunks = layout::workbench_body(area);

    let rows = app.visible_rows();
    let table_rows = rows.iter().enumerate().map(|(index, row)| {
        let marker = if index == app.selected { ">" } else { " " };
        let staged = row
            .cleanup_advice
            .as_ref()
            .and_then(|advice| advice.rule_id.as_ref())
            .is_some_and(|rule_id| app.basket.contains_key(rule_id));
        Row::new(vec![
            Cell::from(marker),
            Cell::from(if staged { "*" } else { " " }),
            Cell::from(if row.has_children {
                "dir"
            } else {
                row.kind.label()
            }),
            Cell::from(format_bytes(row.metrics.logical_bytes)),
            Cell::from(if options.visual_bars {
                byte_bar(row.metrics.logical_bytes, max_logical(&rows), BAR_WIDTH)
            } else {
                String::new()
            }),
            Cell::from(row.name.clone()),
            Cell::from(advice_label(row)),
        ])
        .style(selected_style(index == app.selected, options.color))
    });
    let table = Table::new(
        table_rows,
        [
            Constraint::Length(2),
            Constraint::Length(1),
            Constraint::Length(5),
            Constraint::Length(11),
            Constraint::Length(BAR_WIDTH as u16),
            Constraint::Min(16),
            Constraint::Length(18),
        ],
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(map_title(app, "Map")),
    )
    .column_spacing(1);
    frame.render_widget(table, chunks.primary);
    render_details(frame, app, chunks.details);
}

fn render_treemap(frame: &mut Frame<'_>, app: &TuiApp, area: Rect, options: ViewOptions) {
    let chunks = layout::workbench_body(area);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(map_title(app, "Treemap"));
    let tile_area = layout::bordered_inner(chunks.primary);
    frame.render_widget(block, chunks.primary);

    let rows = app.visible_rows();
    let tiles = layout::treemap_tiles(&rows, tile_area);
    if tiles.is_empty() {
        frame.render_widget(Paragraph::new(treemap_empty_message(app)), tile_area);
    } else {
        for (index, tile) in tiles.iter().enumerate() {
            render_treemap_tile(
                frame,
                tile,
                index,
                tile.row_index == Some(app.selected),
                options,
            );
        }
    }

    render_details(frame, app, chunks.details);
}

fn render_distribution(frame: &mut Frame<'_>, app: &TuiApp, area: Rect, options: ViewOptions) {
    let chunks = layout::workbench_body(area);
    let rows = app.distribution_rows();
    let max = max_distribution_logical(&rows);
    let selected = app.selected_distribution_index();
    let table_rows = rows.iter().enumerate().map(|(index, row)| {
        let marker = if index == selected { ">" } else { " " };
        Row::new(vec![
            Cell::from(marker),
            Cell::from(row.label.clone()),
            Cell::from(format_bytes(row.metrics.logical_bytes)),
            Cell::from(
                row.metrics
                    .allocated_bytes
                    .map(format_bytes)
                    .unwrap_or_else(|| "-".to_string()),
            ),
            Cell::from(distribution_count_label(row)),
            Cell::from(distribution_share_label(row)),
            Cell::from(if options.visual_bars {
                byte_bar(row.metrics.logical_bytes, max, BAR_WIDTH)
            } else {
                String::new()
            }),
        ])
        .style(selected_style(index == selected, options.color))
    });
    let table = Table::new(
        table_rows,
        [
            Constraint::Length(2),
            Constraint::Min(14),
            Constraint::Length(11),
            Constraint::Length(11),
            Constraint::Length(18),
            Constraint::Length(8),
            Constraint::Length(BAR_WIDTH as u16),
        ],
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(distribution_title(app.screen)),
    )
    .column_spacing(1);
    frame.render_widget(table, chunks.primary);
    render_distribution_details(frame, rows.get(selected), chunks.details);
}

fn render_treemap_tile(
    frame: &mut Frame<'_>,
    tile: &TreemapTile,
    index: usize,
    selected: bool,
    options: ViewOptions,
) {
    if tile.rect.width == 0 || tile.rect.height == 0 {
        return;
    }
    let style = treemap_tile_style(index, selected, options.color);
    let label_width = usize::from(tile.rect.width.saturating_sub(2)).max(1);
    let mut lines = Vec::new();
    if tile.rect.width >= 6 {
        lines.push(Line::from(trim_to_width(tile.label.clone(), label_width)));
    }
    if tile.rect.height >= 2 && tile.rect.width >= 8 {
        lines.push(Line::from(trim_to_width(
            format_bytes(tile.logical_bytes),
            label_width,
        )));
    }
    let paragraph = Paragraph::new(lines).style(style);
    if tile.rect.width >= 8 && tile.rect.height >= 3 {
        frame.render_widget(
            paragraph.block(Block::default().borders(Borders::ALL)),
            tile.rect,
        );
    } else {
        frame.render_widget(paragraph, tile.rect);
    }
}

fn render_details(frame: &mut Frame<'_>, app: &TuiApp, area: Rect) {
    let mut lines = Vec::new();
    if app.screen == TuiScreen::Treemap {
        lines.extend(treemap_context_lines(app));
        lines.push(Line::from(""));
    }
    if let Some(row) = app.selected_row() {
        lines.push(Line::from(row.path.display().to_string()));
        lines.push(Line::from(format!(
            "{} logical, {}",
            format_bytes(row.metrics.logical_bytes),
            format_count(row.metrics.files, "file", "files")
        )));
        if let Some(advice) = row.cleanup_advice {
            lines.push(Line::from(format!("Advice: {}", advice.status.label())));
            lines.push(Line::from(format!("Reason: {}", advice.reason)));
            if let Some(rule_id) = advice.rule_id {
                lines.push(Line::from(format!("Rule: {rule_id}")));
            }
            if !advice.required_flags.is_empty() {
                lines.push(Line::from(format!(
                    "Flags: {}",
                    advice.required_flags.join(", ")
                )));
            }
            if !advice.required_warnings.is_empty() {
                lines.push(Line::from(format!(
                    "Warnings: {}",
                    advice.required_warnings.join(", ")
                )));
            }
        } else {
            lines.push(Line::from("Advice: none"));
        }
    } else {
        lines.push(Line::from("No entries"));
    }

    lines.push(Line::from(""));
    lines.push(Line::from("Rule basket"));
    if app.basket.is_empty() {
        lines.push(Line::from("  empty"));
    } else {
        lines.push(Line::from("  preview includes all matching rule targets"));
        for item in app.basket.values() {
            lines.push(Line::from(format!("  {}", basket::label(item))));
        }
    }

    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: true })
            .block(Block::default().borders(Borders::ALL).title("Details")),
        area,
    );
}

fn render_distribution_details(
    frame: &mut Frame<'_>,
    selected_row: Option<&DiskMapDistributionRow>,
    area: Rect,
) {
    let mut lines = Vec::new();
    if let Some(row) = selected_row {
        lines.push(Line::from(row.label.clone()));
        lines.push(Line::from(format!("Group: {}", row.kind.label())));
        lines.push(Line::from(format!(
            "Logical: {}",
            format_bytes(row.metrics.logical_bytes)
        )));
        lines.push(Line::from(format!(
            "Allocated: {}",
            row.metrics
                .allocated_bytes
                .map(format_bytes)
                .unwrap_or_else(|| "unknown".to_string())
        )));
        lines.push(Line::from(format!(
            "Unique: {}",
            row.metrics
                .unique_logical_bytes
                .map(format_bytes)
                .unwrap_or_else(|| "unknown".to_string())
        )));
        lines.push(Line::from(format!(
            "Count: {}",
            distribution_count_label(row)
        )));
        lines.push(Line::from(format!(
            "Share: {}",
            distribution_share_label(row)
        )));
        lines.push(Line::from(""));
        lines.push(Line::from("Distribution rows are read-only."));
        lines.push(Line::from("Stage cleanup from map entries."));
    } else {
        lines.push(Line::from("No distribution row selected."));
    }

    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: true }).block(
            Block::default()
                .borders(Borders::ALL)
                .title("Distribution details"),
        ),
        area,
    );
}

fn render_plan(frame: &mut Frame<'_>, plan: Option<&CleanupPlan>, title: &'static str, area: Rect) {
    let lines = plan_lines(title, plan);
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: true })
            .block(Block::default().borders(Borders::ALL).title(title)),
        area,
    );
}

fn render_busy(frame: &mut Frame<'_>, app: &TuiApp, area: Rect) {
    let mut lines = vec![Line::from(app.message.clone())];
    lines.extend(task_status_lines(app.task_status.as_ref()));
    lines.push(Line::from(
        "Esc cancels the current task when possible. q quits.",
    ));
    frame.render_widget(
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title("Working")),
        area,
    );
}

fn render_confirm(frame: &mut Frame<'_>, app: &TuiApp, area: Rect) {
    let lines = vec![
        Line::from("Rebecca will move allowed targets to the system trash."),
        Line::from(format!("Required phrase: {}", app.confirmation_phrase())),
        Line::from(format!("Input: {}", app.message)),
    ];
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .title("Confirm cleanup"),
        ),
        area,
    );
}

fn render_history(frame: &mut Frame<'_>, app: &TuiApp, area: Rect) {
    let lines = history_lines(app);
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: true })
            .block(Block::default().borders(Borders::ALL).title("History")),
        area,
    );
}

fn render_help(frame: &mut Frame<'_>, area: Rect) {
    frame.render_widget(
        Paragraph::new(help_lines()).block(Block::default().borders(Borders::ALL).title("Help")),
        area,
    );
}

fn render_error(frame: &mut Frame<'_>, app: &TuiApp, area: Rect) {
    frame.render_widget(
        Paragraph::new(app.message.clone())
            .block(Block::default().borders(Borders::ALL).title("Error")),
        area,
    );
}

fn render_status(frame: &mut Frame<'_>, app: &TuiApp, area: Rect) {
    let filter_text = app.active_filter_text();
    let filter_label = app.active_filter_label();
    let search = if app.is_search_editing() {
        format!(" | {filter_label} search: {filter_text}")
    } else if filter_text.is_empty() {
        String::new()
    } else {
        format!(" | {filter_label} filter: {filter_text}")
    };
    frame.render_widget(
        Paragraph::new(format!(
            "{}{}{} | Enter open | Space stage | c preview | ? all keys | q quit",
            app.message,
            search,
            active_group_filter_status(app)
        )),
        area,
    );
}

fn plan_lines(_title: &'static str, plan: Option<&CleanupPlan>) -> Vec<Line<'static>> {
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
        Line::from(format!(
            "Pending reclaim: {} ({})",
            plan.summary.pending_reclaim_bytes,
            format_bytes(plan.summary.pending_reclaim_bytes)
        )),
        Line::from("Normal execution moves targets to system trash or Recycle Bin."),
        Line::from("Free pending space after execution: rebecca trash empty --yes"),
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
        Line::from("r patches the active directory, R refreshes the root"),
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

fn task_status_lines(status: Option<&TuiTaskStatus>) -> Vec<Line<'static>> {
    task_status_plain_lines(status)
        .into_iter()
        .map(Line::from)
        .collect()
}

fn task_status_plain_lines(status: Option<&TuiTaskStatus>) -> Vec<String> {
    let Some(status) = status else {
        return Vec::new();
    };

    let mut lines = Vec::new();
    lines.push(format!("Task: {} | {}", status.label, status.phase));
    if status.cancel_requested {
        lines.push(status.cancel_wait_message().to_string());
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

fn active_group_filter_status(app: &TuiApp) -> String {
    app.active_group_filter_label()
        .map(|label| format!(" | group filter: {label}"))
        .unwrap_or_default()
}

fn map_title(app: &TuiApp, prefix: &str) -> String {
    match app.active_group_filter_label() {
        Some(label) => format!("{prefix}: {} [{label}]", app.current_node_name()),
        None => format!("{prefix}: {}", app.current_node_name()),
    }
}

fn treemap_context_lines(app: &TuiApp) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    lines.push(Line::from(format!("Scope: {}", app.current_node_name())));
    lines.push(Line::from(format!(
        "Breadcrumb: {}",
        app.current_scope_breadcrumb()
    )));
    lines.push(Line::from(format!("Zoom depth: {}", app.zoom_depth())));
    lines.push(Line::from(format!(
        "Filter: {}",
        app.active_scope_filter_summary()
            .unwrap_or_else(|| "none".to_string())
    )));
    if let Some(summary) = app.treemap_selection_summary() {
        lines.push(Line::from(format!("Selected tile: {}", summary.name)));
        lines.push(Line::from(format!("Kind: {}", summary.kind)));
        lines.push(Line::from(format!(
            "Drillable: {}",
            if summary.drillable { "yes" } else { "no" }
        )));
        if let Some(reason) = summary.non_drillable_reason {
            lines.push(Line::from(format!("Reason: {reason}")));
        }
        lines.push(Line::from(format!("Action: {}", summary.primary_action)));
    } else {
        lines.push(Line::from("Selected tile: none"));
        lines.push(Line::from("Drillable: no"));
        lines.push(Line::from("Action: select a directory tile"));
    }
    lines
}

fn treemap_empty_message(app: &TuiApp) -> String {
    if app.active_scope_filter_summary().is_some() {
        "No entries match the active filters in this scope.".to_string()
    } else {
        "No non-empty entries.".to_string()
    }
}

fn header_style(color: bool) -> Style {
    if color {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().add_modifier(Modifier::BOLD)
    }
}

fn selected_style(selected: bool, color: bool) -> Style {
    if selected {
        let style = Style::default().add_modifier(Modifier::BOLD);
        if color {
            style.fg(Color::Black).bg(Color::Cyan)
        } else {
            style
        }
    } else {
        Style::default()
    }
}

fn treemap_tile_style(index: usize, selected: bool, color: bool) -> Style {
    if selected {
        return selected_style(true, color);
    }
    if !color {
        return Style::default();
    }
    match index % 4 {
        0 => Style::default().fg(Color::White).bg(Color::DarkGray),
        1 => Style::default().fg(Color::White).bg(Color::Blue),
        2 => Style::default().fg(Color::Black).bg(Color::Green),
        _ => Style::default().fg(Color::Black).bg(Color::Yellow),
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

#[cfg(test)]
mod tests {
    use unicode_width::UnicodeWidthStr;

    use super::*;

    #[test]
    fn trim_to_width_respects_display_width_for_cjk_text() {
        let line = trim_to_width("缓存目录-with-a-very-long-name".to_string(), 12);

        assert!(UnicodeWidthStr::width(line.as_str()) <= 12, "{line}");
        assert!(line.ends_with("..."));
    }
}

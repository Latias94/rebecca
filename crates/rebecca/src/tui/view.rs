use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
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
use crate::tui::input::{TuiMouseAction, TuiMouseEvent, TuiMouseEventKind};
use crate::tui::model::TuiScreen;
use crate::tui::navigation::RootChoice;
use crate::tui::progress::TuiTaskStatus;
use crate::tui::treemap::{self, TreemapItem, TreemapTile};

const BAR_WIDTH: usize = 12;
const TREEMAP_TILE_LIMIT: usize = 24;

#[derive(Debug, Clone, Copy)]
pub(crate) struct ViewOptions {
    pub(crate) width: usize,
    pub(crate) visual_bars: bool,
    pub(crate) color: bool,
}

pub(crate) fn render(frame: &mut Frame<'_>, app: &TuiApp, options: ViewOptions) {
    let chunks = screen_chunks(frame.area());

    render_header(frame, app, chunks[0], options.color);
    match app.screen {
        TuiScreen::RootPicker => render_root_picker(frame, app, chunks[1], options.color),
        TuiScreen::Map => render_map(frame, app, chunks[1], options),
        TuiScreen::Treemap => render_treemap(frame, app, chunks[1], options),
        TuiScreen::Types | TuiScreen::Extensions => {
            render_distribution(frame, app, chunks[1], options);
        }
        TuiScreen::Busy => render_busy(frame, app, chunks[1]),
        TuiScreen::Preview => {
            render_plan(frame, app.preview.as_ref(), "Cleanup preview", chunks[1])
        }
        TuiScreen::Confirm => render_confirm(frame, app, chunks[1]),
        TuiScreen::Executed => {
            render_plan(frame, app.executed.as_ref(), "Cleanup result", chunks[1])
        }
        TuiScreen::History => render_history(frame, app, chunks[1]),
        TuiScreen::Help => render_help(frame, chunks[1]),
        TuiScreen::Error => render_error(frame, app, chunks[1]),
    }
    render_status(frame, app, chunks[2]);
}

pub(crate) fn snapshot(app: &TuiApp, options: ViewOptions) -> String {
    let mut lines = Vec::new();
    let width = options.width;
    lines.push(trim_to_width(
        format!(
            "Rebecca TUI | {} | basket {} | sort {}",
            screen_label(app.screen),
            app.basket.len(),
            app.sort.label()
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

pub(crate) fn hit_test(
    app: &TuiApp,
    _options: ViewOptions,
    area: Rect,
    mouse: TuiMouseEvent,
) -> Option<TuiMouseAction> {
    let chunks = screen_chunks(area);
    let point = (mouse.column, mouse.row);
    if matches!(mouse.kind, TuiMouseEventKind::LeftDown) && rect_contains(chunks[0], point) {
        return hit_header_tab(chunks[0], point);
    }

    if !rect_contains(chunks[1], point) {
        return None;
    }

    match mouse.kind {
        TuiMouseEventKind::ScrollUp => Some(TuiMouseAction::ScrollUp),
        TuiMouseEventKind::ScrollDown => Some(TuiMouseAction::ScrollDown),
        TuiMouseEventKind::LeftDown => match app.screen {
            TuiScreen::Map => hit_map_row(app, chunks[1], point),
            TuiScreen::Treemap => hit_treemap_tile(app, chunks[1], point),
            TuiScreen::Types | TuiScreen::Extensions => hit_distribution_row(app, chunks[1], point),
            _ => None,
        },
    }
}

fn render_header(frame: &mut Frame<'_>, app: &TuiApp, area: Rect, color: bool) {
    let mut spans = vec![Span::styled("Rebecca ", header_style(color))];
    for (label, screen) in header_tab_specs() {
        spans.push(Span::styled(
            format!("[{label}]"),
            selected_style(app.screen == screen, color),
        ));
        spans.push(Span::raw(" "));
    }
    spans.push(Span::raw(format!(
        " basket:{}  sort:{}",
        app.basket.len(),
        app.sort.label()
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
    let chunks = map_details_chunks(area);

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
            .title(format!("Map: {}", app.current_node_name())),
    )
    .column_spacing(1);
    frame.render_widget(table, chunks[0]);
    render_details(frame, app, chunks[1]);
}

fn render_treemap(frame: &mut Frame<'_>, app: &TuiApp, area: Rect, options: ViewOptions) {
    let chunks = map_details_chunks(area);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!("Treemap: {}", app.current_node_name()));
    let tile_area = bordered_inner(chunks[0]);
    frame.render_widget(block, chunks[0]);

    let rows = app.visible_rows();
    let tiles = treemap_tiles(&rows, tile_area);
    if tiles.is_empty() {
        frame.render_widget(Paragraph::new("No non-empty entries."), tile_area);
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

    render_details(frame, app, chunks[1]);
}

fn render_distribution(frame: &mut Frame<'_>, app: &TuiApp, area: Rect, options: ViewOptions) {
    let chunks = map_details_chunks(area);
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
    frame.render_widget(table, chunks[0]);
    render_distribution_details(frame, rows.get(selected), chunks[1]);
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
        Line::from("Rebecca will move allowed targets to recoverable trash."),
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
            "{}{} | 1 map 4/w treemap 2/t types 3/x extensions Tab cycle s sort / filter r refresh R root b restore Space stage c preview g history ? help q quit | mouse click/wheel",
            app.message, search
        )),
        area,
    );
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
    lines.push(trim_to_width(
        format!("Map: {}", app.current_node_name()),
        width,
    ));
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
    lines.push(trim_to_width(
        format!("Treemap: {}", app.current_node_name()),
        width,
    ));
    if rows.is_empty() {
        lines.push(trim_to_width(
            "No entries for this scope.".to_string(),
            width,
        ));
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
    for line in plan_lines(title, plan) {
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
        Line::from("e execute  Esc return  q quit"),
    ]
}

fn help_lines() -> Vec<Line<'static>> {
    vec![
        Line::from("j/k or arrows move"),
        Line::from("Enter/l opens a directory, h/Esc moves back"),
        Line::from("1 map, 4/w treemap, 2/t types, 3/x extensions, Tab cycles views"),
        Line::from("/ filters the active view, s cycles sort"),
        Line::from("r refreshes the active directory, R refreshes the root, b restores a scan"),
        Line::from("Mouse: click tabs or rows to select, wheel moves selection"),
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

fn screen_chunks(area: Rect) -> std::rc::Rc<[Rect]> {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(8),
            Constraint::Length(2),
        ])
        .split(area)
}

fn map_details_chunks(area: Rect) -> std::rc::Rc<[Rect]> {
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(68), Constraint::Percentage(32)])
        .split(area)
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

fn header_tab_specs() -> [(&'static str, TuiScreen); 4] {
    [
        ("1 Map", TuiScreen::Map),
        ("4 Treemap", TuiScreen::Treemap),
        ("2 Types", TuiScreen::Types),
        ("3 Ext", TuiScreen::Extensions),
    ]
}

fn header_tab_rects(area: Rect) -> Vec<(Rect, TuiScreen)> {
    let mut x = area.x.saturating_add("Rebecca ".len() as u16);
    let mut rects = Vec::new();
    for (label, screen) in header_tab_specs() {
        let width = (label.len() + 2) as u16;
        rects.push((
            Rect {
                x,
                y: area.y,
                width,
                height: area.height.min(1),
            },
            screen,
        ));
        x = x.saturating_add(width).saturating_add(1);
    }
    rects
}

fn hit_header_tab(area: Rect, point: (u16, u16)) -> Option<TuiMouseAction> {
    header_tab_rects(area)
        .into_iter()
        .find_map(|(rect, screen)| {
            rect_contains(rect, point).then_some(TuiMouseAction::SwitchScreen(screen))
        })
}

fn hit_map_row(app: &TuiApp, area: Rect, point: (u16, u16)) -> Option<TuiMouseAction> {
    let chunks = map_details_chunks(area);
    table_row_at(chunks[0], point, app.visible_rows().len()).map(TuiMouseAction::SelectMapRow)
}

fn hit_distribution_row(app: &TuiApp, area: Rect, point: (u16, u16)) -> Option<TuiMouseAction> {
    let chunks = map_details_chunks(area);
    table_row_at(chunks[0], point, app.distribution_rows().len())
        .map(TuiMouseAction::SelectDistributionRow)
}

fn hit_treemap_tile(app: &TuiApp, area: Rect, point: (u16, u16)) -> Option<TuiMouseAction> {
    let chunks = map_details_chunks(area);
    let tile_area = bordered_inner(chunks[0]);
    if !rect_contains(tile_area, point) {
        return None;
    }
    let rows = app.visible_rows();
    treemap_tiles(&rows, tile_area)
        .into_iter()
        .find_map(|tile| {
            rect_contains(tile.rect, point)
                .then_some(tile.row_index)
                .flatten()
                .map(TuiMouseAction::SelectMapRow)
        })
}

fn table_row_at(area: Rect, point: (u16, u16), len: usize) -> Option<usize> {
    if len == 0 || !rect_contains(area, point) {
        return None;
    }
    let body_y = area.y.saturating_add(1);
    if point.1 < body_y {
        return None;
    }
    let body_height = area.height.saturating_sub(2);
    if body_height == 0 {
        return None;
    }
    let index = usize::from(point.1.saturating_sub(body_y));
    (index < len && index < usize::from(body_height)).then_some(index)
}

fn treemap_tiles(rows: &[DiskMapVisibleRow], area: Rect) -> Vec<TreemapTile> {
    let items = rows
        .iter()
        .enumerate()
        .map(|(index, row)| TreemapItem {
            row_index: Some(index),
            label: row.name.clone(),
            logical_bytes: row.metrics.logical_bytes,
        })
        .collect::<Vec<_>>();
    treemap::layout_treemap(&items, area, TREEMAP_TILE_LIMIT)
}

fn bordered_inner(area: Rect) -> Rect {
    Rect {
        x: area.x.saturating_add(1),
        y: area.y.saturating_add(1),
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    }
}

fn rect_contains(rect: Rect, point: (u16, u16)) -> bool {
    point.0 >= rect.x
        && point.0 < rect.x.saturating_add(rect.width)
        && point.1 >= rect.y
        && point.1 < rect.y.saturating_add(rect.height)
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use unicode_width::UnicodeWidthStr;

    use rebecca::core::disk_map::{
        DiskMapEntry, DiskMapEntryKind, DiskMapGroup, DiskMapGroupKind, DiskMapMetrics,
        DiskMapReport, DiskMapRoot, DiskMapRootStatus,
    };
    use rebecca::core::disk_session::DiskMapSession;
    use rebecca::core::plan::{EstimateProvenance, EstimateSource};
    use rebecca::core::scan::ScanBackendKind;

    use super::*;
    use crate::tui::input::{TuiKey, TuiMouseEvent, TuiMouseEventKind};

    #[test]
    fn trim_to_width_respects_display_width_for_cjk_text() {
        let line = trim_to_width("缓存目录-with-a-very-long-name".to_string(), 12);

        assert!(UnicodeWidthStr::width(line.as_str()) <= 12, "{line}");
        assert!(line.ends_with("..."));
    }

    #[test]
    fn hit_test_header_tab_switches_to_treemap() {
        let app = test_app();

        let action = hit_test(
            &app,
            view_options(),
            Rect::new(0, 0, 100, 30),
            mouse_left(18, 0),
        );

        assert_eq!(
            action,
            Some(TuiMouseAction::SwitchScreen(TuiScreen::Treemap))
        );
    }

    #[test]
    fn hit_test_map_row_selects_visible_row() {
        let app = test_app();

        let action = hit_test(
            &app,
            view_options(),
            Rect::new(0, 0, 100, 30),
            mouse_left(2, 2),
        );

        assert_eq!(action, Some(TuiMouseAction::SelectMapRow(0)));
    }

    #[test]
    fn hit_test_distribution_row_selects_distribution_row() {
        let mut app = test_app();
        app.handle_key(TuiKey::Char('x'));

        let action = hit_test(
            &app,
            view_options(),
            Rect::new(0, 0, 100, 30),
            mouse_left(2, 2),
        );

        assert_eq!(action, Some(TuiMouseAction::SelectDistributionRow(0)));
    }

    #[test]
    fn hit_test_treemap_tile_selects_map_row() {
        let mut app = test_app();
        app.handle_key(TuiKey::Char('4'));

        let action = hit_test(
            &app,
            view_options(),
            Rect::new(0, 0, 100, 30),
            mouse_left(2, 3),
        );

        assert_eq!(action, Some(TuiMouseAction::SelectMapRow(0)));
    }

    #[test]
    fn hit_test_wheel_moves_active_selection() {
        let app = test_app();

        let action = hit_test(
            &app,
            view_options(),
            Rect::new(0, 0, 100, 30),
            TuiMouseEvent {
                column: 2,
                row: 3,
                kind: TuiMouseEventKind::ScrollDown,
            },
        );

        assert_eq!(action, Some(TuiMouseAction::ScrollDown));
    }

    #[test]
    fn table_row_at_ignores_table_borders() {
        let area = Rect::new(0, 1, 40, 5);

        assert_eq!(table_row_at(area, (2, 1), 10), None);
        assert_eq!(table_row_at(area, (2, 2), 10), Some(0));
        assert_eq!(table_row_at(area, (2, 4), 10), Some(2));
        assert_eq!(table_row_at(area, (2, 5), 10), None);
    }

    fn view_options() -> ViewOptions {
        ViewOptions {
            width: 100,
            visual_bars: true,
            color: true,
        }
    }

    fn mouse_left(column: u16, row: u16) -> TuiMouseEvent {
        TuiMouseEvent {
            column,
            row,
            kind: TuiMouseEventKind::LeftDown,
        }
    }

    fn test_app() -> TuiApp {
        TuiApp::from_session(
            DiskMapSession::from_report(test_report()),
            ScanBackendKind::PortableRecursive,
            100,
        )
    }

    fn test_report() -> DiskMapReport {
        let root = PathBuf::from("/tmp");
        DiskMapReport {
            roots: vec![DiskMapRoot {
                path: root.clone(),
                status: DiskMapRootStatus::Scanned,
                metrics: metrics(100, 2, 1),
                estimate_source: EstimateSource::FreshScan,
                estimate_provenance: EstimateProvenance::default(),
                reason: None,
            }],
            totals: metrics(100, 2, 1),
            top_entries: vec![
                DiskMapEntry {
                    path: root.join("cache"),
                    root: root.clone(),
                    kind: DiskMapEntryKind::Directory,
                    depth: 1,
                    logical_bytes: 60,
                    allocated_bytes: None,
                    unique_logical_bytes: Some(60),
                    unique_allocated_bytes: None,
                    files: 1,
                    directories: 1,
                    estimate_source: EstimateSource::FreshScan,
                    estimate_provenance: EstimateProvenance::default(),
                    cleanup_advice: None,
                },
                DiskMapEntry {
                    path: root.join("log.tmp"),
                    root: root.clone(),
                    kind: DiskMapEntryKind::File,
                    depth: 1,
                    logical_bytes: 40,
                    allocated_bytes: None,
                    unique_logical_bytes: Some(40),
                    unique_allocated_bytes: None,
                    files: 1,
                    directories: 0,
                    estimate_source: EstimateSource::FreshScan,
                    estimate_provenance: EstimateProvenance::default(),
                    cleanup_advice: None,
                },
            ],
            groups: vec![DiskMapGroup {
                kind: DiskMapGroupKind::Extension,
                key: ".tmp".to_string(),
                label: ".tmp".to_string(),
                metrics: metrics(40, 1, 0),
            }],
            diagnostic_summary: Default::default(),
            diagnostics: Vec::new(),
        }
    }

    fn metrics(logical_bytes: u64, files: u64, directories: u64) -> DiskMapMetrics {
        DiskMapMetrics {
            logical_bytes,
            allocated_bytes: None,
            unique_logical_bytes: Some(logical_bytes),
            unique_allocated_bytes: None,
            files,
            directories,
        }
    }
}

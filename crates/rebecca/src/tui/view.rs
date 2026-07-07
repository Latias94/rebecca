use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table, Wrap};
use rebecca::core::cleanup_advice::CleanupAdviceStatus;
use rebecca::core::disk_session::DiskMapVisibleRow;
use rebecca::core::plan::CleanupPlan;

use crate::output::format_bytes;
use crate::text::format_count;
use crate::tui::app::{CleanupBasketItem, RootChoice, TuiApp, TuiScreen};

const BAR_WIDTH: usize = 12;

#[derive(Debug, Clone, Copy)]
pub(crate) struct ViewOptions {
    pub(crate) width: usize,
    pub(crate) visual_bars: bool,
    pub(crate) color: bool,
}

pub(crate) fn render(frame: &mut Frame<'_>, app: &TuiApp, options: ViewOptions) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(8),
            Constraint::Length(2),
        ])
        .split(frame.area());

    render_header(frame, app, chunks[0], options.color);
    match app.screen {
        TuiScreen::RootPicker => render_root_picker(frame, app, chunks[1], options.color),
        TuiScreen::Map => render_map(frame, app, chunks[1], options),
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

fn render_header(frame: &mut Frame<'_>, app: &TuiApp, area: Rect, color: bool) {
    let title = Line::from(vec![
        Span::styled("Rebecca", header_style(color)),
        Span::raw(" "),
        Span::styled(screen_label(app.screen), label_style(color)),
        Span::raw(format!(
            "  basket:{}  sort:{}",
            app.basket.len(),
            app.sort.label()
        )),
    ]);
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
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(68), Constraint::Percentage(32)])
        .split(area);

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
    lines.push(Line::from("Basket"));
    if app.basket.is_empty() {
        lines.push(Line::from("  empty"));
    } else {
        for item in app.basket.values() {
            lines.push(Line::from(format!("  {}", basket_label(item))));
        }
    }

    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: true })
            .block(Block::default().borders(Borders::ALL).title("Details")),
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
    let search = if app.is_search_editing() {
        format!(" | search: {}", app.search_query)
    } else if app.search_query.is_empty() {
        String::new()
    } else {
        format!(" | filter: {}", app.search_query)
    };
    frame.render_widget(
        Paragraph::new(format!(
            "{}{} | j/k move Enter open h back Space stage c preview g history ? help q quit",
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
        Line::from("/ searches visible paths, s cycles sort"),
        Line::from("Space stages cleanup advice, c previews the plan"),
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

fn screen_label(screen: TuiScreen) -> &'static str {
    match screen {
        TuiScreen::RootPicker => "root-picker",
        TuiScreen::Map => "map",
        TuiScreen::Preview => "preview",
        TuiScreen::Confirm => "confirm",
        TuiScreen::Executed => "executed",
        TuiScreen::History => "history",
        TuiScreen::Help => "help",
        TuiScreen::Error => "error",
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

fn label_style(color: bool) -> Style {
    if color {
        Style::default().fg(Color::White)
    } else {
        Style::default()
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

fn advice_label(row: &DiskMapVisibleRow) -> String {
    row.cleanup_advice
        .as_ref()
        .map(|advice| advice.status.label().to_string())
        .unwrap_or_else(|| CleanupAdviceStatus::Unknown.label().to_string())
}

fn basket_label(item: &CleanupBasketItem) -> String {
    let mut label = format!("{} [{}]", item.rule_id, item.status.label());
    if !item.required_flags.is_empty() {
        label.push_str(" flags:");
        label.push_str(&item.required_flags.join(","));
    }
    if !item.required_warnings.is_empty() {
        label.push_str(" warnings:");
        label.push_str(&item.required_warnings.join(","));
    }
    label
}

fn max_logical(rows: &[DiskMapVisibleRow]) -> u64 {
    rows.iter()
        .map(|row| row.metrics.logical_bytes)
        .max()
        .unwrap_or(0)
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
    let mut chars = value.chars();
    let trimmed = chars.by_ref().take(width).collect::<String>();
    if chars.next().is_some() && width > 3 {
        format!("{}...", trimmed.chars().take(width - 3).collect::<String>())
    } else {
        trimmed
    }
}

fn line_to_plain(line: &Line<'_>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>()
}

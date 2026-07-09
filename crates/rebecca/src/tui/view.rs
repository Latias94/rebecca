use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table, Wrap};
use rebecca_core::disk_session::DiskMapDistributionRow;
use rebecca_core::plan::CleanupPlan;

use crate::output::format_bytes;
use crate::text::format_count;
use crate::tui::app::TuiApp;
use crate::tui::basket;
use crate::tui::frame_projection::TuiFrameProjection;
use crate::tui::layout;
use crate::tui::model::TuiScreen;
use crate::tui::presentation::{
    BAR_WIDTH, active_group_filter_status, advice_label, byte_bar, distribution_count_label,
    distribution_share_label, distribution_title, group_filter_suffix, help_ratatui_lines,
    history_ratatui_lines, map_title, max_distribution_logical, max_logical, plan_ratatui_lines,
    task_status_ratatui_lines, treemap_context_ratatui_lines, treemap_empty_message, trim_to_width,
};
use crate::tui::treemap::TreemapTile;

#[derive(Debug, Clone, Copy)]
pub(crate) struct ViewOptions {
    pub(crate) width: usize,
    pub(crate) visual_bars: bool,
    pub(crate) color: bool,
}

pub(crate) fn render(frame: &mut Frame<'_>, app: &TuiApp, options: ViewOptions) {
    let layout = layout::frame(frame.area());
    let projection = app.frame_projection();

    render_header(frame, app, layout.header, options.color);
    match app.screen {
        TuiScreen::RootPicker => render_root_picker(frame, app, layout.body, options.color),
        TuiScreen::Map => render_map(frame, app, &projection, layout.body, options),
        TuiScreen::Treemap => render_treemap(frame, app, &projection, layout.body, options),
        TuiScreen::Types | TuiScreen::Extensions => {
            render_distribution(frame, app, &projection, layout.body, options);
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
        " reclaim:{}  sort:{}{}",
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

fn render_map(
    frame: &mut Frame<'_>,
    app: &TuiApp,
    projection: &TuiFrameProjection,
    area: Rect,
    options: ViewOptions,
) {
    let chunks = layout::workbench_body(area);

    let rows = projection.visible_rows();
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
                byte_bar(row.metrics.logical_bytes, max_logical(rows), BAR_WIDTH)
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
    render_details(frame, app, projection, chunks.details);
}

fn render_treemap(
    frame: &mut Frame<'_>,
    app: &TuiApp,
    projection: &TuiFrameProjection,
    area: Rect,
    options: ViewOptions,
) {
    let chunks = layout::workbench_body(area);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(map_title(app, "Treemap"));
    let tile_area = layout::bordered_inner(chunks.primary);
    frame.render_widget(block, chunks.primary);

    let rows = projection.visible_rows();
    let tiles = layout::treemap_tiles(rows, tile_area);
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

    render_details(frame, app, projection, chunks.details);
}

fn render_distribution(
    frame: &mut Frame<'_>,
    app: &TuiApp,
    projection: &TuiFrameProjection,
    area: Rect,
    options: ViewOptions,
) {
    let chunks = layout::workbench_body(area);
    let rows = projection.distribution_rows();
    let max = max_distribution_logical(rows);
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
    render_distribution_details(
        frame,
        projection.selected_distribution_row(selected),
        chunks.details,
    );
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

fn render_details(
    frame: &mut Frame<'_>,
    app: &TuiApp,
    projection: &TuiFrameProjection,
    area: Rect,
) {
    let mut lines = Vec::new();
    if app.screen == TuiScreen::Treemap {
        lines.extend(treemap_context_ratatui_lines(
            app,
            projection.treemap_selection_summary(),
        ));
        lines.push(Line::from(""));
    }
    if let Some(row) = projection.selected_row(app.selected) {
        lines.push(Line::from(row.path.display().to_string()));
        lines.push(Line::from(format!(
            "{} logical, {}",
            format_bytes(row.metrics.logical_bytes),
            format_count(row.metrics.files, "file", "files")
        )));
        if let Some(advice) = row.cleanup_advice.as_ref() {
            lines.push(Line::from(format!("Advice: {}", advice.status.label())));
            lines.push(Line::from(format!("Reason: {}", advice.reason)));
            if let Some(rule_id) = advice.rule_id.as_ref() {
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
            if let Some(guidance) = advice.manual_guidance.as_ref() {
                lines.push(Line::from(format!(
                    "Manual review: {}",
                    guidance.manual_review_hint
                )));
                if let Some(tool_hint) = guidance.external_tool_hint.as_ref() {
                    lines.push(Line::from(format!("Tool hint: {tool_hint}")));
                }
                if let Some(evidence_path) = guidance.evidence_path.as_ref() {
                    lines.push(Line::from(format!("Evidence: {}", evidence_path.display())));
                }
            }
        } else {
            lines.push(Line::from("Advice: none"));
        }
    } else {
        lines.push(Line::from("No entries"));
    }

    lines.push(Line::from(""));
    lines.push(Line::from("Reclaim Basket"));
    if app.basket.is_empty() {
        lines.push(Line::from("  empty"));
    } else {
        lines.push(Line::from(format!(
            "  selected scopes: {} ({})",
            basket::total_source_logical_bytes(&app.basket),
            format_bytes(basket::total_source_logical_bytes(&app.basket))
        )));
        lines.push(Line::from(
            "  preview expands these rules into concrete targets",
        ));
        for item in app.basket.values() {
            lines.push(Line::from(format!("  {}", basket::label(item))));
            lines.push(Line::from(format!(
                "    from {}",
                basket::source_summary(item)
            )));
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
    let lines = plan_ratatui_lines(plan);
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: true })
            .block(Block::default().borders(Borders::ALL).title(title)),
        area,
    );
}

fn render_busy(frame: &mut Frame<'_>, app: &TuiApp, area: Rect) {
    let mut lines = vec![Line::from(app.message.clone())];
    lines.extend(task_status_ratatui_lines(app.task_status.as_ref()));
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
        Line::from("Rebecca will move allowed targets to the system trash or Recycle Bin."),
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
    let lines = history_ratatui_lines(app);
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: true })
            .block(Block::default().borders(Borders::ALL).title("History")),
        area,
    );
}

fn render_help(frame: &mut Frame<'_>, area: Rect) {
    frame.render_widget(
        Paragraph::new(help_ratatui_lines())
            .block(Block::default().borders(Borders::ALL).title("Help")),
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
            "{}{}{} | Enter open | Space add | c preview | ? all keys | q quit",
            app.message,
            search,
            active_group_filter_status(app)
        )),
        area,
    );
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

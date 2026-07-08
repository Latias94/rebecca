use crate::output::format_bytes;
use crate::tui::app::TuiApp;
use crate::tui::frame_projection::TuiFrameProjection;
use crate::tui::model::TuiScreen;
use crate::tui::navigation::RootChoice;
use crate::tui::presentation::{
    BAR_WIDTH, advice_label, byte_bar, distribution_count_label, distribution_empty_label,
    distribution_share_label, distribution_title, group_filter_suffix, help_lines, history_lines,
    map_title, max_distribution_logical, max_logical, plan_lines, screen_label, task_status_lines,
    treemap_context_lines, treemap_empty_message, trim_to_width,
};
use crate::tui::view::ViewOptions;

pub(crate) fn snapshot(app: &TuiApp, options: ViewOptions) -> String {
    let mut lines = Vec::new();
    let width = options.width;
    let projection = app.frame_projection();
    lines.push(trim_to_width(
        format!(
            "Rebecca TUI | {} | reclaim {} | sort {}{}",
            screen_label(app.screen),
            app.basket.len(),
            app.sort.label(),
            group_filter_suffix(app)
        ),
        width,
    ));
    match app.screen {
        TuiScreen::RootPicker => snapshot_root_picker(app, width, &mut lines),
        TuiScreen::Map => snapshot_map(app, &projection, options, &mut lines),
        TuiScreen::Treemap => snapshot_treemap(app, &projection, &mut lines, width),
        TuiScreen::Types | TuiScreen::Extensions => {
            snapshot_distribution(app, &projection, options, &mut lines)
        }
        TuiScreen::Busy => {
            lines.push(trim_to_width(format!("Busy: {}", app.message), width));
            for line in task_status_lines(app.task_status.as_ref()) {
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

fn snapshot_map(
    app: &TuiApp,
    projection: &TuiFrameProjection,
    options: ViewOptions,
    lines: &mut Vec<String>,
) {
    let width = options.width;
    let rows = projection.visible_rows();
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
                        byte_bar(row.metrics.logical_bytes, max_logical(rows), BAR_WIDTH)
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

fn snapshot_treemap(
    app: &TuiApp,
    projection: &TuiFrameProjection,
    lines: &mut Vec<String>,
    width: usize,
) {
    let rows = projection.visible_rows();
    let total = rows
        .iter()
        .map(|row| row.metrics.logical_bytes)
        .sum::<u64>()
        .max(1);
    lines.push(trim_to_width(map_title(app, "Treemap"), width));
    for line in treemap_context_lines(app, projection.treemap_selection_summary()) {
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

fn snapshot_distribution(
    app: &TuiApp,
    projection: &TuiFrameProjection,
    options: ViewOptions,
    lines: &mut Vec<String>,
) {
    let width = options.width;
    let rows = projection.distribution_rows();
    let max = max_distribution_logical(rows);
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
    plan: Option<&rebecca::core::plan::CleanupPlan>,
    width: usize,
    lines: &mut Vec<String>,
) {
    lines.push(trim_to_width(title.to_string(), width));
    for line in plan_lines(plan) {
        lines.push(trim_to_width(line, width));
    }
}

fn snapshot_help(width: usize, lines: &mut Vec<String>) {
    for line in help_lines() {
        lines.push(trim_to_width(line, width));
    }
}

fn snapshot_history(app: &TuiApp, width: usize, lines: &mut Vec<String>) {
    for line in history_lines(app) {
        lines.push(trim_to_width(line, width));
    }
}

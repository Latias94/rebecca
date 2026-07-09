use std::collections::BTreeSet;

use ratatui::text::Line;
use rebecca_core::cleanup_advice::CleanupAdviceStatus;
use rebecca_core::disk_session::{DiskMapDistributionRow, DiskMapVisibleRow};
use rebecca_core::plan::CleanupPlan;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::output::format_bytes;
use crate::text::format_count;
use crate::tui::app::{TuiApp, TuiTreemapSelectionSummary};
use crate::tui::model::TuiScreen;
use crate::tui::progress::TuiTaskStatus;

pub(crate) const BAR_WIDTH: usize = 12;

const CONFIRM_RULE_PREVIEW_LIMIT: usize = 4;

pub(crate) fn plan_lines(plan: Option<&CleanupPlan>) -> Vec<String> {
    let Some(plan) = plan else {
        return vec!["No plan available.".to_string()];
    };
    vec![
        format!(
            "Targets: {} total, {} allowed, {} blocked, {} skipped, {} failed",
            plan.summary.total_targets,
            plan.summary.allowed_targets,
            plan.summary.blocked_targets,
            plan.summary.skipped_targets,
            plan.summary.failed_targets
        ),
        format!(
            "Estimated: {} ({})",
            plan.summary.estimated_bytes,
            format_bytes(plan.summary.estimated_bytes)
        ),
        format!(
            "Freed: {} ({})",
            plan.summary.freed_bytes,
            format_bytes(plan.summary.freed_bytes)
        ),
        format!(
            "Pending reclaim: {} ({})",
            plan.summary.pending_reclaim_bytes,
            format_bytes(plan.summary.pending_reclaim_bytes)
        ),
        "Normal execution moves targets to system trash or Recycle Bin.".to_string(),
        "Preview pending trash space after execution: rebecca trash empty".to_string(),
        "Empty after review: rebecca trash empty --yes".to_string(),
        "e execute  Esc return  q quit".to_string(),
    ]
}

pub(crate) fn confirm_lines(app: &TuiApp) -> Vec<String> {
    let Some(plan) = app.preview.as_ref() else {
        return vec![
            "No cleanup preview is available.".to_string(),
            "Esc returns to the previous screen.".to_string(),
        ];
    };

    let mut lines = vec![
        "Execution: move allowed targets to the system trash or Recycle Bin.".to_string(),
        format!(
            "Allowed: {} of {} | Estimated: {} ({})",
            format_count(plan.summary.allowed_targets as u64, "target", "targets"),
            format_count(
                plan.summary.total_targets as u64,
                "total target",
                "total targets"
            ),
            plan.summary.estimated_bytes,
            format_bytes(plan.summary.estimated_bytes)
        ),
        format!(
            "Not moving now: {}, {}, {}",
            format_count(plan.summary.blocked_targets as u64, "blocked target", "blocked targets"),
            format_count(plan.summary.skipped_targets as u64, "skipped target", "skipped targets"),
            format_count(plan.summary.failed_targets as u64, "failed target", "failed targets")
        ),
        format!("Basket rules: {}", selected_rules_summary(app)),
        format!("Safety gates: {}", safety_gate_summary(app)),
        "After execution: preview trash with `rebecca trash empty`; empty reviewed trash with `rebecca trash empty --yes`.".to_string(),
        "Permanent delete is CLI-only: rerun the previewed rule with `rebecca clean --yes --permanent`.".to_string(),
        format!("Required phrase: {}", app.confirmation_phrase()),
        format!("Input: {}", app.confirmation_input),
    ];
    if app.message.starts_with("Confirmation must") {
        lines.push(app.message.clone());
    }
    lines.push("Enter executes; Esc returns to preview.".to_string());
    lines
}

pub(crate) fn help_lines() -> Vec<String> {
    vec![
        "j/k or arrows move".to_string(),
        "Enter/l opens a directory or treemap tile, h/Esc moves back".to_string(),
        "1 map, 4/w treemap, 2/t types, 3/x extensions, Tab cycles views".to_string(),
        "Enter filters by selected type/extension; Backspace clears group filter".to_string(),
        "/ filters the active view, s cycles sort".to_string(),
        "r patches the active directory, R refreshes the root".to_string(),
        "Mouse: click tabs, map rows, treemap tiles, or distribution rows; click selects only"
            .to_string(),
        "Space adds a cleanup rule to the Reclaim Basket".to_string(),
        "c previews concrete targets; e executes only after typed confirmation".to_string(),
        "g shows recent cleanup history".to_string(),
        "q quits".to_string(),
    ]
}

pub(crate) fn history_lines(app: &TuiApp) -> Vec<String> {
    if app.history.is_empty() {
        return vec!["No cleanup history entries yet.".to_string()];
    }

    let mut lines = Vec::with_capacity(app.history.len() + 2);
    lines.push("Recent cleanup history".to_string());
    for (index, entry) in app.history.iter().rev().enumerate() {
        let bytes = entry
            .summary
            .freed_bytes
            .saturating_add(entry.summary.pending_reclaim_bytes);
        lines.push(format!(
            "#{:>2} {} | targets {} done, {} blocked, {} skipped | {}",
            index + 1,
            entry.recorded_at_unix_seconds,
            entry.summary.completed_targets,
            entry.summary.blocked_targets,
            entry.summary.skipped_targets,
            format_bytes(bytes),
        ));
    }
    lines.push("Esc returns to map, q quits.".to_string());
    lines
}

pub(crate) fn task_status_lines(status: Option<&TuiTaskStatus>) -> Vec<String> {
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

pub(crate) fn treemap_context_lines(
    app: &TuiApp,
    summary: Option<&TuiTreemapSelectionSummary>,
) -> Vec<String> {
    let mut lines = Vec::new();
    lines.push(format!("Scope: {}", app.current_node_name()));
    lines.push(format!("Breadcrumb: {}", app.current_scope_breadcrumb()));
    lines.push(format!("Zoom depth: {}", app.zoom_depth()));
    lines.push(format!(
        "Filter: {}",
        app.active_scope_filter_summary()
            .unwrap_or_else(|| "none".to_string())
    ));
    if let Some(summary) = summary {
        lines.push(format!("Selected tile: {}", summary.name));
        lines.push(format!("Kind: {}", summary.kind));
        lines.push(format!(
            "Drillable: {}",
            if summary.drillable { "yes" } else { "no" }
        ));
        if let Some(reason) = summary.non_drillable_reason.as_ref() {
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

pub(crate) fn group_filter_suffix(app: &TuiApp) -> String {
    app.active_group_filter_label()
        .map(|label| format!(" | filter {label}"))
        .unwrap_or_default()
}

pub(crate) fn active_group_filter_status(app: &TuiApp) -> String {
    app.active_group_filter_label()
        .map(|label| format!(" | group filter: {label}"))
        .unwrap_or_default()
}

pub(crate) fn map_title(app: &TuiApp, prefix: &str) -> String {
    match app.active_group_filter_label() {
        Some(label) => format!("{prefix}: {} [{label}]", app.current_node_name()),
        None => format!("{prefix}: {}", app.current_node_name()),
    }
}

pub(crate) fn plan_ratatui_lines(plan: Option<&CleanupPlan>) -> Vec<Line<'static>> {
    strings_to_lines(plan_lines(plan))
}

pub(crate) fn confirm_ratatui_lines(app: &TuiApp) -> Vec<Line<'static>> {
    strings_to_lines(confirm_lines(app))
}

pub(crate) fn help_ratatui_lines() -> Vec<Line<'static>> {
    strings_to_lines(help_lines())
}

pub(crate) fn history_ratatui_lines(app: &TuiApp) -> Vec<Line<'static>> {
    strings_to_lines(history_lines(app))
}

pub(crate) fn task_status_ratatui_lines(status: Option<&TuiTaskStatus>) -> Vec<Line<'static>> {
    strings_to_lines(task_status_lines(status))
}

pub(crate) fn treemap_context_ratatui_lines(
    app: &TuiApp,
    summary: Option<&TuiTreemapSelectionSummary>,
) -> Vec<Line<'static>> {
    strings_to_lines(treemap_context_lines(app, summary))
}

pub(crate) fn advice_label(row: &DiskMapVisibleRow) -> String {
    row.cleanup_advice
        .as_ref()
        .map(|advice| advice.status.label().to_string())
        .unwrap_or_else(|| CleanupAdviceStatus::Unknown.label().to_string())
}

pub(crate) fn max_logical(rows: &[DiskMapVisibleRow]) -> u64 {
    rows.iter()
        .map(|row| row.metrics.logical_bytes)
        .max()
        .unwrap_or(0)
}

pub(crate) fn max_distribution_logical(rows: &[DiskMapDistributionRow]) -> u64 {
    rows.iter()
        .map(|row| row.metrics.logical_bytes)
        .max()
        .unwrap_or(0)
}

pub(crate) fn distribution_title(screen: TuiScreen) -> &'static str {
    match screen {
        TuiScreen::Types => "Types: file kind distribution",
        TuiScreen::Extensions => "Extensions: suffix distribution",
        _ => "Distribution",
    }
}

pub(crate) fn distribution_empty_label(screen: TuiScreen) -> &'static str {
    match screen {
        TuiScreen::Types => "No type distribution for this scan.",
        TuiScreen::Extensions => "No extension distribution for this scan.",
        _ => "No distribution rows for this scan.",
    }
}

pub(crate) fn distribution_count_label(row: &DiskMapDistributionRow) -> String {
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

pub(crate) fn distribution_share_label(row: &DiskMapDistributionRow) -> String {
    if row.scope_logical_bytes == 0 {
        return "0.0%".to_string();
    }
    format!(
        "{:.1}%",
        (row.metrics.logical_bytes as f64 / row.scope_logical_bytes as f64) * 100.0
    )
}

pub(crate) fn byte_bar(value: u64, max: u64, width: usize) -> String {
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

pub(crate) fn trim_to_width(value: impl Into<String>, width: usize) -> String {
    let value = value.into();
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

pub(crate) fn treemap_empty_message(app: &TuiApp) -> String {
    if app.active_scope_filter_summary().is_some() {
        "No entries match the active filters in this scope.".to_string()
    } else {
        "No non-empty entries.".to_string()
    }
}

pub(crate) fn screen_label(screen: TuiScreen) -> &'static str {
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

fn strings_to_lines(lines: Vec<String>) -> Vec<Line<'static>> {
    lines.into_iter().map(Line::from).collect()
}

fn selected_rules_summary(app: &TuiApp) -> String {
    if app.basket.is_empty() {
        return "none".to_string();
    }

    let rules: Vec<&str> = app
        .basket
        .keys()
        .take(CONFIRM_RULE_PREVIEW_LIMIT)
        .map(String::as_str)
        .collect();
    let omitted = app.basket.len().saturating_sub(rules.len());
    let mut summary = rules.join(", ");
    if omitted > 0 {
        summary.push_str(&format!(", +{omitted} more"));
    }
    summary
}

fn safety_gate_summary(app: &TuiApp) -> String {
    let mut flags = BTreeSet::new();
    let mut warnings = BTreeSet::new();
    for item in app.basket.values() {
        flags.extend(item.required_flags.iter().cloned());
        warnings.extend(item.required_warnings.iter().cloned());
    }

    let mut parts = Vec::new();
    if !flags.is_empty() {
        parts.push(format!(
            "flags {}",
            flags.into_iter().collect::<Vec<_>>().join(", ")
        ));
    }
    if !warnings.is_empty() {
        parts.push(format!(
            "warnings {}",
            warnings.into_iter().collect::<Vec<_>>().join(", ")
        ));
    }
    if parts.is_empty() {
        "none".to_string()
    } else {
        parts.join("; ")
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use rebecca_core::cleanup_advice::CleanupAdviceStatus;
    use rebecca_core::plan::{CleanupPlan, CleanupSummary};
    use rebecca_core::scan::ScanBackendKind;
    use rebecca_core::{DeleteMode, PlanRequest, Platform};
    use unicode_width::UnicodeWidthStr;

    use crate::tui::basket::CleanupBasketItem;

    use super::*;

    #[test]
    fn plan_lines_recommend_previewing_trash_before_emptying() {
        let mut plan = CleanupPlan::empty(PlanRequest::for_platform(
            Platform::Windows,
            DeleteMode::RecoverableDelete,
        ));
        plan.summary = CleanupSummary {
            pending_reclaim_bytes: 5,
            ..CleanupSummary::default()
        };

        let lines = plan_lines(Some(&plan));
        let preview_index = lines
            .iter()
            .position(|line| {
                line == "Preview pending trash space after execution: rebecca trash empty"
            })
            .expect("TUI plan lines should recommend reviewing pending trash space");
        let empty_index = lines
            .iter()
            .position(|line| line == "Empty after review: rebecca trash empty --yes")
            .expect("TUI plan lines should still show the confirmed empty command");

        assert!(preview_index < empty_index);
    }

    #[test]
    fn confirm_lines_show_execution_decision_context() {
        let mut plan = CleanupPlan::empty(PlanRequest::for_platform(
            Platform::Linux,
            DeleteMode::DryRun,
        ));
        plan.summary = CleanupSummary {
            total_targets: 4,
            allowed_targets: 2,
            blocked_targets: 1,
            skipped_targets: 1,
            estimated_bytes: 4096,
            ..CleanupSummary::default()
        };
        let mut app = TuiApp::root_picker(Vec::new(), ScanBackendKind::PortableRecursive, 100);
        app.preview = Some(plan);
        app.confirmation_input = "CLEAN 4096".to_string();
        app.basket.insert(
            "linux.browser-cache".to_string(),
            CleanupBasketItem {
                rule_id: "linux.browser-cache".to_string(),
                status: CleanupAdviceStatus::MaybeCleanable,
                reason: "browser cache".to_string(),
                required_flags: vec!["--allow-moderate".to_string()],
                required_warnings: vec!["active-process".to_string()],
                source_path: PathBuf::from("/home/user/.cache/browser"),
                source_logical_bytes: 4096,
                source_files: 0,
                source_directories: 0,
                covered_path_count: 2,
            },
        );

        let text = confirm_lines(&app).join("\n");

        assert!(
            text.contains("Execution: move allowed targets to the system trash or Recycle Bin.")
        );
        assert!(
            text.contains("Allowed: 2 targets of 4 total targets | Estimated: 4096 (4.00 KiB)")
        );
        assert!(
            text.contains("Not moving now: 1 blocked target, 1 skipped target, 0 failed targets")
        );
        assert!(text.contains("Basket rules: linux.browser-cache"));
        assert!(text.contains("Safety gates: flags --allow-moderate; warnings active-process"));
        assert!(text.contains("Required phrase: CLEAN 4096"));
        assert!(text.contains("Input: CLEAN 4096"));
        assert!(text.contains("rebecca trash empty --yes"));
        assert!(text.contains("rebecca clean --yes --permanent"));
    }

    #[test]
    fn trim_to_width_respects_display_width_for_cjk_text() {
        let line = trim_to_width("缓存目录-with-a-very-long-name", 12);

        assert!(UnicodeWidthStr::width(line.as_str()) <= 12, "{line}");
        assert!(line.ends_with("..."));
    }
}

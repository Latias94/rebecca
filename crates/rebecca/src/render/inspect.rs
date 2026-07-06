use std::collections::BTreeSet;

use anyhow::Result;
use rebecca::core::cleanup_advice::{CleanupAdvice, CleanupAdviceStatus};
use rebecca::core::disk_map::{DiskMapEntry, DiskMapMetrics, DiskMapReport};
use rebecca::core::inspect::SpaceInsightReport;
use rebecca::core::lint::LintReport;

use crate::output::{format_bytes, format_shell_command};
use crate::render::{estimate_provenance_suffix, format_count};

const MAP_ENTRY_DEFAULT_BAR_WIDTH: usize = 20;
const MAP_ENTRY_MIN_BAR_WIDTH: usize = 4;
const MAP_ENTRY_MAX_BAR_WIDTH: usize = 80;
const MAP_ENTRY_PATH_MAX_CHARS: usize = 96;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct InspectMapRenderOptions {
    pub(crate) screen_reader: bool,
    pub(crate) full_path: bool,
    pub(crate) no_bars: bool,
    pub(crate) bar_width: Option<usize>,
}

pub(crate) fn print_space_report(report: &SpaceInsightReport) -> Result<()> {
    println!("Space insight");
    println!("Roots: {}", report.roots.len());
    println!(
        "Estimated bytes: {} ({})",
        report.totals.estimated_bytes,
        format_bytes(report.totals.estimated_bytes)
    );
    println!("Files: {}", report.totals.files);
    println!("Directories: {}", report.totals.directories);
    println!("Diagnostics: {}", report.diagnostic_summary.total);

    if !report.top_entries.is_empty() {
        println!();
        println!("Top entries:");
        for entry in &report.top_entries {
            println!(
                "  - {} [{}] {} bytes ({}){} - {} files, {} dirs",
                entry.path.display(),
                entry.kind.label(),
                entry.estimated_bytes,
                format_bytes(entry.estimated_bytes),
                estimate_provenance_suffix(entry.estimate_source, &entry.estimate_provenance),
                entry.files,
                entry.directories
            );
        }
    }

    if report.diagnostic_summary.total > 0 {
        println!();
        println!(
            "Space diagnostics: {}",
            format_count(
                report.diagnostic_summary.total,
                "observation",
                "observations"
            )
        );
        for summary in &report.diagnostic_summary.by_kind {
            println!(
                "  - {}: {}",
                summary.kind.label(),
                format_count(summary.count, "observation", "observations")
            );
        }
        if report.diagnostic_summary.truncated > 0 {
            println!(
                "  - truncated: {} not shown",
                format_count(
                    report.diagnostic_summary.truncated,
                    "observation",
                    "observations"
                )
            );
        }
    }

    if !report.diagnostics.is_empty() {
        println!();
        println!(
            "Space diagnostic samples: {}",
            format_count(
                report.diagnostic_summary.retained,
                "observation",
                "observations"
            )
        );
        for diagnostic in &report.diagnostics {
            println!(
                "  - {} {} - {}",
                diagnostic.kind.label(),
                diagnostic.path.display(),
                diagnostic.detail
            );
        }
    }

    Ok(())
}

pub(crate) fn print_map_report(
    report: &DiskMapReport,
    options: InspectMapRenderOptions,
) -> Result<()> {
    println!("Disk map");
    println!("Roots: {}", report.roots.len());
    println!(
        "Logical bytes: {} ({})",
        report.totals.logical_bytes,
        format_bytes(report.totals.logical_bytes)
    );
    if let Some(allocated_bytes) = report.totals.allocated_bytes {
        println!(
            "Allocated bytes: {} ({})",
            allocated_bytes,
            format_bytes(allocated_bytes)
        );
    }
    if let Some(unique_logical_bytes) = report.totals.unique_logical_bytes {
        println!(
            "Unique logical bytes: {} ({})",
            unique_logical_bytes,
            format_bytes(unique_logical_bytes)
        );
    }
    if let Some(unique_allocated_bytes) = report.totals.unique_allocated_bytes {
        println!(
            "Unique allocated bytes: {} ({})",
            unique_allocated_bytes,
            format_bytes(unique_allocated_bytes)
        );
    }
    println!("Files: {}", report.totals.files);
    println!("Directories: {}", report.totals.directories);
    println!("Diagnostics: {}", report.diagnostic_summary.total);

    if !report.top_entries.is_empty() {
        println!();
        print_top_map_entries(report, options);
    }

    print_cleanup_advice_summary(report);

    if !report.groups.is_empty() {
        println!();
        print_map_groups(report, options);
    }

    if report.diagnostic_summary.total > 0 {
        println!();
        println!(
            "Disk map diagnostics: {}",
            format_count(
                report.diagnostic_summary.total,
                "observation",
                "observations"
            )
        );
        for summary in &report.diagnostic_summary.by_kind {
            println!(
                "  - {}: {}",
                summary.kind.label(),
                format_count(summary.count, "observation", "observations")
            );
        }
        if report.diagnostic_summary.truncated > 0 {
            println!(
                "  - truncated: {} not shown",
                format_count(
                    report.diagnostic_summary.truncated,
                    "observation",
                    "observations"
                )
            );
        }
    }

    if !report.diagnostics.is_empty() {
        println!();
        println!(
            "Disk map diagnostic samples: {}",
            format_count(
                report.diagnostic_summary.retained,
                "observation",
                "observations"
            )
        );
        for diagnostic in &report.diagnostics {
            println!(
                "  - {} {} - {}",
                diagnostic.kind.label(),
                diagnostic.path.display(),
                diagnostic.detail
            );
        }
    }

    Ok(())
}

fn print_top_map_entries(report: &DiskMapReport, options: InspectMapRenderOptions) {
    if options.screen_reader {
        println!("Top map entries (screen-reader):");
        for (index, entry) in report.top_entries.iter().enumerate() {
            print_screen_reader_map_entry(index + 1, entry, report.totals.logical_bytes);
        }
    } else {
        println!("Top map entries:");
        for (index, entry) in report.top_entries.iter().enumerate() {
            print_visual_map_entry(index + 1, entry, report.totals.logical_bytes, options);
        }
    }
}

fn print_visual_map_entry(
    rank: usize,
    entry: &DiskMapEntry,
    total_logical_bytes: u64,
    options: InspectMapRenderOptions,
) {
    let share_percent = share_percent(entry.logical_bytes, total_logical_bytes);
    println!(
        "  #{rank:<2} {} bytes ({}) {:>5.1}%{} {} [{} depth={}]{} - {}, {}{}",
        entry.logical_bytes,
        format_bytes(entry.logical_bytes),
        share_percent,
        visual_usage_bar_suffix(share_percent, options),
        map_entry_path(entry, options),
        entry.kind.label(),
        entry.depth,
        map_entry_metric_suffix(entry),
        format_count(entry.files, "file", "files"),
        format_count(entry.directories, "dir", "dirs"),
        cleanup_advice_suffix(entry.cleanup_advice.as_ref())
    );
}

fn print_map_groups(report: &DiskMapReport, options: InspectMapRenderOptions) {
    if options.screen_reader {
        println!("Map groups (screen-reader):");
        for (index, group) in report.groups.iter().enumerate() {
            let share_percent =
                share_percent(group.metrics.logical_bytes, report.totals.logical_bytes);
            println!(
                "  #{}: {} [{}]; {} bytes ({}); share {:.1}%; {}{}",
                index + 1,
                group.label,
                group.kind.label(),
                group.metrics.logical_bytes,
                format_bytes(group.metrics.logical_bytes),
                share_percent,
                format_count(group.metrics.files, "file", "files"),
                screen_reader_metrics_suffix(&group.metrics)
            );
        }
        return;
    }

    println!("Map groups:");
    for (index, group) in report.groups.iter().enumerate() {
        let share_percent = share_percent(group.metrics.logical_bytes, report.totals.logical_bytes);
        println!(
            "  #{:<2} {} bytes ({}) {:>5.1}%{} {} [{}]{} - {}",
            index + 1,
            group.metrics.logical_bytes,
            format_bytes(group.metrics.logical_bytes),
            share_percent,
            visual_usage_bar_suffix(share_percent, options),
            group.label,
            group.kind.label(),
            map_metrics_suffix(&group.metrics),
            format_count(group.metrics.files, "file", "files")
        );
    }
}

fn print_screen_reader_map_entry(rank: usize, entry: &DiskMapEntry, total_logical_bytes: u64) {
    let share_percent = share_percent(entry.logical_bytes, total_logical_bytes);
    println!(
        "  #{rank}: {} bytes ({}); share {:.1}%; path {}; kind {}; depth {}; {}; {}{}{}",
        entry.logical_bytes,
        format_bytes(entry.logical_bytes),
        share_percent,
        entry.path.display(),
        entry.kind.label(),
        entry.depth,
        format_count(entry.files, "file", "files"),
        format_count(entry.directories, "directory", "directories"),
        screen_reader_metric_suffix(entry),
        cleanup_advice_screen_reader_suffix(entry.cleanup_advice.as_ref())
    );
}

fn share_percent(bytes: u64, total_bytes: u64) -> f64 {
    if total_bytes == 0 {
        return 0.0;
    }

    ((bytes as f64 / total_bytes as f64) * 100.0).clamp(0.0, 100.0)
}

fn usage_bar(share_percent: f64, width: usize) -> String {
    let clamped = share_percent.clamp(0.0, 100.0);
    let filled = ((clamped / 100.0) * width as f64).round() as usize;
    let filled = filled.min(width);
    format!("[{}{}]", "#".repeat(filled), "-".repeat(width - filled))
}

fn visual_usage_bar_suffix(share_percent: f64, options: InspectMapRenderOptions) -> String {
    if options.screen_reader || options.no_bars {
        return String::new();
    }

    format!(" {}", usage_bar(share_percent, map_bar_width(options)))
}

fn map_bar_width(options: InspectMapRenderOptions) -> usize {
    options
        .bar_width
        .unwrap_or(MAP_ENTRY_DEFAULT_BAR_WIDTH)
        .clamp(MAP_ENTRY_MIN_BAR_WIDTH, MAP_ENTRY_MAX_BAR_WIDTH)
}

fn map_entry_path(entry: &DiskMapEntry, options: InspectMapRenderOptions) -> String {
    if options.full_path {
        entry.path.display().to_string()
    } else {
        compact_path(entry, MAP_ENTRY_PATH_MAX_CHARS)
    }
}

fn compact_path(entry: &DiskMapEntry, max_chars: usize) -> String {
    let display = entry.path.display().to_string();
    let char_count = display.chars().count();
    if char_count <= max_chars {
        return display;
    }

    let marker = "...";
    let marker_chars = marker.chars().count();
    if max_chars <= marker_chars + 2 {
        return marker.to_string();
    }

    let visible_chars = max_chars - marker_chars;
    let prefix_chars = visible_chars / 3;
    let suffix_chars = visible_chars - prefix_chars;
    let prefix = display.chars().take(prefix_chars).collect::<String>();
    let suffix = display
        .chars()
        .skip(char_count.saturating_sub(suffix_chars))
        .collect::<String>();
    format!("{prefix}{marker}{suffix}")
}

fn map_metrics_suffix(metrics: &DiskMapMetrics) -> String {
    let allocated = metrics
        .allocated_bytes
        .map(|bytes| format!(", allocated {} ({})", bytes, format_bytes(bytes)))
        .unwrap_or_default();
    let unique_logical = metrics
        .unique_logical_bytes
        .map(|bytes| format!(", unique logical {} ({})", bytes, format_bytes(bytes)))
        .unwrap_or_default();
    let unique_allocated = metrics
        .unique_allocated_bytes
        .map(|bytes| format!(", unique allocated {} ({})", bytes, format_bytes(bytes)))
        .unwrap_or_default();
    format!("{allocated}{unique_logical}{unique_allocated}")
}

fn map_entry_metric_suffix(entry: &DiskMapEntry) -> String {
    let allocated = entry
        .allocated_bytes
        .map(|bytes| format!(", allocated {} ({})", bytes, format_bytes(bytes)))
        .unwrap_or_default();
    let unique_logical = entry
        .unique_logical_bytes
        .map(|bytes| format!(", unique logical {} ({})", bytes, format_bytes(bytes)))
        .unwrap_or_default();
    let unique_allocated = entry
        .unique_allocated_bytes
        .map(|bytes| format!(", unique allocated {} ({})", bytes, format_bytes(bytes)))
        .unwrap_or_default();
    format!(
        "{allocated}{unique_logical}{unique_allocated}{}",
        estimate_provenance_suffix(entry.estimate_source, &entry.estimate_provenance)
    )
}

fn screen_reader_metrics_suffix(metrics: &DiskMapMetrics) -> String {
    let mut parts = Vec::new();
    if let Some(bytes) = metrics.allocated_bytes {
        parts.push(format!("allocated {} ({})", bytes, format_bytes(bytes)));
    }
    if let Some(bytes) = metrics.unique_logical_bytes {
        parts.push(format!(
            "unique logical {} ({})",
            bytes,
            format_bytes(bytes)
        ));
    }
    if let Some(bytes) = metrics.unique_allocated_bytes {
        parts.push(format!(
            "unique allocated {} ({})",
            bytes,
            format_bytes(bytes)
        ));
    }

    if parts.is_empty() {
        String::new()
    } else {
        format!("; {}", parts.join("; "))
    }
}

fn screen_reader_metric_suffix(entry: &DiskMapEntry) -> String {
    let mut parts = Vec::new();
    if let Some(bytes) = entry.allocated_bytes {
        parts.push(format!("allocated {} ({})", bytes, format_bytes(bytes)));
    }
    if let Some(bytes) = entry.unique_logical_bytes {
        parts.push(format!(
            "unique logical {} ({})",
            bytes,
            format_bytes(bytes)
        ));
    }
    if let Some(bytes) = entry.unique_allocated_bytes {
        parts.push(format!(
            "unique allocated {} ({})",
            bytes,
            format_bytes(bytes)
        ));
    }

    let provenance = estimate_provenance_suffix(entry.estimate_source, &entry.estimate_provenance);
    if !provenance.is_empty() {
        parts.push(
            provenance
                .trim()
                .trim_start_matches('[')
                .trim_end_matches(']')
                .to_string(),
        );
    }

    if parts.is_empty() {
        String::new()
    } else {
        format!("; {}", parts.join("; "))
    }
}

fn cleanup_advice_screen_reader_suffix(advice: Option<&CleanupAdvice>) -> String {
    cleanup_advice_suffix(advice)
        .strip_prefix(" - ")
        .map(|suffix| format!("; {suffix}"))
        .unwrap_or_default()
}

fn print_cleanup_advice_summary(report: &DiskMapReport) {
    let advised_entries = report
        .top_entries
        .iter()
        .filter_map(|entry| {
            entry
                .cleanup_advice
                .as_ref()
                .map(|advice| (advice, entry.logical_bytes))
        })
        .collect::<Vec<_>>();

    if advised_entries.is_empty() {
        return;
    }

    println!();
    println!("Cleanup advice summary:");
    for status in [
        CleanupAdviceStatus::Cleanable,
        CleanupAdviceStatus::MaybeCleanable,
        CleanupAdviceStatus::ContainsCleanable,
        CleanupAdviceStatus::Protected,
        CleanupAdviceStatus::Unknown,
    ] {
        let (entries, bytes) = cleanup_advice_totals(&advised_entries, status);
        if entries > 0 {
            println!(
                "- {}: {}, {} ({})",
                status.label(),
                format_count(entries, "entry", "entries"),
                bytes,
                format_bytes(bytes)
            );
        }
    }

    let commands = advised_entries
        .iter()
        .filter_map(|(advice, _)| cleanup_advice_command(advice))
        .collect::<BTreeSet<_>>();

    if !commands.is_empty() {
        println!("Suggested cleanup commands:");
        for command in commands {
            println!("- {command}");
        }
        println!("Cleanup advice is read-only; rerun a suggested command to preview cleanup.");
    } else {
        println!("Suggested cleanup commands: none");
        println!("Cleanup advice is read-only; no cleanup rule matched the ranked entries.");
    }
}

fn cleanup_advice_totals(
    advised_entries: &[(&CleanupAdvice, u64)],
    status: CleanupAdviceStatus,
) -> (u64, u64) {
    advised_entries
        .iter()
        .filter(|(advice, _)| advice.status == status)
        .fold((0_u64, 0_u64), |(entries, bytes), (_, entry_bytes)| {
            (
                entries.saturating_add(1),
                bytes.saturating_add(*entry_bytes),
            )
        })
}

fn cleanup_advice_command(advice: &CleanupAdvice) -> Option<String> {
    let command = advice.suggested_command.as_ref()?;
    let mut args = command.args.clone();
    for flag in &advice.required_flags {
        args.extend(flag.split_whitespace().map(str::to_string));
    }
    for warning in &advice.required_warnings {
        args.push("--allow-warning".to_string());
        args.push(warning.clone());
    }
    Some(format_shell_command(&command.command, &args))
}

fn cleanup_advice_suffix(advice: Option<&CleanupAdvice>) -> String {
    let Some(advice) = advice else {
        return String::new();
    };

    let source = advice
        .source
        .map(|source| format!(" {}", source.label()))
        .unwrap_or_default();
    let rule = advice
        .rule_id
        .as_ref()
        .map(|rule_id| format!(" {rule_id}"))
        .unwrap_or_default();
    format!(" - cleanup: {}{}{}", advice.status.label(), source, rule)
}

pub(crate) fn print_lint_report(report: &LintReport) -> Result<()> {
    println!("Lint report");
    println!("Roots: {}", report.roots.len());
    println!("Reference roots: {}", report.reference_roots.len());
    println!("Files: {}", report.summary.files_scanned);
    println!("Directories: {}", report.summary.directories_scanned);
    println!("Duplicate groups: {}", report.summary.duplicate_groups);
    println!("Large files: {}", report.summary.large_files);
    println!("Empty files: {}", report.summary.empty_files);
    println!("Empty directories: {}", report.summary.empty_directories);
    println!(
        "Conservative reclaim estimate: {} ({})",
        report.summary.conservative_reclaim_bytes,
        format_bytes(report.summary.conservative_reclaim_bytes)
    );

    if !report.duplicate_groups.is_empty() {
        println!();
        println!("Duplicate groups:");
        for group in &report.duplicate_groups {
            println!(
                "  - {} files, {} bytes each, reclaim {} ({})",
                group.total_files,
                group.size_bytes,
                group.conservative_reclaim_bytes,
                format_bytes(group.conservative_reclaim_bytes)
            );
            for file in &group.files {
                println!("    - [{}] {}", file.role.label(), file.path.display());
            }
        }
    }

    if !report.large_files.is_empty() {
        println!();
        println!("Large files:");
        for file in &report.large_files {
            println!(
                "  - [{}] {} bytes ({}) {}",
                file.role.label(),
                file.size_bytes,
                format_bytes(file.size_bytes),
                file.path.display()
            );
        }
    }

    if !report.empty_files.is_empty() {
        println!();
        println!("Empty files:");
        for file in &report.empty_files {
            println!("  - [{}] {}", file.role.label(), file.path.display());
        }
    }

    if !report.empty_directories.is_empty() {
        println!();
        println!("Empty directories:");
        for directory in &report.empty_directories {
            println!(
                "  - [{}] depth {} {}",
                directory.role.label(),
                directory.depth,
                directory.path.display()
            );
        }
    }

    if !report.diagnostics.is_empty() {
        println!();
        println!("Inventory diagnostics:");
        for diagnostic in &report.diagnostics {
            println!(
                "  - {} {} - {}",
                diagnostic.kind.label(),
                diagnostic.path.display(),
                diagnostic.detail
            );
        }
    }

    Ok(())
}

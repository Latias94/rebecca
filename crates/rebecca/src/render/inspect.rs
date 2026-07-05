use std::collections::BTreeSet;

use anyhow::Result;
use rebecca::core::cleanup_advice::{CleanupAdvice, CleanupAdviceStatus};
use rebecca::core::disk_map::DiskMapReport;
use rebecca::core::inspect::SpaceInsightReport;
use rebecca::core::lint::LintReport;

use crate::output::{format_bytes, format_shell_command};
use crate::render::{estimate_provenance_suffix, format_count};

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

pub(crate) fn print_map_report(report: &DiskMapReport) -> Result<()> {
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
        println!("Top map entries:");
        for entry in &report.top_entries {
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
            println!(
                "  - {} [{} depth={}] {} bytes ({}){}{}{}{} - {} files, {} dirs{}",
                entry.path.display(),
                entry.kind.label(),
                entry.depth,
                entry.logical_bytes,
                format_bytes(entry.logical_bytes),
                allocated,
                unique_logical,
                unique_allocated,
                estimate_provenance_suffix(entry.estimate_source, &entry.estimate_provenance),
                entry.files,
                entry.directories,
                cleanup_advice_suffix(entry.cleanup_advice.as_ref())
            );
        }
    }

    print_cleanup_advice_summary(report);

    if !report.groups.is_empty() {
        println!();
        println!("Map groups:");
        for group in &report.groups {
            let allocated = group
                .metrics
                .allocated_bytes
                .map(|bytes| format!(", allocated {} ({})", bytes, format_bytes(bytes)))
                .unwrap_or_default();
            let unique_logical = group
                .metrics
                .unique_logical_bytes
                .map(|bytes| format!(", unique logical {} ({})", bytes, format_bytes(bytes)))
                .unwrap_or_default();
            let unique_allocated = group
                .metrics
                .unique_allocated_bytes
                .map(|bytes| format!(", unique allocated {} ({})", bytes, format_bytes(bytes)))
                .unwrap_or_default();
            println!(
                "  - {} [{}] {} bytes ({}){}{}{} - {} files",
                group.label,
                group.kind.label(),
                group.metrics.logical_bytes,
                format_bytes(group.metrics.logical_bytes),
                allocated,
                unique_logical,
                unique_allocated,
                group.metrics.files
            );
        }
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

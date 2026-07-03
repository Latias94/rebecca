use anyhow::Result;
use rebecca::core::disk_map::DiskMapReport;
use rebecca::core::inspect::SpaceInsightReport;
use rebecca::core::lint::LintReport;

use crate::output::format_bytes;
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
                "  - {} [{} depth={}] {} bytes ({}){}{}{}{} - {} files, {} dirs",
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
                entry.directories
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

use std::fmt::Write as _;

use anyhow::Result;
use rebecca_core::cache::{CachePurgeMode, CachePurgeReport, purge_app_cache};
use rebecca_core::config::load_app_paths;

use crate::output::format_bytes;

#[derive(Debug)]
pub struct CachePurgeOptions {
    pub dry_run: bool,
    pub json: bool,
    pub yes: bool,
}

pub fn purge(options: CachePurgeOptions) -> Result<()> {
    let paths = load_app_paths()?;
    let mode = if options.yes && !options.dry_run {
        CachePurgeMode::Delete
    } else {
        CachePurgeMode::DryRun
    };
    let report = purge_app_cache(&paths, mode)?;

    if options.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    print!("{}", render_cache_purge_report(&report));
    Ok(())
}

fn render_cache_purge_report(report: &CachePurgeReport) -> String {
    let mut output = String::new();
    writeln!(output, "Rebecca cache: {}", report.cache_dir.display()).unwrap();
    writeln!(output, "Mode: {}", mode_label(report.mode)).unwrap();
    writeln!(
        output,
        "Lifecycle: {} ({})",
        report.cache_dir_lifecycle.label(),
        report.cache_dir_retention.label()
    )
    .unwrap();
    writeln!(
        output,
        "Cache directory exists: {}",
        yes_no(report.cache_dir_exists)
    )
    .unwrap();
    writeln!(
        output,
        "Preserves cache directory: {}",
        yes_no(report.preserves_cache_dir)
    )
    .unwrap();
    writeln!(
        output,
        "Entries: {}, files: {}, directories: {}",
        report.summary.total_entries, report.summary.files, report.summary.directories
    )
    .unwrap();
    writeln!(
        output,
        "Entry status: {} would delete, {} deleted, {} skipped, {} failed",
        report.summary.would_delete_entries,
        report.summary.deleted_entries,
        report.summary.skipped_entries,
        report.summary.failed_entries
    )
    .unwrap();
    writeln!(
        output,
        "Estimated bytes: {} ({})",
        report.summary.estimated_bytes,
        format_bytes(report.summary.estimated_bytes)
    )
    .unwrap();
    writeln!(
        output,
        "Reclaimed bytes: {} ({})",
        report.summary.reclaimed_bytes,
        format_bytes(report.summary.reclaimed_bytes)
    )
    .unwrap();

    if !report.summary.issue_matrix.is_empty() {
        writeln!(output, "Issue matrix:").unwrap();
        for issue in &report.summary.issue_matrix {
            writeln!(
                output,
                "- {} {}: {}, {} ({})",
                issue.status.label(),
                issue.reason_code.label(),
                format_count(issue.entries, "entry", "entries"),
                issue.estimated_bytes,
                format_bytes(issue.estimated_bytes)
            )
            .unwrap();
        }
    }

    if report.entries.is_empty() {
        writeln!(output, "No cache entries found.").unwrap();
        return output;
    }

    if report.mode == CachePurgeMode::DryRun {
        writeln!(
            output,
            "Run with --yes to purge these rebuildable cache entries."
        )
        .unwrap();
    }

    writeln!(output, "Cache entries:").unwrap();
    for entry in &report.entries {
        let reason = entry
            .reason
            .as_deref()
            .map(|reason| format!(" - {reason}"))
            .unwrap_or_default();
        writeln!(
            output,
            "- {}: {} ({}; {} file(s), {} dir(s)){}",
            entry.status.label(),
            entry.path.display(),
            format_bytes(entry.estimated_bytes),
            entry.files,
            entry.directories,
            reason
        )
        .unwrap();
    }

    output
}

fn mode_label(mode: CachePurgeMode) -> &'static str {
    match mode {
        CachePurgeMode::DryRun => "dry-run",
        CachePurgeMode::Delete => "delete",
    }
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

fn format_count(value: usize, singular: &str, plural: &str) -> String {
    if value == 1 {
        format!("1 {singular}")
    } else {
        format!("{value} {plural}")
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use rebecca_core::cache::{
        CachePurgeEntry, CachePurgeEntryKind, CachePurgeEntryReason, CachePurgeEntryStatus,
        CachePurgeIssueSummary, CachePurgeSummary,
    };
    use rebecca_core::config::{AppStorageLifecycle, AppStorageRetention};

    use super::{CachePurgeMode, CachePurgeReport, render_cache_purge_report};

    #[test]
    fn render_cache_purge_report_includes_issue_matrix_when_present() {
        let report = CachePurgeReport {
            cache_dir: PathBuf::from("cache"),
            cache_dir_lifecycle: AppStorageLifecycle::RebuildableCache,
            cache_dir_retention: AppStorageRetention::Rebuildable,
            cache_dir_exists: true,
            preserves_cache_dir: true,
            mode: CachePurgeMode::DryRun,
            deleted: false,
            summary: CachePurgeSummary {
                total_entries: 1,
                would_delete_entries: 0,
                deleted_entries: 0,
                skipped_entries: 1,
                failed_entries: 0,
                files: 0,
                directories: 0,
                estimated_bytes: 0,
                reclaimed_bytes: 0,
                issue_matrix: vec![CachePurgeIssueSummary {
                    status: CachePurgeEntryStatus::Skipped,
                    reason_code: CachePurgeEntryReason::SymlinkSkipped,
                    entries: 1,
                    estimated_bytes: 0,
                }],
            },
            entries: vec![CachePurgeEntry {
                path: PathBuf::from("cache/link"),
                kind: CachePurgeEntryKind::Symlink,
                status: CachePurgeEntryStatus::Skipped,
                estimated_bytes: 0,
                files: 0,
                directories: 0,
                reason: Some("symlink entries are skipped".to_string()),
                reason_code: Some(CachePurgeEntryReason::SymlinkSkipped),
            }],
        };

        let rendered = render_cache_purge_report(&report);

        assert!(rendered.contains("Issue matrix:"));
        assert!(rendered.contains("- skipped symlink-skipped: 1 entry, 0 (0 B)"));
        assert!(rendered.contains("Run with --yes to purge these rebuildable cache entries."));
    }

    #[test]
    fn render_cache_purge_report_omits_empty_issue_matrix() {
        let report = CachePurgeReport {
            cache_dir: PathBuf::from("cache"),
            cache_dir_lifecycle: AppStorageLifecycle::RebuildableCache,
            cache_dir_retention: AppStorageRetention::Rebuildable,
            cache_dir_exists: false,
            preserves_cache_dir: true,
            mode: CachePurgeMode::DryRun,
            deleted: false,
            summary: CachePurgeSummary::default(),
            entries: Vec::new(),
        };

        let rendered = render_cache_purge_report(&report);

        assert!(!rendered.contains("Issue matrix:"));
        assert!(rendered.contains("No cache entries found."));
    }
}

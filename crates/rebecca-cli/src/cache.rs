use std::fmt::Write as _;

use anyhow::Result;
use rebecca_core::cache::{CachePurgeMode, CachePurgeReport, purge_app_cache};
use rebecca_core::config::load_app_paths;

use crate::cache_view::CachePurgeProjection;
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
    let projection = CachePurgeProjection::new(report);
    let mut output = String::new();
    writeln!(output, "Rebecca cache: {}", projection.cache_dir.display()).unwrap();
    writeln!(output, "Mode: {}", projection.mode_label).unwrap();
    writeln!(
        output,
        "Lifecycle: {} ({})",
        projection.lifecycle_label, projection.retention_label
    )
    .unwrap();
    writeln!(
        output,
        "Cache directory exists: {}",
        projection.cache_dir_exists_label
    )
    .unwrap();
    writeln!(
        output,
        "Preserves cache directory: {}",
        projection.preserves_cache_dir_label
    )
    .unwrap();
    writeln!(
        output,
        "Entries: {}, files: {}, directories: {}",
        projection.summary.total_entries, projection.summary.files, projection.summary.directories
    )
    .unwrap();
    writeln!(
        output,
        "Entry status: {} would delete, {} deleted, {} skipped, {} failed",
        projection.summary.would_delete_entries,
        projection.summary.deleted_entries,
        projection.summary.skipped_entries,
        projection.summary.failed_entries
    )
    .unwrap();
    writeln!(
        output,
        "Estimated bytes: {} ({})",
        projection.summary.estimated_bytes,
        format_bytes(projection.summary.estimated_bytes)
    )
    .unwrap();
    writeln!(
        output,
        "Reclaimed bytes: {} ({})",
        projection.summary.reclaimed_bytes,
        format_bytes(projection.summary.reclaimed_bytes)
    )
    .unwrap();

    if !projection.issue_matrix().is_empty() {
        writeln!(output, "Issue matrix:").unwrap();
        for issue in projection.issue_matrix() {
            writeln!(
                output,
                "- {} {}: {}, {} ({})",
                issue.status_label,
                issue.reason_label,
                issue.entries_label,
                issue.estimated_bytes,
                format_bytes(issue.estimated_bytes)
            )
            .unwrap();
        }
    }

    if projection.is_empty() {
        writeln!(output, "No cache entries found.").unwrap();
        return output;
    }

    if projection.show_delete_hint() {
        writeln!(
            output,
            "Run with --yes to purge these rebuildable cache entries."
        )
        .unwrap();
    }

    writeln!(output, "Cache entries:").unwrap();
    for entry in projection.entries() {
        writeln!(
            output,
            "- {}: {} ({}; {} file(s), {} dir(s)){}",
            entry.status_label,
            entry.path.display(),
            format_bytes(entry.estimated_bytes),
            entry.files,
            entry.directories,
            entry.reason_suffix
        )
        .unwrap();
    }

    output
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

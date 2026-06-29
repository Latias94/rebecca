use std::path::Path;

use rebecca::core::cache::{CachePurgeEntry, CachePurgeMode, CachePurgeReport, CachePurgeSummary};

#[derive(Debug, Clone)]
pub(crate) struct CachePurgeProjection<'a> {
    pub(crate) cache_dir: &'a Path,
    pub(crate) mode_label: &'static str,
    pub(crate) lifecycle_label: &'static str,
    pub(crate) retention_label: &'static str,
    pub(crate) cache_dir_exists_label: &'static str,
    pub(crate) preserves_cache_dir_label: &'static str,
    pub(crate) summary: CachePurgeHumanSummary,
    issue_matrix: Vec<CachePurgeIssueRow>,
    entries: Vec<CachePurgeEntryRow<'a>>,
    show_delete_hint: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct CachePurgeHumanSummary {
    pub(crate) total_entries: usize,
    pub(crate) would_delete_entries: usize,
    pub(crate) deleted_entries: usize,
    pub(crate) skipped_entries: usize,
    pub(crate) failed_entries: usize,
    pub(crate) files: u64,
    pub(crate) directories: u64,
    pub(crate) estimated_bytes: u64,
    pub(crate) reclaimed_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CachePurgeIssueRow {
    pub(crate) status_label: &'static str,
    pub(crate) reason_label: &'static str,
    pub(crate) entries_label: String,
    pub(crate) estimated_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CachePurgeEntryRow<'a> {
    pub(crate) status_label: &'static str,
    pub(crate) path: &'a Path,
    pub(crate) estimated_bytes: u64,
    pub(crate) files: u64,
    pub(crate) directories: u64,
    pub(crate) reason_suffix: String,
}

impl<'a> CachePurgeProjection<'a> {
    pub(crate) fn new(report: &'a CachePurgeReport) -> Self {
        let issue_matrix = report
            .summary
            .issue_matrix
            .iter()
            .map(|issue| CachePurgeIssueRow {
                status_label: issue.status.label(),
                reason_label: issue.reason_code.label(),
                entries_label: format_count(issue.entries, "entry", "entries"),
                estimated_bytes: issue.estimated_bytes,
            })
            .collect();

        let entries = report
            .entries
            .iter()
            .map(CachePurgeEntryRow::from)
            .collect();

        Self {
            cache_dir: report.cache_dir.as_path(),
            mode_label: mode_label(report.mode),
            lifecycle_label: report.cache_dir_lifecycle.label(),
            retention_label: report.cache_dir_retention.label(),
            cache_dir_exists_label: yes_no(report.cache_dir_exists),
            preserves_cache_dir_label: yes_no(report.preserves_cache_dir),
            summary: CachePurgeHumanSummary::from(&report.summary),
            issue_matrix,
            entries,
            show_delete_hint: report.mode == CachePurgeMode::DryRun && !report.entries.is_empty(),
        }
    }

    pub(crate) fn issue_matrix(&self) -> &[CachePurgeIssueRow] {
        &self.issue_matrix
    }

    pub(crate) fn entries(&self) -> &[CachePurgeEntryRow<'a>] {
        &self.entries
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(crate) fn show_delete_hint(&self) -> bool {
        self.show_delete_hint
    }
}

impl From<&CachePurgeSummary> for CachePurgeHumanSummary {
    fn from(summary: &CachePurgeSummary) -> Self {
        Self {
            total_entries: summary.total_entries,
            would_delete_entries: summary.would_delete_entries,
            deleted_entries: summary.deleted_entries,
            skipped_entries: summary.skipped_entries,
            failed_entries: summary.failed_entries,
            files: summary.files,
            directories: summary.directories,
            estimated_bytes: summary.estimated_bytes,
            reclaimed_bytes: summary.reclaimed_bytes,
        }
    }
}

impl<'a> From<&'a CachePurgeEntry> for CachePurgeEntryRow<'a> {
    fn from(entry: &'a CachePurgeEntry) -> Self {
        Self {
            status_label: entry.status.label(),
            path: entry.path.as_path(),
            estimated_bytes: entry.estimated_bytes,
            files: entry.files,
            directories: entry.directories,
            reason_suffix: entry
                .reason
                .as_deref()
                .map(|reason| format!(" - {reason}"))
                .unwrap_or_default(),
        }
    }
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

    use rebecca::core::cache::{
        CachePurgeEntryKind, CachePurgeEntryReason, CachePurgeEntryStatus, CachePurgeIssueSummary,
    };
    use rebecca::core::config::{AppStorageLifecycle, AppStorageRetention};

    use super::*;

    fn cache_report(mode: CachePurgeMode, entries: Vec<CachePurgeEntry>) -> CachePurgeReport {
        let issue_matrix = if entries.is_empty() {
            Vec::new()
        } else {
            vec![CachePurgeIssueSummary {
                status: CachePurgeEntryStatus::Skipped,
                reason_code: CachePurgeEntryReason::SymlinkSkipped,
                entries: 1,
                estimated_bytes: 0,
            }]
        };

        let summary = CachePurgeSummary {
            total_entries: entries.len(),
            would_delete_entries: entries
                .iter()
                .filter(|entry| entry.status == CachePurgeEntryStatus::WouldDelete)
                .count(),
            deleted_entries: entries
                .iter()
                .filter(|entry| entry.status == CachePurgeEntryStatus::Deleted)
                .count(),
            skipped_entries: entries
                .iter()
                .filter(|entry| entry.status == CachePurgeEntryStatus::Skipped)
                .count(),
            failed_entries: entries
                .iter()
                .filter(|entry| entry.status == CachePurgeEntryStatus::Failed)
                .count(),
            files: entries.iter().map(|entry| entry.files).sum(),
            directories: entries.iter().map(|entry| entry.directories).sum(),
            estimated_bytes: entries.iter().map(|entry| entry.estimated_bytes).sum(),
            reclaimed_bytes: entries
                .iter()
                .filter(|entry| entry.status == CachePurgeEntryStatus::Deleted)
                .map(|entry| entry.estimated_bytes)
                .sum(),
            issue_matrix,
        };

        CachePurgeReport {
            cache_dir: PathBuf::from("cache"),
            cache_dir_lifecycle: AppStorageLifecycle::RebuildableCache,
            cache_dir_retention: AppStorageRetention::Rebuildable,
            cache_dir_exists: true,
            preserves_cache_dir: true,
            mode,
            deleted: mode == CachePurgeMode::Delete,
            summary,
            entries,
        }
    }

    fn skipped_symlink_entry() -> CachePurgeEntry {
        CachePurgeEntry {
            path: PathBuf::from("cache/link"),
            kind: CachePurgeEntryKind::Symlink,
            status: CachePurgeEntryStatus::Skipped,
            estimated_bytes: 0,
            files: 0,
            directories: 0,
            reason: Some("symlink entries are skipped".to_string()),
            reason_code: Some(CachePurgeEntryReason::SymlinkSkipped),
        }
    }

    #[test]
    fn projection_prepares_issue_rows_and_entry_reason_suffixes() {
        let report = cache_report(CachePurgeMode::DryRun, vec![skipped_symlink_entry()]);

        let projection = CachePurgeProjection::new(&report);

        assert_eq!(projection.mode_label, "dry-run");
        assert_eq!(projection.cache_dir_exists_label, "yes");
        assert_eq!(projection.summary.total_entries, 1);
        assert_eq!(projection.issue_matrix().len(), 1);
        assert_eq!(projection.issue_matrix()[0].status_label, "skipped");
        assert_eq!(projection.issue_matrix()[0].reason_label, "symlink-skipped");
        assert_eq!(projection.issue_matrix()[0].entries_label, "1 entry");
        assert_eq!(projection.entries().len(), 1);
        assert_eq!(
            projection.entries()[0].reason_suffix,
            " - symlink entries are skipped"
        );
    }

    #[test]
    fn projection_only_prompts_for_confirmation_on_non_empty_dry_runs() {
        let dry_run = cache_report(CachePurgeMode::DryRun, vec![skipped_symlink_entry()]);
        let delete = cache_report(CachePurgeMode::Delete, vec![skipped_symlink_entry()]);
        let empty = cache_report(CachePurgeMode::DryRun, Vec::new());

        assert!(CachePurgeProjection::new(&dry_run).show_delete_hint());
        assert!(!CachePurgeProjection::new(&delete).show_delete_hint());
        assert!(!CachePurgeProjection::new(&empty).show_delete_hint());
    }
}

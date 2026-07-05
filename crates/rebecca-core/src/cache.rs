use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::TargetStatus;
use crate::config::{AppPaths, AppStorageLifecycle, AppStorageRetention};
use crate::error::{RebeccaError, Result};
use crate::execution::{ExecutionActionReport, ExecutionReport};
use crate::path_overlap::paths_overlap;
use crate::plan::CleanupTargetDeletionStyle;
use crate::scan::ScanEngine;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CachePurgeMode {
    DryRun,
    RecoverableDelete,
    PermanentDelete,
}

impl CachePurgeMode {
    fn deletes(self) -> bool {
        matches!(self, Self::RecoverableDelete | Self::PermanentDelete)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachePurgeOutcome {
    pub reclaimed_bytes: u64,
    pub pending_reclaim_bytes: u64,
    pub note: Option<String>,
}

impl CachePurgeOutcome {
    pub fn recoverable(estimated_bytes: u64, note: impl Into<String>) -> Self {
        Self {
            reclaimed_bytes: 0,
            pending_reclaim_bytes: estimated_bytes,
            note: Some(note.into()),
        }
    }

    pub fn permanent(estimated_bytes: u64) -> Self {
        Self {
            reclaimed_bytes: estimated_bytes,
            pending_reclaim_bytes: 0,
            note: None,
        }
    }
}

pub trait CachePurgeBackend {
    fn purge(
        &self,
        path: &Path,
        kind: CachePurgeEntryKind,
        estimated_bytes: u64,
    ) -> Result<CachePurgeOutcome>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct PermanentCachePurgeBackend;

impl CachePurgeBackend for PermanentCachePurgeBackend {
    fn purge(
        &self,
        path: &Path,
        kind: CachePurgeEntryKind,
        estimated_bytes: u64,
    ) -> Result<CachePurgeOutcome> {
        permanently_delete_cache_entry(path, kind)?;
        Ok(CachePurgeOutcome::permanent(estimated_bytes))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachePurgeReport {
    pub cache_dir: PathBuf,
    pub cache_dir_lifecycle: AppStorageLifecycle,
    pub cache_dir_retention: AppStorageRetention,
    pub cache_dir_exists: bool,
    pub preserves_cache_dir: bool,
    pub mode: CachePurgeMode,
    pub deleted: bool,
    pub summary: CachePurgeSummary,
    pub entries: Vec<CachePurgeEntry>,
    pub execution_report: ExecutionReport,
}

impl CachePurgeReport {
    fn empty(cache_dir: PathBuf, mode: CachePurgeMode) -> Self {
        Self {
            cache_dir,
            cache_dir_lifecycle: AppStorageLifecycle::RebuildableCache,
            cache_dir_retention: AppStorageRetention::Rebuildable,
            cache_dir_exists: false,
            preserves_cache_dir: true,
            mode,
            deleted: mode.deletes(),
            summary: CachePurgeSummary::default(),
            entries: Vec::new(),
            execution_report: cache_execution_report(mode, &[]),
        }
    }

    fn recompute_summary(&mut self) {
        self.summary = CachePurgeSummary::from_entries(&self.entries);
        self.execution_report = cache_execution_report(self.mode, &self.entries);
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachePurgeSummary {
    pub total_entries: usize,
    pub would_delete_entries: usize,
    pub deleted_entries: usize,
    pub skipped_entries: usize,
    pub failed_entries: usize,
    pub files: u64,
    pub directories: u64,
    pub estimated_bytes: u64,
    pub reclaimed_bytes: u64,
    pub pending_reclaim_bytes: u64,
    pub recoverably_deleted_entries: usize,
    pub permanently_deleted_entries: usize,
    pub issue_matrix: Vec<CachePurgeIssueSummary>,
}

impl CachePurgeSummary {
    fn from_entries(entries: &[CachePurgeEntry]) -> Self {
        let mut summary = Self::default();
        let mut issue_matrix = BTreeMap::new();
        for entry in entries {
            summary.total_entries += 1;
            summary.files = summary.files.saturating_add(entry.files);
            summary.directories = summary.directories.saturating_add(entry.directories);
            summary.estimated_bytes = summary
                .estimated_bytes
                .saturating_add(entry.estimated_bytes);

            match entry.status {
                CachePurgeEntryStatus::WouldDelete => summary.would_delete_entries += 1,
                CachePurgeEntryStatus::RecoverablyDeleted => {
                    summary.deleted_entries += 1;
                    summary.recoverably_deleted_entries += 1;
                    summary.pending_reclaim_bytes = summary
                        .pending_reclaim_bytes
                        .saturating_add(entry.pending_reclaim_bytes);
                }
                CachePurgeEntryStatus::PermanentlyDeleted => {
                    summary.deleted_entries += 1;
                    summary.permanently_deleted_entries += 1;
                    summary.reclaimed_bytes = summary
                        .reclaimed_bytes
                        .saturating_add(entry.reclaimed_bytes);
                }
                CachePurgeEntryStatus::Skipped => summary.skipped_entries += 1,
                CachePurgeEntryStatus::Failed => summary.failed_entries += 1,
            }

            if let Some(reason_code) = entry.reason_code {
                let bucket = issue_matrix
                    .entry((entry.status, reason_code))
                    .or_insert_with(|| CachePurgeIssueSummary {
                        status: entry.status,
                        reason_code,
                        entries: 0,
                        estimated_bytes: 0,
                    });
                bucket.entries = bucket.entries.saturating_add(1);
                bucket.estimated_bytes =
                    bucket.estimated_bytes.saturating_add(entry.estimated_bytes);
            }
        }

        summary.issue_matrix = issue_matrix.into_values().collect();
        summary
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachePurgeIssueSummary {
    pub status: CachePurgeEntryStatus,
    pub reason_code: CachePurgeEntryReason,
    pub entries: usize,
    pub estimated_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachePurgeEntry {
    pub path: PathBuf,
    pub kind: CachePurgeEntryKind,
    pub status: CachePurgeEntryStatus,
    pub estimated_bytes: u64,
    pub reclaimed_bytes: u64,
    pub pending_reclaim_bytes: u64,
    pub files: u64,
    pub directories: u64,
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<CachePurgeEntryReason>,
}

impl CachePurgeEntry {
    fn failed(
        path: PathBuf,
        kind: CachePurgeEntryKind,
        reason_code: CachePurgeEntryReason,
        reason: String,
    ) -> Self {
        Self {
            path,
            kind,
            status: CachePurgeEntryStatus::Failed,
            estimated_bytes: 0,
            reclaimed_bytes: 0,
            pending_reclaim_bytes: 0,
            files: 0,
            directories: 0,
            reason: Some(reason),
            reason_code: Some(reason_code),
        }
    }

    fn skipped(
        path: PathBuf,
        kind: CachePurgeEntryKind,
        reason_code: CachePurgeEntryReason,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            path,
            kind,
            status: CachePurgeEntryStatus::Skipped,
            estimated_bytes: 0,
            reclaimed_bytes: 0,
            pending_reclaim_bytes: 0,
            files: 0,
            directories: 0,
            reason: Some(reason.into()),
            reason_code: Some(reason_code),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CachePurgeEntryKind {
    File,
    Directory,
    Symlink,
    Other,
}

impl CachePurgeEntryKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Directory => "directory",
            Self::Symlink => "symlink",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CachePurgeEntryStatus {
    WouldDelete,
    RecoverablyDeleted,
    PermanentlyDeleted,
    Skipped,
    Failed,
}

impl CachePurgeEntryStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::WouldDelete => "would-delete",
            Self::RecoverablyDeleted => "recoverably-deleted",
            Self::PermanentlyDeleted => "permanently-deleted",
            Self::Skipped => "skipped",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CachePurgeEntryReason {
    SymlinkSkipped,
    UnsupportedEntryType,
    MetadataReadFailed,
    MeasurementFailed,
    DeleteFailed,
}

impl CachePurgeEntryReason {
    pub fn label(self) -> &'static str {
        match self {
            Self::SymlinkSkipped => "symlink-skipped",
            Self::UnsupportedEntryType => "unsupported-entry-type",
            Self::MetadataReadFailed => "metadata-read-failed",
            Self::MeasurementFailed => "measurement-failed",
            Self::DeleteFailed => "delete-failed",
        }
    }
}

pub fn purge_app_cache(paths: &AppPaths, mode: CachePurgeMode) -> Result<CachePurgeReport> {
    match mode {
        CachePurgeMode::DryRun | CachePurgeMode::PermanentDelete => {
            purge_app_cache_with_backend(paths, mode, &PermanentCachePurgeBackend)
        }
        CachePurgeMode::RecoverableDelete => Err(RebeccaError::PlatformUnavailable(
            "recoverable cache purge requires a platform cache purge backend".to_string(),
        )),
    }
}

pub fn purge_app_cache_with_backend<B: CachePurgeBackend>(
    paths: &AppPaths,
    mode: CachePurgeMode,
    backend: &B,
) -> Result<CachePurgeReport> {
    validate_cache_purge_boundary(paths)?;

    let mut report = CachePurgeReport::empty(paths.cache_dir.clone(), mode);
    let cache_metadata = match std::fs::symlink_metadata(&paths.cache_dir) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(report),
        Err(err) => {
            return Err(RebeccaError::ExecutionFailed(format!(
                "cache purge failed at {}: {}",
                paths.cache_dir.display(),
                err
            )));
        }
    };

    if cache_metadata.file_type().is_symlink() {
        return Err(RebeccaError::SafetyBlocked(format!(
            "cache purge refused to operate on symlinked cache directory {}",
            paths.cache_dir.display()
        )));
    }
    if !cache_metadata.is_dir() {
        return Err(RebeccaError::SafetyBlocked(format!(
            "cache purge requires a directory at {}",
            paths.cache_dir.display()
        )));
    }
    report.cache_dir_exists = true;

    let read_dir = std::fs::read_dir(&paths.cache_dir).map_err(|err| {
        RebeccaError::ExecutionFailed(format!(
            "cache purge failed to read {}: {}",
            paths.cache_dir.display(),
            err
        ))
    })?;

    let mut entries = Vec::new();
    for entry in read_dir {
        let entry = entry.map_err(|err| {
            RebeccaError::ExecutionFailed(format!(
                "cache purge failed while reading {}: {}",
                paths.cache_dir.display(),
                err
            ))
        })?;
        entries.push(purge_cache_entry(entry.path(), mode, backend));
    }

    entries.sort_by(|left, right| left.path.cmp(&right.path));
    report.entries = entries;
    report.recompute_summary();
    Ok(report)
}

fn validate_cache_purge_boundary(paths: &AppPaths) -> Result<()> {
    if !paths.cache_dir.is_absolute() {
        return Err(RebeccaError::SafetyBlocked(format!(
            "cache purge requires an absolute cache directory, got {}",
            paths.cache_dir.display()
        )));
    }

    for entry in paths
        .storage_entries()
        .into_iter()
        .filter(|entry| entry.retention == AppStorageRetention::Preserve)
    {
        if paths_overlap(&paths.cache_dir, &entry.path) {
            return Err(RebeccaError::SafetyBlocked(format!(
                "cache directory {} overlaps preserved {} at {}",
                paths.cache_dir.display(),
                entry.id.label(),
                entry.path.display()
            )));
        }
    }

    Ok(())
}

fn purge_cache_entry<B: CachePurgeBackend>(
    path: PathBuf,
    mode: CachePurgeMode,
    backend: &B,
) -> CachePurgeEntry {
    let metadata = match std::fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(err) => {
            return CachePurgeEntry::failed(
                path,
                CachePurgeEntryKind::Other,
                CachePurgeEntryReason::MetadataReadFailed,
                err.to_string(),
            );
        }
    };

    let file_type = metadata.file_type();
    if file_type.is_symlink() {
        return CachePurgeEntry::skipped(
            path,
            CachePurgeEntryKind::Symlink,
            CachePurgeEntryReason::SymlinkSkipped,
            "symlink entries are skipped",
        );
    }

    let kind = if metadata.is_file() {
        CachePurgeEntryKind::File
    } else if metadata.is_dir() {
        CachePurgeEntryKind::Directory
    } else {
        return CachePurgeEntry::skipped(
            path,
            CachePurgeEntryKind::Other,
            CachePurgeEntryReason::UnsupportedEntryType,
            "unsupported cache entry type",
        );
    };

    let report = match ScanEngine::new().measure_path(&path) {
        Ok(report) => report,
        Err(err) => {
            return CachePurgeEntry::failed(
                path,
                kind,
                CachePurgeEntryReason::MeasurementFailed,
                err.to_string(),
            );
        }
    };

    let (status, reason, reclaimed_bytes, pending_reclaim_bytes) = if mode.deletes() {
        match backend.purge(&path, kind, report.bytes_scanned) {
            Ok(outcome) => {
                let status = match mode {
                    CachePurgeMode::DryRun => unreachable!("dry-run does not call backend"),
                    CachePurgeMode::RecoverableDelete => CachePurgeEntryStatus::RecoverablyDeleted,
                    CachePurgeMode::PermanentDelete => CachePurgeEntryStatus::PermanentlyDeleted,
                };
                (
                    status,
                    outcome.note,
                    outcome.reclaimed_bytes,
                    outcome.pending_reclaim_bytes,
                )
            }
            Err(err) => {
                return CachePurgeEntry {
                    path,
                    kind,
                    status: CachePurgeEntryStatus::Failed,
                    estimated_bytes: report.bytes_scanned,
                    reclaimed_bytes: 0,
                    pending_reclaim_bytes: 0,
                    files: report.files_scanned,
                    directories: report.directories_scanned,
                    reason: Some(err.to_string()),
                    reason_code: Some(CachePurgeEntryReason::DeleteFailed),
                };
            }
        }
    } else {
        (CachePurgeEntryStatus::WouldDelete, None, 0, 0)
    };

    CachePurgeEntry {
        path,
        kind,
        status,
        estimated_bytes: report.bytes_scanned,
        reclaimed_bytes,
        pending_reclaim_bytes,
        files: report.files_scanned,
        directories: report.directories_scanned,
        reason,
        reason_code: None,
    }
}

fn cache_execution_report(mode: CachePurgeMode, entries: &[CachePurgeEntry]) -> ExecutionReport {
    let actions = entries
        .iter()
        .enumerate()
        .map(|(target_index, entry)| ExecutionActionReport {
            target_index,
            rule_id: "rebecca.cache-purge".to_string(),
            path: entry.path.clone(),
            deletion_style: CleanupTargetDeletionStyle::DeleteWholePath,
            estimated_bytes: entry.estimated_bytes,
            status: cache_entry_execution_status(entry.status),
            reason: entry.reason.clone(),
            reason_code: entry
                .reason_code
                .map(|reason_code| reason_code.label().to_string()),
            attempted_paths: cache_entry_attempted_paths(entry),
            confirmed_reclaimed_bytes: entry.reclaimed_bytes,
            pending_reclaim_bytes: entry.pending_reclaim_bytes,
        })
        .collect();

    ExecutionReport::from_actions_with_dry_run(actions, mode == CachePurgeMode::DryRun)
}

fn cache_entry_execution_status(status: CachePurgeEntryStatus) -> TargetStatus {
    match status {
        CachePurgeEntryStatus::WouldDelete => TargetStatus::Allowed,
        CachePurgeEntryStatus::RecoverablyDeleted | CachePurgeEntryStatus::PermanentlyDeleted => {
            TargetStatus::Completed
        }
        CachePurgeEntryStatus::Skipped => TargetStatus::Skipped,
        CachePurgeEntryStatus::Failed => TargetStatus::Failed,
    }
}

fn cache_entry_attempted_paths(entry: &CachePurgeEntry) -> Vec<PathBuf> {
    match entry.status {
        CachePurgeEntryStatus::RecoverablyDeleted
        | CachePurgeEntryStatus::PermanentlyDeleted
        | CachePurgeEntryStatus::Failed => vec![entry.path.clone()],
        CachePurgeEntryStatus::WouldDelete | CachePurgeEntryStatus::Skipped => Vec::new(),
    }
}

fn permanently_delete_cache_entry(path: &Path, kind: CachePurgeEntryKind) -> std::io::Result<()> {
    match kind {
        CachePurgeEntryKind::File => std::fs::remove_file(path),
        CachePurgeEntryKind::Directory => std::fs::remove_dir_all(path),
        CachePurgeEntryKind::Symlink | CachePurgeEntryKind::Other => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CachePurgeBackend, CachePurgeEntry, CachePurgeEntryKind, CachePurgeEntryReason,
        CachePurgeEntryStatus, CachePurgeIssueSummary, CachePurgeMode, CachePurgeOutcome,
        CachePurgeSummary, cache_execution_report, permanently_delete_cache_entry, purge_app_cache,
        purge_app_cache_with_backend,
    };
    use crate::config::{AppPaths, AppStorageLifecycle, AppStorageRetention};
    use crate::error::Result;

    #[derive(Debug, Default, Clone, Copy)]
    struct RecoverableTestBackend;

    impl CachePurgeBackend for RecoverableTestBackend {
        fn purge(
            &self,
            path: &std::path::Path,
            kind: CachePurgeEntryKind,
            estimated_bytes: u64,
        ) -> Result<CachePurgeOutcome> {
            permanently_delete_cache_entry(path, kind)?;
            Ok(CachePurgeOutcome::recoverable(
                estimated_bytes,
                "moved to test recovery",
            ))
        }
    }

    fn app_paths(temp: &tempfile::TempDir) -> AppPaths {
        AppPaths {
            config_dir: temp.path().join("config"),
            config_file: temp.path().join("config").join("config.toml"),
            state_dir: temp.path().join("state"),
            cache_dir: temp.path().join("cache"),
            history_file: temp.path().join("state").join("history.jsonl"),
        }
    }

    #[test]
    fn cache_purge_dry_run_reports_direct_contents_without_deleting() {
        let temp = tempfile::tempdir().unwrap();
        let paths = app_paths(&temp);
        std::fs::create_dir_all(paths.cache_dir.join("nested")).unwrap();
        std::fs::write(paths.cache_dir.join("file.bin"), b"abc").unwrap();
        std::fs::write(paths.cache_dir.join("nested").join("nested.bin"), b"de").unwrap();

        let report = purge_app_cache(&paths, CachePurgeMode::DryRun).unwrap();

        assert!(!report.deleted);
        assert_eq!(report.mode, CachePurgeMode::DryRun);
        assert_eq!(
            report.cache_dir_lifecycle,
            AppStorageLifecycle::RebuildableCache
        );
        assert_eq!(report.cache_dir_retention, AppStorageRetention::Rebuildable);
        assert!(report.cache_dir_exists);
        assert!(report.preserves_cache_dir);
        assert_eq!(report.summary.total_entries, 2);
        assert_eq!(report.summary.would_delete_entries, 2);
        assert_eq!(report.summary.deleted_entries, 0);
        assert_eq!(report.summary.files, 2);
        assert_eq!(report.summary.directories, 1);
        assert_eq!(report.summary.estimated_bytes, 5);
        assert_eq!(report.summary.reclaimed_bytes, 0);
        assert_eq!(report.summary.pending_reclaim_bytes, 0);
        assert_eq!(report.summary.recoverably_deleted_entries, 0);
        assert_eq!(report.summary.permanently_deleted_entries, 0);
        assert!(report.summary.issue_matrix.is_empty());
        assert!(report.execution_report.dry_run);
        assert_eq!(report.execution_report.summary.total_actions, 2);
        assert_eq!(report.execution_report.summary.estimated_bytes, 5);
        assert!(
            report
                .execution_report
                .actions
                .iter()
                .all(|action| action.status == crate::TargetStatus::Allowed)
        );
        assert!(paths.cache_dir.join("file.bin").exists());
        assert!(paths.cache_dir.join("nested").join("nested.bin").exists());
        assert!(
            report
                .entries
                .iter()
                .all(|entry| entry.status == CachePurgeEntryStatus::WouldDelete)
        );
    }

    #[test]
    fn cache_purge_recoverable_delete_removes_contents_and_reports_pending_reclaim() {
        let temp = tempfile::tempdir().unwrap();
        let paths = app_paths(&temp);
        std::fs::create_dir_all(paths.cache_dir.join("nested")).unwrap();
        std::fs::write(paths.cache_dir.join("file.bin"), b"abc").unwrap();
        std::fs::write(paths.cache_dir.join("nested").join("nested.bin"), b"de").unwrap();

        let report = purge_app_cache_with_backend(
            &paths,
            CachePurgeMode::RecoverableDelete,
            &RecoverableTestBackend,
        )
        .unwrap();

        assert!(report.deleted);
        assert_eq!(report.mode, CachePurgeMode::RecoverableDelete);
        assert_eq!(report.summary.total_entries, 2);
        assert_eq!(report.summary.deleted_entries, 2);
        assert_eq!(report.summary.recoverably_deleted_entries, 2);
        assert_eq!(report.summary.permanently_deleted_entries, 0);
        assert_eq!(report.summary.reclaimed_bytes, 0);
        assert_eq!(report.summary.pending_reclaim_bytes, 5);
        assert!(report.summary.issue_matrix.is_empty());
        assert!(!report.execution_report.dry_run);
        assert_eq!(report.execution_report.summary.completed_actions, 2);
        assert_eq!(report.execution_report.summary.pending_reclaim_bytes, 5);
        assert!(paths.cache_dir.exists());
        assert_eq!(std::fs::read_dir(&paths.cache_dir).unwrap().count(), 0);
        assert!(report.entries.iter().all(|entry| {
            entry.status == CachePurgeEntryStatus::RecoverablyDeleted
                && entry.reason.as_deref() == Some("moved to test recovery")
        }));
    }

    #[test]
    fn cache_purge_permanent_delete_removes_direct_contents_but_keeps_cache_dir() {
        let temp = tempfile::tempdir().unwrap();
        let paths = app_paths(&temp);
        std::fs::create_dir_all(paths.cache_dir.join("nested")).unwrap();
        std::fs::write(paths.cache_dir.join("file.bin"), b"abc").unwrap();
        std::fs::write(paths.cache_dir.join("nested").join("nested.bin"), b"de").unwrap();

        let report = purge_app_cache(&paths, CachePurgeMode::PermanentDelete).unwrap();

        assert!(report.deleted);
        assert_eq!(report.mode, CachePurgeMode::PermanentDelete);
        assert_eq!(report.summary.total_entries, 2);
        assert_eq!(report.summary.deleted_entries, 2);
        assert_eq!(report.summary.recoverably_deleted_entries, 0);
        assert_eq!(report.summary.permanently_deleted_entries, 2);
        assert_eq!(report.summary.reclaimed_bytes, 5);
        assert_eq!(report.summary.pending_reclaim_bytes, 0);
        assert!(report.summary.issue_matrix.is_empty());
        assert_eq!(report.execution_report.summary.completed_actions, 2);
        assert_eq!(report.execution_report.summary.confirmed_reclaimed_bytes, 5);
        assert!(paths.cache_dir.exists());
        assert_eq!(std::fs::read_dir(&paths.cache_dir).unwrap().count(), 0);
        assert!(
            report
                .entries
                .iter()
                .all(|entry| entry.status == CachePurgeEntryStatus::PermanentlyDeleted)
        );
    }

    #[test]
    fn cache_purge_missing_cache_dir_is_empty_success() {
        let temp = tempfile::tempdir().unwrap();
        let paths = app_paths(&temp);

        let report = purge_app_cache(&paths, CachePurgeMode::DryRun).unwrap();

        assert!(!report.cache_dir_exists);
        assert!(report.preserves_cache_dir);
        assert_eq!(report.summary.total_entries, 0);
        assert!(report.summary.issue_matrix.is_empty());
        assert!(report.entries.is_empty());
    }

    #[test]
    fn cache_purge_rejects_overlap_with_preserved_state() {
        let temp = tempfile::tempdir().unwrap();
        let mut paths = app_paths(&temp);
        paths.cache_dir = paths.state_dir.clone();
        std::fs::create_dir_all(&paths.cache_dir).unwrap();

        let err = purge_app_cache(&paths, CachePurgeMode::DryRun).unwrap_err();

        assert!(err.to_string().contains("overlaps preserved"));
    }

    #[test]
    fn cache_purge_rejects_relative_cache_dir() {
        let temp = tempfile::tempdir().unwrap();
        let mut paths = app_paths(&temp);
        paths.cache_dir = std::path::PathBuf::from("cache");

        let err = purge_app_cache(&paths, CachePurgeMode::DryRun).unwrap_err();

        assert!(err.to_string().contains("absolute cache directory"));
    }

    #[test]
    fn cache_purge_summary_groups_issue_matrix_by_status_and_reason() {
        let entries = vec![
            CachePurgeEntry {
                path: std::path::PathBuf::from("cache/a"),
                kind: CachePurgeEntryKind::Symlink,
                status: CachePurgeEntryStatus::Skipped,
                estimated_bytes: 0,
                reclaimed_bytes: 0,
                pending_reclaim_bytes: 0,
                files: 0,
                directories: 0,
                reason: Some("symlink entries are skipped".to_string()),
                reason_code: Some(CachePurgeEntryReason::SymlinkSkipped),
            },
            CachePurgeEntry {
                path: std::path::PathBuf::from("cache/b"),
                kind: CachePurgeEntryKind::Other,
                status: CachePurgeEntryStatus::Skipped,
                estimated_bytes: 0,
                reclaimed_bytes: 0,
                pending_reclaim_bytes: 0,
                files: 0,
                directories: 0,
                reason: Some("unsupported cache entry type".to_string()),
                reason_code: Some(CachePurgeEntryReason::UnsupportedEntryType),
            },
            CachePurgeEntry {
                path: std::path::PathBuf::from("cache/c"),
                kind: CachePurgeEntryKind::File,
                status: CachePurgeEntryStatus::Failed,
                estimated_bytes: 12,
                reclaimed_bytes: 0,
                pending_reclaim_bytes: 0,
                files: 1,
                directories: 0,
                reason: Some("first failure".to_string()),
                reason_code: Some(CachePurgeEntryReason::MeasurementFailed),
            },
            CachePurgeEntry {
                path: std::path::PathBuf::from("cache/d"),
                kind: CachePurgeEntryKind::Directory,
                status: CachePurgeEntryStatus::Failed,
                estimated_bytes: 8,
                reclaimed_bytes: 0,
                pending_reclaim_bytes: 0,
                files: 2,
                directories: 1,
                reason: Some("second failure".to_string()),
                reason_code: Some(CachePurgeEntryReason::MeasurementFailed),
            },
            CachePurgeEntry {
                path: std::path::PathBuf::from("cache/e"),
                kind: CachePurgeEntryKind::Directory,
                status: CachePurgeEntryStatus::Failed,
                estimated_bytes: 7,
                reclaimed_bytes: 0,
                pending_reclaim_bytes: 0,
                files: 3,
                directories: 1,
                reason: Some("delete failed".to_string()),
                reason_code: Some(CachePurgeEntryReason::DeleteFailed),
            },
        ];

        let summary = CachePurgeSummary::from_entries(&entries);

        assert_eq!(summary.total_entries, 5);
        assert_eq!(summary.skipped_entries, 2);
        assert_eq!(summary.failed_entries, 3);
        assert_eq!(
            summary.issue_matrix,
            vec![
                CachePurgeIssueSummary {
                    status: CachePurgeEntryStatus::Skipped,
                    reason_code: CachePurgeEntryReason::SymlinkSkipped,
                    entries: 1,
                    estimated_bytes: 0,
                },
                CachePurgeIssueSummary {
                    status: CachePurgeEntryStatus::Skipped,
                    reason_code: CachePurgeEntryReason::UnsupportedEntryType,
                    entries: 1,
                    estimated_bytes: 0,
                },
                CachePurgeIssueSummary {
                    status: CachePurgeEntryStatus::Failed,
                    reason_code: CachePurgeEntryReason::MeasurementFailed,
                    entries: 2,
                    estimated_bytes: 20,
                },
                CachePurgeIssueSummary {
                    status: CachePurgeEntryStatus::Failed,
                    reason_code: CachePurgeEntryReason::DeleteFailed,
                    entries: 1,
                    estimated_bytes: 7,
                },
            ]
        );

        let execution_report = cache_execution_report(CachePurgeMode::DryRun, &entries);
        assert_eq!(
            execution_report.actions[0].reason_code.as_deref(),
            Some("symlink-skipped")
        );
        assert_eq!(
            execution_report.actions[2].reason_code.as_deref(),
            Some("measurement-failed")
        );
    }
}

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::{AppPaths, AppStorageLifecycle, AppStorageRetention};
use crate::error::{RebeccaError, Result};
use crate::scan::measure_path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CachePurgeMode {
    DryRun,
    Delete,
}

impl CachePurgeMode {
    fn deletes(self) -> bool {
        matches!(self, Self::Delete)
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
        }
    }

    fn recompute_summary(&mut self) {
        self.summary = CachePurgeSummary::from_entries(&self.entries);
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
                CachePurgeEntryStatus::Deleted => {
                    summary.deleted_entries += 1;
                    summary.reclaimed_bytes = summary
                        .reclaimed_bytes
                        .saturating_add(entry.estimated_bytes);
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
    Deleted,
    Skipped,
    Failed,
}

impl CachePurgeEntryStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::WouldDelete => "would-delete",
            Self::Deleted => "deleted",
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
        entries.push(purge_cache_entry(entry.path(), mode));
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

fn purge_cache_entry(path: PathBuf, mode: CachePurgeMode) -> CachePurgeEntry {
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

    let report = match measure_path(&path) {
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

    let status = if mode.deletes() {
        match delete_cache_entry(&path, kind) {
            Ok(()) => CachePurgeEntryStatus::Deleted,
            Err(err) => {
                return CachePurgeEntry {
                    path,
                    kind,
                    status: CachePurgeEntryStatus::Failed,
                    estimated_bytes: report.bytes_scanned,
                    files: report.files_scanned,
                    directories: report.directories_scanned,
                    reason: Some(err.to_string()),
                    reason_code: Some(CachePurgeEntryReason::DeleteFailed),
                };
            }
        }
    } else {
        CachePurgeEntryStatus::WouldDelete
    };

    CachePurgeEntry {
        path,
        kind,
        status,
        estimated_bytes: report.bytes_scanned,
        files: report.files_scanned,
        directories: report.directories_scanned,
        reason: None,
        reason_code: None,
    }
}

fn delete_cache_entry(path: &Path, kind: CachePurgeEntryKind) -> std::io::Result<()> {
    match kind {
        CachePurgeEntryKind::File => std::fs::remove_file(path),
        CachePurgeEntryKind::Directory => std::fs::remove_dir_all(path),
        CachePurgeEntryKind::Symlink | CachePurgeEntryKind::Other => Ok(()),
    }
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    same_or_child_path(left, right) || same_or_child_path(right, left)
}

fn same_or_child_path(parent: &Path, child: &Path) -> bool {
    let parent = comparable_components(parent);
    let child = comparable_components(child);
    !parent.is_empty() && child.len() >= parent.len() && child.starts_with(&parent)
}

fn comparable_components(path: &Path) -> Vec<String> {
    path.components()
        .filter_map(|component| match component {
            Component::Prefix(prefix) => Some(prefix.as_os_str().to_string_lossy().into_owned()),
            Component::RootDir => Some(std::path::MAIN_SEPARATOR.to_string()),
            Component::Normal(value) => Some(value.to_string_lossy().into_owned()),
            Component::ParentDir => Some("..".to_string()),
            Component::CurDir => None,
        })
        .map(|component| {
            if cfg!(windows) {
                component.to_ascii_lowercase()
            } else {
                component
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        CachePurgeEntry, CachePurgeEntryKind, CachePurgeEntryReason, CachePurgeEntryStatus,
        CachePurgeIssueSummary, CachePurgeMode, CachePurgeSummary, purge_app_cache,
    };
    use crate::config::{AppPaths, AppStorageLifecycle, AppStorageRetention};

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
        assert!(report.summary.issue_matrix.is_empty());
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
    fn cache_purge_delete_removes_direct_contents_but_keeps_cache_dir() {
        let temp = tempfile::tempdir().unwrap();
        let paths = app_paths(&temp);
        std::fs::create_dir_all(paths.cache_dir.join("nested")).unwrap();
        std::fs::write(paths.cache_dir.join("file.bin"), b"abc").unwrap();
        std::fs::write(paths.cache_dir.join("nested").join("nested.bin"), b"de").unwrap();

        let report = purge_app_cache(&paths, CachePurgeMode::Delete).unwrap();

        assert!(report.deleted);
        assert_eq!(report.mode, CachePurgeMode::Delete);
        assert_eq!(report.summary.total_entries, 2);
        assert_eq!(report.summary.deleted_entries, 2);
        assert_eq!(report.summary.reclaimed_bytes, 5);
        assert!(report.summary.issue_matrix.is_empty());
        assert!(paths.cache_dir.exists());
        assert_eq!(std::fs::read_dir(&paths.cache_dir).unwrap().count(), 0);
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
    }
}

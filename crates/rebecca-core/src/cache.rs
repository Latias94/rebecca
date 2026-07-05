use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::TargetStatus;
use crate::config::{AppPaths, AppStorageLifecycle, AppStorageRetention};
use crate::error::{RebeccaError, Result};
use crate::execution::{ExecutionActionReport, ExecutionReport};
use crate::path_overlap::paths_overlap;
use crate::plan::{CleanupTargetDeletionStyle, CleanupTargetIssueReason};
use crate::scan::ScanEngine;
use crate::scan_cache::{
    ScanCacheMiss, ScanCachePathSnapshot, ScanCachePolicy, ScanCacheRecord, ScanCacheStore,
};

const NTFS_VOLUME_INDEX_CACHE_DIR: &str = "ntfs-volume-index";

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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CacheNamespace {
    #[default]
    All,
    ScanCache,
    NtfsVolumeIndex,
}

impl CacheNamespace {
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::ScanCache => "scan-cache",
            Self::NtfsVolumeIndex => "ntfs-volume-index",
        }
    }

    fn includes_scan_cache(self) -> bool {
        matches!(self, Self::All | Self::ScanCache)
    }

    fn includes_ntfs_volume_index(self) -> bool {
        matches!(self, Self::All | Self::NtfsVolumeIndex)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheInventory {
    pub cache_dir: PathBuf,
    pub namespace: CacheNamespace,
    pub summary: CacheInventorySummary,
    pub entries: Vec<CacheInventoryEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<CacheInventoryDiagnostic>,
}

impl CacheInventory {
    fn new(cache_dir: PathBuf, namespace: CacheNamespace) -> Self {
        Self {
            cache_dir,
            namespace,
            summary: CacheInventorySummary::default(),
            entries: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    fn recompute_summary(&mut self) {
        self.summary =
            CacheInventorySummary::from_entries_and_diagnostics(&self.entries, &self.diagnostics);
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheInventorySummary {
    pub total_entries: usize,
    pub scan_cache_records: usize,
    pub ntfs_volume_index_manifests: usize,
    pub ntfs_volume_index_payloads: usize,
    pub bytes: u64,
    pub valid_entries: usize,
    pub stale_entries: usize,
    pub corrupt_entries: usize,
    pub missing_payloads: usize,
    pub prunable_entries: usize,
    pub diagnostics: usize,
}

impl CacheInventorySummary {
    fn from_entries_and_diagnostics(
        entries: &[CacheInventoryEntry],
        diagnostics: &[CacheInventoryDiagnostic],
    ) -> Self {
        let mut summary = Self {
            total_entries: entries.len(),
            diagnostics: diagnostics.len(),
            ..Self::default()
        };

        for entry in entries {
            summary.bytes = summary.bytes.saturating_add(entry.bytes);
            if entry.prunable {
                summary.prunable_entries += 1;
            }

            match entry.namespace {
                CacheNamespace::ScanCache => summary.scan_cache_records += 1,
                CacheNamespace::NtfsVolumeIndex => match entry.kind {
                    CacheInventoryEntryKind::NtfsVolumeIndexManifest => {
                        summary.ntfs_volume_index_manifests += 1;
                    }
                    CacheInventoryEntryKind::NtfsVolumeIndexPayload => {
                        summary.ntfs_volume_index_payloads += 1;
                    }
                    CacheInventoryEntryKind::ScanCacheRecord => {}
                },
                CacheNamespace::All => {}
            }

            match entry.status {
                CacheInventoryEntryStatus::Valid => summary.valid_entries += 1,
                CacheInventoryEntryStatus::Stale => summary.stale_entries += 1,
                CacheInventoryEntryStatus::Corrupt | CacheInventoryEntryStatus::Unreadable => {
                    summary.corrupt_entries += 1;
                }
                CacheInventoryEntryStatus::MissingPayload => summary.missing_payloads += 1,
                CacheInventoryEntryStatus::Payload | CacheInventoryEntryStatus::Unknown => {}
            }
        }

        summary
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheInventoryEntry {
    pub namespace: CacheNamespace,
    pub kind: CacheInventoryEntryKind,
    pub status: CacheInventoryEntryStatus,
    pub prunable: bool,
    pub absolute_path: PathBuf,
    pub display_path: PathBuf,
    pub bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record_root: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record_root_display: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend_source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub age_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload_file_name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CacheInventoryEntryKind {
    ScanCacheRecord,
    NtfsVolumeIndexManifest,
    NtfsVolumeIndexPayload,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CacheInventoryEntryStatus {
    Valid,
    Stale,
    Corrupt,
    Unreadable,
    MissingPayload,
    Payload,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheInventoryDiagnostic {
    pub namespace: CacheNamespace,
    pub absolute_path: PathBuf,
    pub display_path: PathBuf,
    pub reason_code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheDoctorReport {
    pub inventory: CacheInventory,
    pub recommendations: Vec<CacheDoctorRecommendation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheDoctorRecommendation {
    pub severity: CacheDoctorSeverity,
    pub reason_code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggested_command: Option<Vec<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CacheDoctorSeverity {
    Info,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachePruneReport {
    pub cache_dir: PathBuf,
    pub namespace: CacheNamespace,
    pub stale_only: bool,
    pub limit: Option<usize>,
    pub dry_run: bool,
    pub selected_entries: Vec<CacheInventoryEntry>,
    pub execution_report: ExecutionReport,
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

pub fn inspect_app_cache(
    paths: &AppPaths,
    namespace: CacheNamespace,
    policy: ScanCachePolicy,
) -> CacheInventory {
    let mut inventory = CacheInventory::new(paths.cache_dir.clone(), namespace);
    if namespace.includes_scan_cache() {
        inspect_scan_cache(paths, policy, &mut inventory);
    }
    if namespace.includes_ntfs_volume_index() {
        inspect_ntfs_volume_index_cache(paths, &mut inventory);
    }
    inventory.entries.sort_by(|left, right| {
        left.namespace
            .label()
            .cmp(right.namespace.label())
            .then_with(|| left.absolute_path.cmp(&right.absolute_path))
    });
    inventory.recompute_summary();
    inventory
}

pub fn doctor_app_cache(paths: &AppPaths, policy: ScanCachePolicy) -> CacheDoctorReport {
    let inventory = inspect_app_cache(paths, CacheNamespace::All, policy);
    let mut recommendations = Vec::new();

    if inventory.summary.total_entries == 0 && inventory.summary.diagnostics == 0 {
        recommendations.push(CacheDoctorRecommendation {
            severity: CacheDoctorSeverity::Info,
            reason_code: "cache-empty".to_string(),
            message: "No Rebecca cache records were found.".to_string(),
            suggested_command: None,
        });
    }

    if inventory.summary.prunable_entries > 0 {
        recommendations.push(CacheDoctorRecommendation {
            severity: CacheDoctorSeverity::Warning,
            reason_code: "prunable-cache-records".to_string(),
            message: format!(
                "{} cache record(s) can be pruned safely.",
                inventory.summary.prunable_entries
            ),
            suggested_command: Some(vec![
                "cache".to_string(),
                "prune".to_string(),
                "--stale-only".to_string(),
            ]),
        });
    }

    if inventory.summary.missing_payloads > 0 {
        recommendations.push(CacheDoctorRecommendation {
            severity: CacheDoctorSeverity::Warning,
            reason_code: "ntfs-payload-mismatch".to_string(),
            message: format!(
                "{} NTFS volume-index manifest(s) reference missing payload files.",
                inventory.summary.missing_payloads
            ),
            suggested_command: Some(vec![
                "cache".to_string(),
                "prune".to_string(),
                "--namespace".to_string(),
                "ntfs-volume-index".to_string(),
                "--stale-only".to_string(),
            ]),
        });
    }

    CacheDoctorReport {
        inventory,
        recommendations,
    }
}

pub fn prune_app_cache_inventory(
    paths: &AppPaths,
    namespace: CacheNamespace,
    policy: ScanCachePolicy,
    stale_only: bool,
    limit: Option<usize>,
    dry_run: bool,
) -> CachePruneReport {
    let inventory = inspect_app_cache(paths, namespace, policy);
    let mut selected_entries = inventory
        .entries
        .into_iter()
        .filter(|entry| !stale_only || entry.prunable)
        .collect::<Vec<_>>();
    if let Some(limit) = limit {
        selected_entries.truncate(limit);
    }

    let actions = selected_entries
        .iter()
        .enumerate()
        .map(|(index, entry)| prune_entry_action(index, entry, dry_run))
        .collect::<Vec<_>>();
    let execution_report = ExecutionReport::from_actions_with_dry_run(actions, dry_run);

    CachePruneReport {
        cache_dir: paths.cache_dir.clone(),
        namespace,
        stale_only,
        limit,
        dry_run,
        selected_entries,
        execution_report,
    }
}

fn inspect_scan_cache(paths: &AppPaths, policy: ScanCachePolicy, inventory: &mut CacheInventory) {
    let store = ScanCacheStore::from_app_paths(paths);
    let entries = match fs::read_dir(store.root_dir()) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return,
        Err(err) => {
            inventory.diagnostics.push(cache_diagnostic(
                CacheNamespace::ScanCache,
                paths,
                store.root_dir(),
                "scan-cache-read-failed",
                format!("failed to read scan cache directory: {err}"),
            ));
            return;
        }
    };

    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                inventory.diagnostics.push(cache_diagnostic(
                    CacheNamespace::ScanCache,
                    paths,
                    store.root_dir(),
                    "scan-cache-entry-read-failed",
                    format!("failed to read scan cache entry: {err}"),
                ));
                continue;
            }
        };
        let path = entry.path();
        if !is_json_file(&path) {
            continue;
        }
        inventory
            .entries
            .push(inspect_scan_cache_file(paths, &store, &path, policy));
    }
}

fn inspect_scan_cache_file(
    paths: &AppPaths,
    store: &ScanCacheStore,
    path: &Path,
    policy: ScanCachePolicy,
) -> CacheInventoryEntry {
    let bytes = file_len(path);
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(_) => {
            return cache_entry(
                CacheEntryFacts::new(
                    CacheNamespace::ScanCache,
                    CacheInventoryEntryKind::ScanCacheRecord,
                    CacheInventoryEntryStatus::Unreadable,
                    true,
                    Some("scan-cache-unreadable"),
                ),
                paths,
                path,
                bytes,
            );
        }
    };

    let record: ScanCacheRecord = match serde_json::from_str(&raw) {
        Ok(record) => record,
        Err(_) => {
            return cache_entry(
                CacheEntryFacts::new(
                    CacheNamespace::ScanCache,
                    CacheInventoryEntryKind::ScanCacheRecord,
                    CacheInventoryEntryStatus::Corrupt,
                    true,
                    Some("scan-cache-corrupt"),
                ),
                paths,
                path,
                bytes,
            );
        }
    };

    let mut entry = cache_entry(
        CacheEntryFacts::new(
            CacheNamespace::ScanCache,
            CacheInventoryEntryKind::ScanCacheRecord,
            CacheInventoryEntryStatus::Valid,
            false,
            None,
        ),
        paths,
        path,
        bytes,
    );
    entry.record_root = Some(record.root.clone());
    entry.record_root_display = Some(display_path(paths, &record.root));
    entry.backend = Some(record.backend.label().to_string());
    entry.backend_source = record.backend_source.clone();
    entry.confidence = Some(record.confidence.label().to_string());
    entry.age_seconds = Some(unix_now().saturating_sub(record.written_at_unix_seconds));

    if store.cache_file_for(&record.root) != path {
        entry.status = CacheInventoryEntryStatus::Stale;
        entry.prunable = true;
        entry.reason_code = Some("scan-cache-path-hash-mismatch".to_string());
        return entry;
    }

    let snapshot = match ScanCachePathSnapshot::read_from_path(&record.root) {
        Ok(snapshot) => snapshot,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            entry.status = CacheInventoryEntryStatus::Stale;
            entry.prunable = true;
            entry.reason_code = Some(ScanCacheMiss::Missing.label().to_string());
            return entry;
        }
        Err(_) => {
            entry.status = CacheInventoryEntryStatus::Unknown;
            entry.reason_code = Some(ScanCacheMiss::MetadataUnavailable.label().to_string());
            return entry;
        }
    };

    if let Some(reason) = record.miss_reason(&record.root, &snapshot, policy, unix_now()) {
        entry.status = CacheInventoryEntryStatus::Stale;
        entry.prunable = reason.should_prune_cache_file();
        entry.reason_code = Some(reason.label().to_string());
    }

    entry
}

fn inspect_ntfs_volume_index_cache(paths: &AppPaths, inventory: &mut CacheInventory) {
    let root = paths.cache_dir.join(NTFS_VOLUME_INDEX_CACHE_DIR);
    let entries = match fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return,
        Err(err) => {
            inventory.diagnostics.push(cache_diagnostic(
                CacheNamespace::NtfsVolumeIndex,
                paths,
                &root,
                "ntfs-volume-index-read-failed",
                format!("failed to read NTFS volume-index cache directory: {err}"),
            ));
            return;
        }
    };

    let mut payload_files = Vec::new();
    let mut referenced_payloads = BTreeMap::new();
    let mut manifest_entries = Vec::new();

    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                inventory.diagnostics.push(cache_diagnostic(
                    CacheNamespace::NtfsVolumeIndex,
                    paths,
                    &root,
                    "ntfs-volume-index-entry-read-failed",
                    format!("failed to read NTFS volume-index cache entry: {err}"),
                ));
                continue;
            }
        };
        let path = entry.path();
        if !is_json_file(&path) {
            continue;
        }
        if is_ntfs_payload_file(&path) {
            payload_files.push(path);
        } else {
            let entry = inspect_ntfs_manifest(paths, &root, &path, &mut referenced_payloads);
            manifest_entries.push(entry);
        }
    }

    inventory.entries.extend(manifest_entries);
    for payload in payload_files {
        inventory.entries.push(inspect_ntfs_payload(
            paths,
            &payload,
            referenced_payloads.contains_key(&file_name_string(&payload)),
        ));
    }
}

fn inspect_ntfs_manifest(
    paths: &AppPaths,
    root: &Path,
    path: &Path,
    referenced_payloads: &mut BTreeMap<String, ()>,
) -> CacheInventoryEntry {
    let bytes = file_len(path);
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(_) => {
            return cache_entry(
                CacheEntryFacts::new(
                    CacheNamespace::NtfsVolumeIndex,
                    CacheInventoryEntryKind::NtfsVolumeIndexManifest,
                    CacheInventoryEntryStatus::Unreadable,
                    true,
                    Some("ntfs-volume-index-manifest-unreadable"),
                ),
                paths,
                path,
                bytes,
            );
        }
    };

    let value: Value = match serde_json::from_str(&raw) {
        Ok(value) => value,
        Err(_) => {
            return cache_entry(
                CacheEntryFacts::new(
                    CacheNamespace::NtfsVolumeIndex,
                    CacheInventoryEntryKind::NtfsVolumeIndexManifest,
                    CacheInventoryEntryStatus::Corrupt,
                    true,
                    Some("ntfs-volume-index-manifest-corrupt"),
                ),
                paths,
                path,
                bytes,
            );
        }
    };

    let payload_file_name = value
        .pointer("/payload/file_name")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    if let Some(file_name) = &payload_file_name {
        referenced_payloads.insert(file_name.clone(), ());
    }

    let mut entry = cache_entry(
        CacheEntryFacts::new(
            CacheNamespace::NtfsVolumeIndex,
            CacheInventoryEntryKind::NtfsVolumeIndexManifest,
            CacheInventoryEntryStatus::Valid,
            false,
            None,
        ),
        paths,
        path,
        bytes,
    );
    entry.payload_file_name = payload_file_name.clone();
    entry.backend_source = value
        .get("source_label")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    entry.age_seconds = value
        .get("created_at_unix_seconds")
        .and_then(Value::as_u64)
        .map(|created| unix_now().saturating_sub(created));

    if let Some(file_name) = payload_file_name {
        let payload_path = root.join(&file_name);
        if !payload_path.exists() {
            entry.status = CacheInventoryEntryStatus::MissingPayload;
            entry.prunable = true;
            entry.reason_code = Some("ntfs-volume-index-payload-missing".to_string());
        }
    }

    entry
}

fn inspect_ntfs_payload(paths: &AppPaths, path: &Path, referenced: bool) -> CacheInventoryEntry {
    let status = if referenced {
        CacheInventoryEntryStatus::Payload
    } else {
        CacheInventoryEntryStatus::Stale
    };
    cache_entry(
        CacheEntryFacts::new(
            CacheNamespace::NtfsVolumeIndex,
            CacheInventoryEntryKind::NtfsVolumeIndexPayload,
            status,
            !referenced,
            (!referenced).then_some("ntfs-volume-index-orphan-payload"),
        ),
        paths,
        path,
        file_len(path),
    )
}

#[derive(Debug, Clone)]
struct CacheEntryFacts {
    namespace: CacheNamespace,
    kind: CacheInventoryEntryKind,
    status: CacheInventoryEntryStatus,
    prunable: bool,
    reason_code: Option<String>,
}

impl CacheEntryFacts {
    fn new(
        namespace: CacheNamespace,
        kind: CacheInventoryEntryKind,
        status: CacheInventoryEntryStatus,
        prunable: bool,
        reason_code: Option<&str>,
    ) -> Self {
        Self {
            namespace,
            kind,
            status,
            prunable,
            reason_code: reason_code.map(ToOwned::to_owned),
        }
    }
}

fn cache_entry(
    facts: CacheEntryFacts,
    paths: &AppPaths,
    path: &Path,
    bytes: u64,
) -> CacheInventoryEntry {
    CacheInventoryEntry {
        namespace: facts.namespace,
        kind: facts.kind,
        status: facts.status,
        prunable: facts.prunable,
        absolute_path: path.to_path_buf(),
        display_path: display_path(paths, path),
        bytes,
        record_root: None,
        record_root_display: None,
        backend: None,
        backend_source: None,
        confidence: None,
        age_seconds: None,
        reason_code: facts.reason_code,
        payload_file_name: None,
    }
}

fn cache_diagnostic(
    namespace: CacheNamespace,
    paths: &AppPaths,
    path: &Path,
    reason_code: impl Into<String>,
    message: impl Into<String>,
) -> CacheInventoryDiagnostic {
    CacheInventoryDiagnostic {
        namespace,
        absolute_path: path.to_path_buf(),
        display_path: display_path(paths, path),
        reason_code: reason_code.into(),
        message: message.into(),
    }
}

fn prune_entry_action(
    target_index: usize,
    entry: &CacheInventoryEntry,
    dry_run: bool,
) -> ExecutionActionReport {
    if dry_run {
        return ExecutionActionReport {
            target_index,
            rule_id: "rebecca.cache-prune".to_string(),
            path: entry.absolute_path.clone(),
            deletion_style: CleanupTargetDeletionStyle::DeleteWholePath,
            estimated_bytes: entry.bytes,
            status: TargetStatus::Allowed,
            reason: entry.reason_code.clone(),
            reason_code: None,
            attempted_paths: Vec::new(),
            confirmed_reclaimed_bytes: 0,
            pending_reclaim_bytes: 0,
        };
    }

    match fs::remove_file(&entry.absolute_path) {
        Ok(()) => ExecutionActionReport {
            target_index,
            rule_id: "rebecca.cache-prune".to_string(),
            path: entry.absolute_path.clone(),
            deletion_style: CleanupTargetDeletionStyle::DeleteWholePath,
            estimated_bytes: entry.bytes,
            status: TargetStatus::Completed,
            reason: entry.reason_code.clone(),
            reason_code: None,
            attempted_paths: vec![entry.absolute_path.clone()],
            confirmed_reclaimed_bytes: entry.bytes,
            pending_reclaim_bytes: 0,
        },
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => ExecutionActionReport {
            target_index,
            rule_id: "rebecca.cache-prune".to_string(),
            path: entry.absolute_path.clone(),
            deletion_style: CleanupTargetDeletionStyle::DeleteWholePath,
            estimated_bytes: entry.bytes,
            status: TargetStatus::Skipped,
            reason: Some("path does not exist".to_string()),
            reason_code: Some(
                CleanupTargetIssueReason::ExecutionTargetMissing
                    .label()
                    .to_string(),
            ),
            attempted_paths: Vec::new(),
            confirmed_reclaimed_bytes: 0,
            pending_reclaim_bytes: 0,
        },
        Err(err) => ExecutionActionReport {
            target_index,
            rule_id: "rebecca.cache-prune".to_string(),
            path: entry.absolute_path.clone(),
            deletion_style: CleanupTargetDeletionStyle::DeleteWholePath,
            estimated_bytes: entry.bytes,
            status: TargetStatus::Failed,
            reason: Some(err.to_string()),
            reason_code: Some(
                CleanupTargetIssueReason::ExecutionFailed
                    .label()
                    .to_string(),
            ),
            attempted_paths: vec![entry.absolute_path.clone()],
            confirmed_reclaimed_bytes: 0,
            pending_reclaim_bytes: 0,
        },
    }
}

fn is_json_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
}

fn is_ntfs_payload_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".index.json"))
}

fn file_name_string(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn file_len(path: &Path) -> u64 {
    fs::metadata(path)
        .map(|metadata| metadata.len())
        .unwrap_or(0)
}

fn display_path(paths: &AppPaths, path: &Path) -> PathBuf {
    path.strip_prefix(&paths.cache_dir)
        .map(Path::to_path_buf)
        .unwrap_or_else(|_| path.to_path_buf())
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
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
        CacheNamespace, CachePurgeBackend, CachePurgeEntry, CachePurgeEntryKind,
        CachePurgeEntryReason, CachePurgeEntryStatus, CachePurgeIssueSummary, CachePurgeMode,
        CachePurgeOutcome, CachePurgeSummary, cache_execution_report, doctor_app_cache,
        inspect_app_cache, permanently_delete_cache_entry, prune_app_cache_inventory,
        purge_app_cache, purge_app_cache_with_backend,
    };
    use crate::config::{AppPaths, AppStorageLifecycle, AppStorageRetention};
    use crate::error::Result;
    use crate::scan::ScanReport;
    use crate::scan_cache::{ScanCachePolicy, ScanCacheStore};

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
    fn cache_inspect_reports_scan_cache_records() {
        let temp = tempfile::tempdir().unwrap();
        let paths = app_paths(&temp);
        let root = temp.path().join("project").join("target");
        std::fs::create_dir_all(&root).unwrap();
        let store = ScanCacheStore::from_app_paths(&paths);
        store
            .store(
                &root,
                ScanReport {
                    bytes_scanned: 42,
                    files_scanned: 2,
                    directories_scanned: 1,
                },
            )
            .unwrap();

        let inventory = inspect_app_cache(
            &paths,
            CacheNamespace::ScanCache,
            ScanCachePolicy::default(),
        );

        assert_eq!(inventory.summary.total_entries, 1);
        assert_eq!(inventory.summary.scan_cache_records, 1);
        assert_eq!(inventory.summary.valid_entries, 1);
        assert_eq!(inventory.entries[0].namespace, CacheNamespace::ScanCache);
        assert_eq!(
            inventory.entries[0].record_root.as_deref(),
            Some(root.as_path())
        );
        assert_eq!(
            inventory.entries[0].backend.as_deref(),
            Some("portable-recursive")
        );
        assert_eq!(inventory.entries[0].confidence.as_deref(), Some("exact"));
    }

    #[test]
    fn cache_inspect_reports_corrupt_scan_cache_record_as_prunable() {
        let temp = tempfile::tempdir().unwrap();
        let paths = app_paths(&temp);
        let corrupt = paths.cache_dir.join("scan").join("bad.json");
        std::fs::create_dir_all(corrupt.parent().unwrap()).unwrap();
        std::fs::write(&corrupt, "{").unwrap();

        let inventory = inspect_app_cache(
            &paths,
            CacheNamespace::ScanCache,
            ScanCachePolicy::default(),
        );

        assert_eq!(inventory.summary.total_entries, 1);
        assert_eq!(inventory.summary.corrupt_entries, 1);
        assert_eq!(inventory.summary.prunable_entries, 1);
        assert_eq!(
            inventory.entries[0].reason_code.as_deref(),
            Some("scan-cache-corrupt")
        );
    }

    #[test]
    fn cache_prune_deletes_selected_stale_records_and_reports_execution() {
        let temp = tempfile::tempdir().unwrap();
        let paths = app_paths(&temp);
        let corrupt = paths.cache_dir.join("scan").join("bad.json");
        std::fs::create_dir_all(corrupt.parent().unwrap()).unwrap();
        std::fs::write(&corrupt, "{").unwrap();

        let report = prune_app_cache_inventory(
            &paths,
            CacheNamespace::ScanCache,
            ScanCachePolicy::default(),
            true,
            Some(1),
            false,
        );

        assert_eq!(report.selected_entries.len(), 1);
        assert_eq!(report.execution_report.summary.completed_actions, 1);
        assert_eq!(report.execution_report.summary.confirmed_reclaimed_bytes, 1);
        assert!(!corrupt.exists());
    }

    #[test]
    fn cache_doctor_recommends_stale_prune_when_records_are_prunable() {
        let temp = tempfile::tempdir().unwrap();
        let paths = app_paths(&temp);
        let corrupt = paths.cache_dir.join("scan").join("bad.json");
        std::fs::create_dir_all(corrupt.parent().unwrap()).unwrap();
        std::fs::write(&corrupt, "{").unwrap();

        let report = doctor_app_cache(&paths, ScanCachePolicy::default());

        assert_eq!(report.inventory.summary.prunable_entries, 1);
        assert!(
            report
                .recommendations
                .iter()
                .any(|recommendation| recommendation.reason_code == "prunable-cache-records")
        );
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

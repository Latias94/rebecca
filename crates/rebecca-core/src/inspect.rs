use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{RebeccaError, Result};
use crate::plan::EstimateSource;
use crate::safety::is_reparse_like;
use crate::scan::{ScanCancellationToken, ScanEngine, ScanReport};
use crate::scan_cache::{ScanCacheLookup, ScanCachePolicy, ScanCacheStore};

pub const DEFAULT_SPACE_INSIGHT_TOP_LIMIT: usize = 10;

#[derive(Debug, Clone)]
pub struct SpaceInsightRequest {
    pub roots: Vec<PathBuf>,
    pub top_limit: usize,
    pub scan_cache: Option<SpaceInsightScanCache>,
}

impl SpaceInsightRequest {
    pub fn new(roots: Vec<PathBuf>) -> Self {
        Self {
            roots,
            top_limit: DEFAULT_SPACE_INSIGHT_TOP_LIMIT,
            scan_cache: None,
        }
    }

    pub fn with_top_limit(mut self, top_limit: usize) -> Self {
        self.top_limit = top_limit;
        self
    }

    pub fn with_scan_cache(mut self, scan_cache: SpaceInsightScanCache) -> Self {
        self.scan_cache = Some(scan_cache);
        self
    }
}

#[derive(Debug, Clone)]
pub struct SpaceInsightScanCache {
    pub store: ScanCacheStore,
    pub policy: ScanCachePolicy,
}

impl SpaceInsightScanCache {
    pub fn new(store: ScanCacheStore, policy: ScanCachePolicy) -> Self {
        Self { store, policy }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpaceInsightReport {
    pub roots: Vec<SpaceInsightRoot>,
    pub totals: SpaceInsightMetrics,
    pub top_entries: Vec<SpaceInsightEntry>,
    pub diagnostics: Vec<SpaceInsightDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpaceInsightRoot {
    pub path: PathBuf,
    pub status: SpaceInsightRootStatus,
    pub metrics: SpaceInsightMetrics,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SpaceInsightRootStatus {
    Scanned,
    Skipped,
}

impl SpaceInsightRootStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Scanned => "scanned",
            Self::Skipped => "skipped",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpaceInsightMetrics {
    pub estimated_bytes: u64,
    pub files: u64,
    pub directories: u64,
}

impl SpaceInsightMetrics {
    fn add_report(&mut self, report: ScanReport) {
        self.estimated_bytes = self.estimated_bytes.saturating_add(report.bytes_scanned);
        self.files = self.files.saturating_add(report.files_scanned);
        self.directories = self.directories.saturating_add(report.directories_scanned);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpaceInsightEntry {
    pub path: PathBuf,
    pub root: PathBuf,
    pub kind: SpaceInsightEntryKind,
    pub estimated_bytes: u64,
    pub files: u64,
    pub directories: u64,
    pub estimate_source: EstimateSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SpaceInsightEntryKind {
    File,
    Directory,
    Other,
}

impl SpaceInsightEntryKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Directory => "directory",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SpaceInsightDiagnostic {
    pub kind: SpaceInsightDiagnosticKind,
    pub path: PathBuf,
    pub detail: String,
}

impl SpaceInsightDiagnostic {
    pub fn new(kind: SpaceInsightDiagnosticKind, path: PathBuf, detail: impl Into<String>) -> Self {
        Self {
            kind,
            path,
            detail: detail.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SpaceInsightDiagnosticKind {
    RootMissing,
    RootMetadataReadSkipped,
    RootNotDirectory,
    ReparsePointSkipped,
    DirectoryReadSkipped,
    DirectoryEntryReadSkipped,
    MetadataReadSkipped,
    ScanFailed,
}

impl SpaceInsightDiagnosticKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::RootMissing => "root-missing",
            Self::RootMetadataReadSkipped => "root-metadata-read-skipped",
            Self::RootNotDirectory => "root-not-directory",
            Self::ReparsePointSkipped => "reparse-point-skipped",
            Self::DirectoryReadSkipped => "directory-read-skipped",
            Self::DirectoryEntryReadSkipped => "directory-entry-read-skipped",
            Self::MetadataReadSkipped => "metadata-read-skipped",
            Self::ScanFailed => "scan-failed",
        }
    }
}

pub fn inspect_space(
    request: &SpaceInsightRequest,
    cancellation: &ScanCancellationToken,
) -> Result<SpaceInsightReport> {
    let mut report = SpaceInsightReport::default();
    let mut top_entries = SpaceInsightTopEntries::new(request.top_limit);

    for root in &request.roots {
        check_cancelled(cancellation)?;
        inspect_root(root, request, cancellation, &mut report, &mut top_entries)?;
    }

    report.top_entries = top_entries.into_sorted_entries();
    report.diagnostics.sort();
    Ok(report)
}

fn inspect_root(
    root: &Path,
    request: &SpaceInsightRequest,
    cancellation: &ScanCancellationToken,
    report: &mut SpaceInsightReport,
    top_entries: &mut SpaceInsightTopEntries,
) -> Result<()> {
    let metadata = match std::fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            push_root_skip(
                report,
                root,
                SpaceInsightDiagnosticKind::RootMissing,
                "space inspection root does not exist",
            );
            return Ok(());
        }
        Err(err) => {
            push_root_skip(
                report,
                root,
                SpaceInsightDiagnosticKind::RootMetadataReadSkipped,
                format!("space inspection root metadata could not be read: {err}"),
            );
            return Ok(());
        }
    };

    if !metadata.is_dir() {
        push_root_skip(
            report,
            root,
            SpaceInsightDiagnosticKind::RootNotDirectory,
            "space inspection root is not a directory",
        );
        return Ok(());
    }

    if is_reparse_like(&metadata) {
        push_root_skip(
            report,
            root,
            SpaceInsightDiagnosticKind::ReparsePointSkipped,
            "space inspection root is a symlink or reparse point",
        );
        return Ok(());
    }

    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(err) => {
            push_root_skip(
                report,
                root,
                SpaceInsightDiagnosticKind::DirectoryReadSkipped,
                format!("space inspection root could not be read: {err}"),
            );
            return Ok(());
        }
    };

    let mut root_metrics = SpaceInsightMetrics::default();
    let mut entry_paths = Vec::new();
    for entry in entries {
        check_cancelled(cancellation)?;
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                report.diagnostics.push(SpaceInsightDiagnostic::new(
                    SpaceInsightDiagnosticKind::DirectoryEntryReadSkipped,
                    root.to_path_buf(),
                    format!("space inspection directory entry could not be read: {err}"),
                ));
                continue;
            }
        };
        entry_paths.push(entry.path());
    }
    entry_paths.sort();

    for path in entry_paths {
        check_cancelled(cancellation)?;
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(err) => {
                report.diagnostics.push(SpaceInsightDiagnostic::new(
                    SpaceInsightDiagnosticKind::MetadataReadSkipped,
                    path,
                    format!("space inspection entry metadata could not be read: {err}"),
                ));
                continue;
            }
        };
        if is_reparse_like(&metadata) {
            report.diagnostics.push(SpaceInsightDiagnostic::new(
                SpaceInsightDiagnosticKind::ReparsePointSkipped,
                path,
                "space inspection entry is a symlink or reparse point",
            ));
            continue;
        }

        match inspect_entry(root, &path, metadata, request, cancellation) {
            Ok(entry) => {
                root_metrics.add_report(ScanReport {
                    bytes_scanned: entry.estimated_bytes,
                    files_scanned: entry.files,
                    directories_scanned: entry.directories,
                });
                top_entries.push(entry);
            }
            Err(err) => report.diagnostics.push(SpaceInsightDiagnostic::new(
                SpaceInsightDiagnosticKind::ScanFailed,
                path,
                err.to_string(),
            )),
        }
    }

    report.totals.estimated_bytes = report
        .totals
        .estimated_bytes
        .saturating_add(root_metrics.estimated_bytes);
    report.totals.files = report.totals.files.saturating_add(root_metrics.files);
    report.totals.directories = report
        .totals
        .directories
        .saturating_add(root_metrics.directories);
    report.roots.push(SpaceInsightRoot {
        path: root.to_path_buf(),
        status: SpaceInsightRootStatus::Scanned,
        metrics: root_metrics,
        reason: None,
    });
    Ok(())
}

#[derive(Debug, Default)]
struct SpaceInsightTopEntries {
    limit: usize,
    heap: BinaryHeap<Reverse<SpaceInsightTopCandidate>>,
    sequence: u64,
}

impl SpaceInsightTopEntries {
    fn new(limit: usize) -> Self {
        Self {
            limit,
            heap: BinaryHeap::with_capacity(limit),
            sequence: 0,
        }
    }

    fn push(&mut self, entry: SpaceInsightEntry) {
        if self.limit == 0 {
            return;
        }

        let candidate = SpaceInsightTopCandidate {
            rank: SpaceInsightTopRank::from_entry(&entry),
            sequence: self.sequence,
            entry,
        };
        self.sequence = self.sequence.saturating_add(1);

        if self.heap.len() < self.limit {
            self.heap.push(Reverse(candidate));
            return;
        }

        if self
            .heap
            .peek()
            .is_some_and(|current| candidate > current.0)
        {
            self.heap.pop();
            self.heap.push(Reverse(candidate));
        }
    }

    fn into_sorted_entries(self) -> Vec<SpaceInsightEntry> {
        let mut candidates = self
            .heap
            .into_iter()
            .map(|Reverse(candidate)| candidate)
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| right.cmp(left));
        candidates
            .into_iter()
            .map(|candidate| candidate.entry)
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SpaceInsightTopCandidate {
    rank: SpaceInsightTopRank,
    sequence: u64,
    entry: SpaceInsightEntry,
}

impl Ord for SpaceInsightTopCandidate {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.rank
            .cmp(&other.rank)
            .then_with(|| other.sequence.cmp(&self.sequence))
    }
}

impl PartialOrd for SpaceInsightTopCandidate {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct SpaceInsightTopRank {
    estimated_bytes: u64,
    files: u64,
    reverse_path: Reverse<PathBuf>,
}

impl SpaceInsightTopRank {
    fn from_entry(entry: &SpaceInsightEntry) -> Self {
        Self {
            estimated_bytes: entry.estimated_bytes,
            files: entry.files,
            reverse_path: Reverse(entry.path.clone()),
        }
    }
}

fn inspect_entry(
    root: &Path,
    path: &Path,
    metadata: std::fs::Metadata,
    request: &SpaceInsightRequest,
    cancellation: &ScanCancellationToken,
) -> Result<SpaceInsightEntry> {
    let kind = if metadata.is_file() {
        SpaceInsightEntryKind::File
    } else if metadata.is_dir() {
        SpaceInsightEntryKind::Directory
    } else {
        SpaceInsightEntryKind::Other
    };

    let (scan_report, estimate_source) = measure_entry(path, request, cancellation)?;
    Ok(SpaceInsightEntry {
        path: path.to_path_buf(),
        root: root.to_path_buf(),
        kind,
        estimated_bytes: scan_report.bytes_scanned,
        files: scan_report.files_scanned,
        directories: scan_report.directories_scanned,
        estimate_source,
    })
}

fn measure_entry(
    path: &Path,
    request: &SpaceInsightRequest,
    cancellation: &ScanCancellationToken,
) -> Result<(ScanReport, EstimateSource)> {
    if let Some(scan_cache) = &request.scan_cache {
        match scan_cache.store.load_with_policy(path, scan_cache.policy) {
            ScanCacheLookup::Hit(report) => return Ok((report, EstimateSource::ScanCache)),
            ScanCacheLookup::Miss(_) => {}
        }
    }

    let measured_scan = ScanEngine::new().measure_scan_with_progress(path, cancellation, |_| {})?;
    let scan_report = measured_scan.report;
    if let Some(scan_cache) = &request.scan_cache
        && let Err(err) =
            scan_cache
                .store
                .store_measured_scan_with_policy(path, measured_scan, scan_cache.policy)
    {
        tracing::debug!(
            path = %path.display(),
            error = %err,
            "inspect scan cache write skipped"
        );
    }

    Ok((scan_report, EstimateSource::FreshScan))
}

fn push_root_skip(
    report: &mut SpaceInsightReport,
    root: &Path,
    kind: SpaceInsightDiagnosticKind,
    detail: impl Into<String>,
) {
    let detail = detail.into();
    report.roots.push(SpaceInsightRoot {
        path: root.to_path_buf(),
        status: SpaceInsightRootStatus::Skipped,
        metrics: SpaceInsightMetrics::default(),
        reason: Some(detail.clone()),
    });
    report.diagnostics.push(SpaceInsightDiagnostic::new(
        kind,
        root.to_path_buf(),
        detail,
    ));
}

fn check_cancelled(cancellation: &ScanCancellationToken) -> Result<()> {
    if cancellation.is_cancelled() {
        return Err(RebeccaError::OperationCancelled(
            "space inspection was cancelled".to_string(),
        ));
    }

    Ok(())
}

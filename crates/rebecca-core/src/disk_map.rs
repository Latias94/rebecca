use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{RebeccaError, Result};
use crate::plan::{EstimateProvenance, EstimateSource};
use crate::safety::is_reparse_like;
use crate::scan::{
    ScanBackendKind, ScanCancellationToken, ScanEngine, ScanEstimateConfidence, ScanReport,
};

pub const DEFAULT_DISK_MAP_TOP_LIMIT: usize = 20;

#[derive(Debug, Clone)]
pub struct DiskMapRequest {
    pub roots: Vec<PathBuf>,
    pub top_limit: usize,
    pub max_depth: Option<usize>,
    pub scan_backend: ScanBackendKind,
}

impl DiskMapRequest {
    pub fn new(roots: Vec<PathBuf>) -> Self {
        Self {
            roots,
            top_limit: DEFAULT_DISK_MAP_TOP_LIMIT,
            max_depth: None,
            scan_backend: ScanBackendKind::PortableRecursive,
        }
    }

    pub fn with_top_limit(mut self, top_limit: usize) -> Self {
        self.top_limit = top_limit;
        self
    }

    pub fn with_max_depth(mut self, max_depth: Option<usize>) -> Self {
        self.max_depth = max_depth;
        self
    }

    pub fn with_scan_backend(mut self, scan_backend: ScanBackendKind) -> Self {
        self.scan_backend = scan_backend;
        self
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiskMapReport {
    pub roots: Vec<DiskMapRoot>,
    pub totals: DiskMapMetrics,
    pub top_entries: Vec<DiskMapEntry>,
    pub diagnostics: Vec<DiskMapDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiskMapRoot {
    pub path: PathBuf,
    pub status: DiskMapRootStatus,
    pub metrics: DiskMapMetrics,
    pub estimate_source: EstimateSource,
    #[serde(default, flatten)]
    pub estimate_provenance: EstimateProvenance,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DiskMapRootStatus {
    Scanned,
    Skipped,
}

impl DiskMapRootStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Scanned => "scanned",
            Self::Skipped => "skipped",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiskMapMetrics {
    pub logical_bytes: u64,
    pub allocated_bytes: Option<u64>,
    pub files: u64,
    pub directories: u64,
}

impl DiskMapMetrics {
    pub(crate) fn add(&mut self, other: Self) {
        self.logical_bytes = self.logical_bytes.saturating_add(other.logical_bytes);
        self.allocated_bytes = add_optional_bytes(
            self.allocated_bytes,
            self.files,
            other.allocated_bytes,
            other.files,
        );
        self.files = self.files.saturating_add(other.files);
        self.directories = self.directories.saturating_add(other.directories);
    }

    fn from_scan_report(report: ScanReport) -> Self {
        Self {
            logical_bytes: report.bytes_scanned,
            allocated_bytes: None,
            files: report.files_scanned,
            directories: report.directories_scanned,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiskMapEntry {
    pub path: PathBuf,
    pub root: PathBuf,
    pub kind: DiskMapEntryKind,
    pub depth: usize,
    pub logical_bytes: u64,
    pub allocated_bytes: Option<u64>,
    pub files: u64,
    pub directories: u64,
    pub estimate_source: EstimateSource,
    #[serde(default, flatten)]
    pub estimate_provenance: EstimateProvenance,
}

impl DiskMapEntry {
    fn portable(
        root: &Path,
        path: PathBuf,
        kind: DiskMapEntryKind,
        depth: usize,
        metrics: DiskMapMetrics,
        estimate_provenance: &EstimateProvenance,
    ) -> Self {
        Self {
            path,
            root: root.to_path_buf(),
            kind,
            depth,
            logical_bytes: metrics.logical_bytes,
            allocated_bytes: metrics.allocated_bytes,
            files: metrics.files,
            directories: metrics.directories,
            estimate_source: EstimateSource::FreshScan,
            estimate_provenance: estimate_provenance.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DiskMapEntryKind {
    File,
    Directory,
    Other,
}

impl DiskMapEntryKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Directory => "directory",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DiskMapDiagnostic {
    pub kind: DiskMapDiagnosticKind,
    pub path: PathBuf,
    pub detail: String,
}

impl DiskMapDiagnostic {
    pub fn new(kind: DiskMapDiagnosticKind, path: PathBuf, detail: impl Into<String>) -> Self {
        Self {
            kind,
            path,
            detail: detail.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DiskMapDiagnosticKind {
    RootMissing,
    RootMetadataReadSkipped,
    ReparsePointSkipped,
    DirectoryReadSkipped,
    DirectoryEntryReadSkipped,
    MetadataReadSkipped,
    Fallback,
    ScanFailed,
}

impl DiskMapDiagnosticKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::RootMissing => "root-missing",
            Self::RootMetadataReadSkipped => "root-metadata-read-skipped",
            Self::ReparsePointSkipped => "reparse-point-skipped",
            Self::DirectoryReadSkipped => "directory-read-skipped",
            Self::DirectoryEntryReadSkipped => "directory-entry-read-skipped",
            Self::MetadataReadSkipped => "metadata-read-skipped",
            Self::Fallback => "fallback",
            Self::ScanFailed => "scan-failed",
        }
    }
}

pub fn inspect_map(
    request: &DiskMapRequest,
    cancellation: &ScanCancellationToken,
) -> Result<DiskMapReport> {
    let mut report = DiskMapReport::default();
    let mut top_entries = DiskMapTopEntries::new(request.top_limit);
    let scan_engine = ScanEngine::new();

    for root in &request.roots {
        check_cancelled(cancellation)?;
        inspect_root(
            root,
            request,
            cancellation,
            &scan_engine,
            &mut report,
            &mut top_entries,
        )?;
    }

    report.top_entries = top_entries.into_sorted_entries();
    report.diagnostics.sort();
    Ok(report)
}

fn inspect_root(
    root: &Path,
    request: &DiskMapRequest,
    cancellation: &ScanCancellationToken,
    scan_engine: &ScanEngine,
    report: &mut DiskMapReport,
    top_entries: &mut DiskMapTopEntries,
) -> Result<()> {
    if request.scan_backend == ScanBackendKind::WindowsNtfsMftExperimental {
        match scan_engine.inspect_windows_ntfs_mft_disk_map(
            root,
            request.top_limit,
            request.max_depth,
            cancellation,
        ) {
            Ok(root_map) => {
                push_backend_root(root, root_map, report, top_entries);
                return Ok(());
            }
            Err(err) if disk_map_backend_error_can_fallback(&err) => {
                return inspect_portable_root(
                    root,
                    request,
                    cancellation,
                    Some(format!(
                        "windows-ntfs-mft-experimental disk-map inventory was unavailable: {err}"
                    )),
                    report,
                    top_entries,
                );
            }
            Err(err) => return Err(err),
        }
    }

    let fallback_reason = (request.scan_backend != ScanBackendKind::PortableRecursive).then(|| {
        format!(
            "{} disk-map inventory is not available; portable recursive inventory was used",
            request.scan_backend.label()
        )
    });
    inspect_portable_root(
        root,
        request,
        cancellation,
        fallback_reason,
        report,
        top_entries,
    )
}

fn inspect_portable_root(
    root: &Path,
    request: &DiskMapRequest,
    cancellation: &ScanCancellationToken,
    fallback_reason: Option<String>,
    report: &mut DiskMapReport,
    top_entries: &mut DiskMapTopEntries,
) -> Result<()> {
    let provenance = portable_estimate_provenance(fallback_reason.clone());
    if let Some(reason) = &fallback_reason {
        report.diagnostics.push(DiskMapDiagnostic::new(
            DiskMapDiagnosticKind::Fallback,
            root.to_path_buf(),
            reason.clone(),
        ));
    }

    let metadata = match std::fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            push_root_skip(
                report,
                root,
                DiskMapDiagnosticKind::RootMissing,
                "disk map root does not exist",
                provenance,
            );
            return Ok(());
        }
        Err(err) => {
            push_root_skip(
                report,
                root,
                DiskMapDiagnosticKind::RootMetadataReadSkipped,
                format!("disk map root metadata could not be read: {err}"),
                provenance,
            );
            return Ok(());
        }
    };

    if is_reparse_like(&metadata) {
        push_root_skip(
            report,
            root,
            DiskMapDiagnosticKind::ReparsePointSkipped,
            "disk map root is a symlink or reparse point",
            provenance,
        );
        return Ok(());
    }

    let max_depth = request.max_depth.unwrap_or(usize::MAX);
    let root_metrics = if metadata.is_file() {
        let metrics = portable_file_metrics(metadata);
        push_portable_entry(
            root,
            root.to_path_buf(),
            DiskMapEntryKind::File,
            0,
            metrics,
            &provenance,
            top_entries,
        );
        metrics
    } else if metadata.is_dir() {
        inspect_portable_directory_root(root, cancellation, max_depth, &provenance, top_entries)?
    } else {
        DiskMapMetrics::from_scan_report(ScanReport::default())
    };

    report.totals.add(root_metrics);
    report.roots.push(DiskMapRoot {
        path: root.to_path_buf(),
        status: DiskMapRootStatus::Scanned,
        metrics: root_metrics,
        estimate_source: EstimateSource::FreshScan,
        estimate_provenance: provenance,
        reason: None,
    });
    Ok(())
}

fn push_backend_root(
    root: &Path,
    root_map: DiskMapBackendRoot,
    report: &mut DiskMapReport,
    top_entries: &mut DiskMapTopEntries,
) {
    report.totals.add(root_map.metrics);
    for entry in root_map.top_entries {
        top_entries.push(entry);
    }
    report.diagnostics.extend(root_map.diagnostics);
    report.roots.push(DiskMapRoot {
        path: root.to_path_buf(),
        status: DiskMapRootStatus::Scanned,
        metrics: root_map.metrics,
        estimate_source: EstimateSource::FreshScan,
        estimate_provenance: root_map.estimate_provenance,
        reason: None,
    });
}

fn inspect_portable_directory_root(
    root: &Path,
    cancellation: &ScanCancellationToken,
    max_depth: usize,
    estimate_provenance: &EstimateProvenance,
    top_entries: &mut DiskMapTopEntries,
) -> Result<DiskMapMetrics> {
    let entries = std::fs::read_dir(root).map_err(|err| {
        RebeccaError::ScanFailed(crate::error::ScanFailure::from_io(
            root,
            crate::error::ScanFailurePhase::DirectoryWalk,
            &err,
        ))
    })?;
    let mut child_paths = Vec::new();
    for entry in entries {
        check_cancelled(cancellation)?;
        let entry = entry.map_err(|err| {
            RebeccaError::ScanFailed(crate::error::ScanFailure::from_io(
                root,
                crate::error::ScanFailurePhase::DirectoryWalk,
                &err,
            ))
        })?;
        child_paths.push(entry.path());
    }
    child_paths.sort();

    let mut metrics = DiskMapMetrics::default();
    for child in child_paths {
        check_cancelled(cancellation)?;
        let child_metrics = inspect_portable_node(
            root,
            &child,
            1,
            cancellation,
            max_depth,
            estimate_provenance,
            top_entries,
        )?;
        metrics.add(child_metrics);
    }
    Ok(metrics)
}

fn inspect_portable_node(
    root: &Path,
    path: &Path,
    depth: usize,
    cancellation: &ScanCancellationToken,
    max_depth: usize,
    estimate_provenance: &EstimateProvenance,
    top_entries: &mut DiskMapTopEntries,
) -> Result<DiskMapMetrics> {
    let metadata = std::fs::symlink_metadata(path).map_err(|err| {
        RebeccaError::ScanFailed(crate::error::ScanFailure::from_io(
            path,
            crate::error::ScanFailurePhase::EntryMetadata,
            &err,
        ))
    })?;
    if is_reparse_like(&metadata) {
        let metrics = DiskMapMetrics::default();
        push_portable_entry_if_visible(
            root,
            path.to_path_buf(),
            DiskMapEntryKind::Other,
            depth,
            metrics,
            max_depth,
            estimate_provenance,
            top_entries,
        );
        return Ok(metrics);
    }

    if metadata.is_file() {
        let metrics = portable_file_metrics(metadata);
        push_portable_entry_if_visible(
            root,
            path.to_path_buf(),
            DiskMapEntryKind::File,
            depth,
            metrics,
            max_depth,
            estimate_provenance,
            top_entries,
        );
        return Ok(metrics);
    }

    if !metadata.is_dir() {
        let metrics = DiskMapMetrics::default();
        push_portable_entry_if_visible(
            root,
            path.to_path_buf(),
            DiskMapEntryKind::Other,
            depth,
            metrics,
            max_depth,
            estimate_provenance,
            top_entries,
        );
        return Ok(metrics);
    }

    let entries = std::fs::read_dir(path).map_err(|err| {
        RebeccaError::ScanFailed(crate::error::ScanFailure::from_io(
            path,
            crate::error::ScanFailurePhase::DirectoryWalk,
            &err,
        ))
    })?;
    let mut child_paths = Vec::new();
    for entry in entries {
        check_cancelled(cancellation)?;
        let entry = entry.map_err(|err| {
            RebeccaError::ScanFailed(crate::error::ScanFailure::from_io(
                path,
                crate::error::ScanFailurePhase::DirectoryWalk,
                &err,
            ))
        })?;
        child_paths.push(entry.path());
    }
    child_paths.sort();

    let mut metrics = DiskMapMetrics {
        logical_bytes: 0,
        allocated_bytes: None,
        files: 0,
        directories: 1,
    };
    for child in child_paths {
        check_cancelled(cancellation)?;
        let child_metrics = inspect_portable_node(
            root,
            &child,
            depth.saturating_add(1),
            cancellation,
            max_depth,
            estimate_provenance,
            top_entries,
        )?;
        metrics.add(child_metrics);
    }

    push_portable_entry_if_visible(
        root,
        path.to_path_buf(),
        DiskMapEntryKind::Directory,
        depth,
        metrics,
        max_depth,
        estimate_provenance,
        top_entries,
    );
    Ok(metrics)
}

fn portable_file_metrics(metadata: std::fs::Metadata) -> DiskMapMetrics {
    DiskMapMetrics {
        logical_bytes: metadata.len(),
        allocated_bytes: None,
        files: 1,
        directories: 0,
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "entry projection keeps call sites explicit during traversal"
)]
fn push_portable_entry_if_visible(
    root: &Path,
    path: PathBuf,
    kind: DiskMapEntryKind,
    depth: usize,
    metrics: DiskMapMetrics,
    max_depth: usize,
    estimate_provenance: &EstimateProvenance,
    top_entries: &mut DiskMapTopEntries,
) {
    if depth <= max_depth {
        push_portable_entry(
            root,
            path,
            kind,
            depth,
            metrics,
            estimate_provenance,
            top_entries,
        );
    }
}

fn push_portable_entry(
    root: &Path,
    path: PathBuf,
    kind: DiskMapEntryKind,
    depth: usize,
    metrics: DiskMapMetrics,
    estimate_provenance: &EstimateProvenance,
    top_entries: &mut DiskMapTopEntries,
) {
    top_entries.push(DiskMapEntry::portable(
        root,
        path,
        kind,
        depth,
        metrics,
        estimate_provenance,
    ));
}

#[derive(Debug, Default)]
pub(crate) struct DiskMapTopEntries {
    limit: usize,
    heap: BinaryHeap<Reverse<DiskMapTopCandidate>>,
    sequence: u64,
}

impl DiskMapTopEntries {
    pub(crate) fn new(limit: usize) -> Self {
        Self {
            limit,
            heap: BinaryHeap::with_capacity(limit),
            sequence: 0,
        }
    }

    pub(crate) fn push(&mut self, entry: DiskMapEntry) {
        if self.limit == 0 {
            return;
        }

        let candidate = DiskMapTopCandidate {
            rank: DiskMapTopRank::from_entry(&entry),
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

    pub(crate) fn into_sorted_entries(self) -> Vec<DiskMapEntry> {
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
struct DiskMapTopCandidate {
    rank: DiskMapTopRank,
    sequence: u64,
    entry: DiskMapEntry,
}

impl Ord for DiskMapTopCandidate {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.rank
            .cmp(&other.rank)
            .then_with(|| other.sequence.cmp(&self.sequence))
    }
}

impl PartialOrd for DiskMapTopCandidate {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct DiskMapTopRank {
    logical_bytes: u64,
    files: u64,
    directories: u64,
    reverse_path: Reverse<PathBuf>,
}

impl DiskMapTopRank {
    fn from_entry(entry: &DiskMapEntry) -> Self {
        Self {
            logical_bytes: entry.logical_bytes,
            files: entry.files,
            directories: entry.directories,
            reverse_path: Reverse(entry.path.clone()),
        }
    }
}

fn push_root_skip(
    report: &mut DiskMapReport,
    root: &Path,
    kind: DiskMapDiagnosticKind,
    detail: impl Into<String>,
    estimate_provenance: EstimateProvenance,
) {
    let detail = detail.into();
    report.roots.push(DiskMapRoot {
        path: root.to_path_buf(),
        status: DiskMapRootStatus::Skipped,
        metrics: DiskMapMetrics::default(),
        estimate_source: EstimateSource::NotMeasured,
        estimate_provenance,
        reason: Some(detail.clone()),
    });
    report
        .diagnostics
        .push(DiskMapDiagnostic::new(kind, root.to_path_buf(), detail));
}

pub(crate) struct DiskMapBackendRoot {
    pub(crate) metrics: DiskMapMetrics,
    pub(crate) top_entries: Vec<DiskMapEntry>,
    pub(crate) diagnostics: Vec<DiskMapDiagnostic>,
    pub(crate) estimate_provenance: EstimateProvenance,
}

fn portable_estimate_provenance(fallback_reason: Option<String>) -> EstimateProvenance {
    let mut provenance = EstimateProvenance::from_backend_confidence(
        ScanBackendKind::PortableRecursive,
        ScanEstimateConfidence::Exact,
    );
    provenance.estimate_fallback_reason = fallback_reason;
    provenance
}

fn add_optional_bytes(
    left: Option<u64>,
    left_files: u64,
    right: Option<u64>,
    right_files: u64,
) -> Option<u64> {
    if right_files == 0 {
        return left;
    }

    match (left, right) {
        (None, Some(right)) if left_files == 0 => Some(right),
        (Some(left), Some(right)) => Some(left.saturating_add(right)),
        _ => None,
    }
}

fn disk_map_backend_error_can_fallback(err: &RebeccaError) -> bool {
    matches!(
        err,
        RebeccaError::PlatformUnavailable(_)
            | RebeccaError::ScanFailed(_)
            | RebeccaError::SafetyBlocked(_)
    )
}

fn check_cancelled(cancellation: &ScanCancellationToken) -> Result<()> {
    if cancellation.is_cancelled() {
        return Err(RebeccaError::OperationCancelled(
            "disk map inspection was cancelled".to_string(),
        ));
    }

    Ok(())
}

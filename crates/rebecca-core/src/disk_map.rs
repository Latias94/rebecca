use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::io;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PortableMetadataKind {
    File,
    Directory,
    Other,
}

trait PortableDiskMapWalker {
    type Metadata;
    type ReadDir;

    fn symlink_metadata(&self, path: &Path) -> io::Result<Self::Metadata>;
    fn is_reparse_like(&self, path: &Path, metadata: &Self::Metadata) -> bool;
    fn metadata_len(&self, metadata: &Self::Metadata) -> u64;
    fn metadata_kind(&self, metadata: &Self::Metadata) -> PortableMetadataKind;
    fn read_dir(&self, path: &Path) -> io::Result<Self::ReadDir>;
    fn next_entry(&self, entries: &mut Self::ReadDir) -> Option<io::Result<PathBuf>>;
}

#[derive(Debug, Default)]
struct FsPortableDiskMapWalker;

impl PortableDiskMapWalker for FsPortableDiskMapWalker {
    type Metadata = std::fs::Metadata;
    type ReadDir = std::fs::ReadDir;

    fn symlink_metadata(&self, path: &Path) -> io::Result<Self::Metadata> {
        std::fs::symlink_metadata(path)
    }

    fn is_reparse_like(&self, _path: &Path, metadata: &Self::Metadata) -> bool {
        is_reparse_like(metadata)
    }

    fn metadata_len(&self, metadata: &Self::Metadata) -> u64 {
        metadata.len()
    }

    fn metadata_kind(&self, metadata: &Self::Metadata) -> PortableMetadataKind {
        if metadata.is_file() {
            PortableMetadataKind::File
        } else if metadata.is_dir() {
            PortableMetadataKind::Directory
        } else {
            PortableMetadataKind::Other
        }
    }

    fn read_dir(&self, path: &Path) -> io::Result<Self::ReadDir> {
        std::fs::read_dir(path)
    }

    fn next_entry(&self, entries: &mut Self::ReadDir) -> Option<io::Result<PathBuf>> {
        entries.next().map(|entry| entry.map(|entry| entry.path()))
    }
}

struct PortableDiskMapTraversal<'a, W>
where
    W: PortableDiskMapWalker,
{
    root: &'a Path,
    cancellation: &'a ScanCancellationToken,
    max_depth: usize,
    estimate_provenance: &'a EstimateProvenance,
    top_entries: &'a mut DiskMapTopEntries,
    diagnostics: &'a mut Vec<DiskMapDiagnostic>,
    walker: &'a W,
}

impl<'a, W> PortableDiskMapTraversal<'a, W>
where
    W: PortableDiskMapWalker,
{
    fn inspect_root_directory(&mut self) -> Result<DiskMapMetrics> {
        let child_paths = self.read_sorted_child_paths(self.root)?;

        let mut metrics = DiskMapMetrics::default();
        for child in child_paths {
            check_cancelled(self.cancellation)?;
            let child_metrics = self.inspect_node(&child, 1)?;
            metrics.add(child_metrics);
        }
        Ok(metrics)
    }

    fn inspect_node(&mut self, path: &Path, depth: usize) -> Result<DiskMapMetrics> {
        let metadata = match self.walker.symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(err) => {
                self.diagnostics.push(DiskMapDiagnostic::new(
                    DiskMapDiagnosticKind::MetadataReadSkipped,
                    path.to_path_buf(),
                    format!("disk map entry metadata could not be read: {err}"),
                ));
                return Ok(DiskMapMetrics::default());
            }
        };
        if self.walker.is_reparse_like(path, &metadata) {
            self.diagnostics.push(DiskMapDiagnostic::new(
                DiskMapDiagnosticKind::ReparsePointSkipped,
                path.to_path_buf(),
                "disk map entry is a symlink or reparse point",
            ));
            let metrics = DiskMapMetrics::default();
            self.push_entry_if_visible(path, DiskMapEntryKind::Other, depth, metrics);
            return Ok(metrics);
        }

        match self.walker.metadata_kind(&metadata) {
            PortableMetadataKind::File => {
                let metrics = portable_file_metrics(self.walker.metadata_len(&metadata));
                self.push_entry_if_visible(path, DiskMapEntryKind::File, depth, metrics);
                Ok(metrics)
            }
            PortableMetadataKind::Directory => self.inspect_directory_node(path, depth),
            PortableMetadataKind::Other => {
                let metrics = DiskMapMetrics::default();
                self.push_entry_if_visible(path, DiskMapEntryKind::Other, depth, metrics);
                Ok(metrics)
            }
        }
    }

    fn inspect_directory_node(&mut self, path: &Path, depth: usize) -> Result<DiskMapMetrics> {
        let child_paths = match self.read_sorted_child_paths(path) {
            Ok(child_paths) => child_paths,
            Err(err @ RebeccaError::OperationCancelled(_)) => return Err(err),
            Err(err) => {
                self.diagnostics.push(DiskMapDiagnostic::new(
                    DiskMapDiagnosticKind::DirectoryReadSkipped,
                    path.to_path_buf(),
                    format!("disk map directory could not be read: {err}"),
                ));
                return Ok(DiskMapMetrics::default());
            }
        };

        let mut metrics = DiskMapMetrics {
            logical_bytes: 0,
            allocated_bytes: None,
            files: 0,
            directories: 1,
        };
        for child in child_paths {
            check_cancelled(self.cancellation)?;
            let child_metrics = self.inspect_node(&child, depth.saturating_add(1))?;
            metrics.add(child_metrics);
        }

        self.push_entry_if_visible(path, DiskMapEntryKind::Directory, depth, metrics);
        Ok(metrics)
    }

    fn read_sorted_child_paths(&mut self, path: &Path) -> Result<Vec<PathBuf>> {
        let mut entries = self.walker.read_dir(path).map_err(|err| {
            RebeccaError::ScanFailed(crate::error::ScanFailure::from_io(
                path,
                crate::error::ScanFailurePhase::DirectoryWalk,
                &err,
            ))
        })?;
        let mut child_paths = Vec::new();
        while let Some(entry) = self.walker.next_entry(&mut entries) {
            check_cancelled(self.cancellation)?;
            match entry {
                Ok(path) => child_paths.push(path),
                Err(err) => self.diagnostics.push(DiskMapDiagnostic::new(
                    DiskMapDiagnosticKind::DirectoryEntryReadSkipped,
                    path.to_path_buf(),
                    format!("disk map directory entry could not be read: {err}"),
                )),
            }
        }
        child_paths.sort();
        Ok(child_paths)
    }

    fn push_entry_if_visible(
        &mut self,
        path: &Path,
        kind: DiskMapEntryKind,
        depth: usize,
        metrics: DiskMapMetrics,
    ) {
        push_portable_entry_if_visible(
            self.root,
            path.to_path_buf(),
            kind,
            depth,
            metrics,
            self.max_depth,
            self.estimate_provenance,
            self.top_entries,
        );
    }
}

fn inspect_root(
    root: &Path,
    request: &DiskMapRequest,
    cancellation: &ScanCancellationToken,
    scan_engine: &ScanEngine,
    report: &mut DiskMapReport,
    top_entries: &mut DiskMapTopEntries,
) -> Result<()> {
    let walker = FsPortableDiskMapWalker;
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
                    &walker,
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
        &walker,
    )
}

fn inspect_portable_root<W>(
    root: &Path,
    request: &DiskMapRequest,
    cancellation: &ScanCancellationToken,
    fallback_reason: Option<String>,
    report: &mut DiskMapReport,
    top_entries: &mut DiskMapTopEntries,
    walker: &W,
) -> Result<()>
where
    W: PortableDiskMapWalker,
{
    let provenance = portable_estimate_provenance(fallback_reason.clone());
    if let Some(reason) = &fallback_reason {
        report.diagnostics.push(DiskMapDiagnostic::new(
            DiskMapDiagnosticKind::Fallback,
            root.to_path_buf(),
            reason.clone(),
        ));
    }

    let metadata = match walker.symlink_metadata(root) {
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

    if walker.is_reparse_like(root, &metadata) {
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
    let root_metrics = match walker.metadata_kind(&metadata) {
        PortableMetadataKind::File => {
            let metrics = portable_file_metrics(walker.metadata_len(&metadata));
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
        }
        PortableMetadataKind::Directory => match (PortableDiskMapTraversal {
            root,
            cancellation,
            max_depth,
            estimate_provenance: &provenance,
            top_entries,
            diagnostics: &mut report.diagnostics,
            walker,
        })
        .inspect_root_directory()
        {
            Ok(metrics) => metrics,
            Err(err @ RebeccaError::OperationCancelled(_)) => return Err(err),
            Err(err) => {
                push_root_skip(
                    report,
                    root,
                    DiskMapDiagnosticKind::DirectoryReadSkipped,
                    format!("disk map root directory could not be read: {err}"),
                    provenance,
                );
                return Ok(());
            }
        },
        PortableMetadataKind::Other => DiskMapMetrics::from_scan_report(ScanReport::default()),
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

fn portable_file_metrics(len: u64) -> DiskMapMetrics {
    DiskMapMetrics {
        logical_bytes: len,
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

#[cfg(test)]
fn inspect_map_with_walker_for_test<W>(
    request: &DiskMapRequest,
    cancellation: &ScanCancellationToken,
    walker: &W,
) -> Result<DiskMapReport>
where
    W: PortableDiskMapWalker,
{
    let mut report = DiskMapReport::default();
    let mut top_entries = DiskMapTopEntries::new(request.top_limit);

    for root in &request.roots {
        check_cancelled(cancellation)?;
        inspect_portable_root(
            root,
            request,
            cancellation,
            None,
            &mut report,
            &mut top_entries,
            walker,
        )?;
    }

    report.top_entries = top_entries.into_sorted_entries();
    report.diagnostics.sort();
    Ok(report)
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, VecDeque};
    use std::io;

    use super::*;

    #[derive(Debug, Clone)]
    enum FakeEntry {
        Path(PathBuf),
        Error(&'static str),
    }

    #[derive(Debug, Clone)]
    struct FakeMetadata {
        kind: FakeMetadataKind,
        len: u64,
    }

    #[derive(Debug, Clone, Copy)]
    enum FakeMetadataKind {
        File,
        Directory,
        Other,
    }

    #[derive(Debug, Default)]
    struct FakeDiskMapWalker {
        metadata: BTreeMap<PathBuf, std::result::Result<FakeMetadata, &'static str>>,
        directories: BTreeMap<PathBuf, std::result::Result<VecDeque<FakeEntry>, &'static str>>,
        reparse_paths: Vec<PathBuf>,
    }

    impl FakeDiskMapWalker {
        fn with_file(mut self, path: impl Into<PathBuf>, len: u64) -> Self {
            self.metadata.insert(
                path.into(),
                Ok(FakeMetadata {
                    kind: FakeMetadataKind::File,
                    len,
                }),
            );
            self
        }

        fn with_directory(
            mut self,
            path: impl Into<PathBuf>,
            entries: impl IntoIterator<Item = FakeEntry>,
        ) -> Self {
            let path = path.into();
            self.metadata.insert(
                path.clone(),
                Ok(FakeMetadata {
                    kind: FakeMetadataKind::Directory,
                    len: 0,
                }),
            );
            self.directories
                .insert(path, Ok(entries.into_iter().collect()));
            self
        }

        fn with_other(mut self, path: impl Into<PathBuf>) -> Self {
            self.metadata.insert(
                path.into(),
                Ok(FakeMetadata {
                    kind: FakeMetadataKind::Other,
                    len: 0,
                }),
            );
            self
        }

        fn with_metadata_error(mut self, path: impl Into<PathBuf>, message: &'static str) -> Self {
            self.metadata.insert(path.into(), Err(message));
            self
        }

        fn with_directory_error(mut self, path: impl Into<PathBuf>, message: &'static str) -> Self {
            let path = path.into();
            self.metadata.insert(
                path.clone(),
                Ok(FakeMetadata {
                    kind: FakeMetadataKind::Directory,
                    len: 0,
                }),
            );
            self.directories.insert(path, Err(message));
            self
        }

        fn with_reparse(mut self, path: impl Into<PathBuf>) -> Self {
            self.reparse_paths.push(path.into());
            self
        }
    }

    impl PortableDiskMapWalker for FakeDiskMapWalker {
        type Metadata = FakeMetadata;
        type ReadDir = VecDeque<FakeEntry>;

        fn symlink_metadata(&self, path: &Path) -> io::Result<Self::Metadata> {
            match self.metadata.get(path) {
                Some(Ok(metadata)) => Ok(metadata.clone()),
                Some(Err(message)) => Err(io::Error::other(*message)),
                None => Err(io::Error::new(io::ErrorKind::NotFound, "missing")),
            }
        }

        fn is_reparse_like(&self, path: &Path, _metadata: &Self::Metadata) -> bool {
            self.reparse_paths.iter().any(|candidate| candidate == path)
        }

        fn metadata_len(&self, metadata: &Self::Metadata) -> u64 {
            metadata.len
        }

        fn metadata_kind(&self, metadata: &Self::Metadata) -> PortableMetadataKind {
            match metadata.kind {
                FakeMetadataKind::File => PortableMetadataKind::File,
                FakeMetadataKind::Directory => PortableMetadataKind::Directory,
                FakeMetadataKind::Other => PortableMetadataKind::Other,
            }
        }

        fn read_dir(&self, path: &Path) -> io::Result<Self::ReadDir> {
            match self.directories.get(path) {
                Some(Ok(entries)) => Ok(entries.clone()),
                Some(Err(message)) => Err(io::Error::other(*message)),
                None => Err(io::Error::new(io::ErrorKind::NotFound, "missing dir")),
            }
        }

        fn next_entry(&self, entries: &mut Self::ReadDir) -> Option<io::Result<PathBuf>> {
            entries.pop_front().map(|entry| match entry {
                FakeEntry::Path(path) => Ok(path),
                FakeEntry::Error(message) => Err(io::Error::other(message)),
            })
        }
    }

    #[test]
    fn portable_map_skips_child_metadata_errors_with_diagnostics() {
        let root = PathBuf::from("C:\\root");
        let readable = root.join("readable.bin");
        let missing = root.join("missing.bin");
        let walker = FakeDiskMapWalker::default()
            .with_directory(
                &root,
                [
                    FakeEntry::Path(readable.clone()),
                    FakeEntry::Path(missing.clone()),
                ],
            )
            .with_file(&readable, 7)
            .with_metadata_error(&missing, "raced away");
        let report = inspect_map_with_walker_for_test(
            &DiskMapRequest::new(vec![root.clone()]),
            &ScanCancellationToken::new(),
            &walker,
        )
        .unwrap();

        assert_eq!(report.totals.logical_bytes, 7);
        assert_eq!(report.totals.files, 1);
        assert!(report.diagnostics.iter().any(|diagnostic| {
            diagnostic.kind == DiskMapDiagnosticKind::MetadataReadSkipped
                && diagnostic.path == missing
                && diagnostic.detail.contains("raced away")
        }));
    }

    #[test]
    fn portable_map_skips_child_directory_read_errors_with_diagnostics() {
        let root = PathBuf::from("C:\\root");
        let readable = root.join("readable.bin");
        let unreadable = root.join("locked");
        let walker = FakeDiskMapWalker::default()
            .with_directory(
                &root,
                [
                    FakeEntry::Path(unreadable.clone()),
                    FakeEntry::Path(readable.clone()),
                ],
            )
            .with_file(&readable, 11)
            .with_directory_error(&unreadable, "access denied");
        let report = inspect_map_with_walker_for_test(
            &DiskMapRequest::new(vec![root.clone()]),
            &ScanCancellationToken::new(),
            &walker,
        )
        .unwrap();

        assert_eq!(report.totals.logical_bytes, 11);
        assert_eq!(report.totals.files, 1);
        assert!(report.diagnostics.iter().any(|diagnostic| {
            diagnostic.kind == DiskMapDiagnosticKind::DirectoryReadSkipped
                && diagnostic.path == unreadable
                && diagnostic.detail.contains("access denied")
        }));
    }

    #[test]
    fn portable_map_continues_after_directory_entry_errors() {
        let root = PathBuf::from("C:\\root");
        let readable = root.join("readable.bin");
        let walker = FakeDiskMapWalker::default()
            .with_directory(
                &root,
                [
                    FakeEntry::Error("entry vanished"),
                    FakeEntry::Path(readable.clone()),
                ],
            )
            .with_file(&readable, 13);
        let report = inspect_map_with_walker_for_test(
            &DiskMapRequest::new(vec![root.clone()]),
            &ScanCancellationToken::new(),
            &walker,
        )
        .unwrap();

        assert_eq!(report.totals.logical_bytes, 13);
        assert_eq!(report.totals.files, 1);
        assert!(report.diagnostics.iter().any(|diagnostic| {
            diagnostic.kind == DiskMapDiagnosticKind::DirectoryEntryReadSkipped
                && diagnostic.path == root
                && diagnostic.detail.contains("entry vanished")
        }));
    }

    #[test]
    fn portable_map_reports_reparse_child_skips() {
        let root = PathBuf::from("C:\\root");
        let link = root.join("link");
        let walker = FakeDiskMapWalker::default()
            .with_directory(&root, [FakeEntry::Path(link.clone())])
            .with_other(&link)
            .with_reparse(&link);
        let report = inspect_map_with_walker_for_test(
            &DiskMapRequest::new(vec![root]),
            &ScanCancellationToken::new(),
            &walker,
        )
        .unwrap();

        assert_eq!(report.totals.logical_bytes, 0);
        assert!(report.diagnostics.iter().any(|diagnostic| {
            diagnostic.kind == DiskMapDiagnosticKind::ReparsePointSkipped && diagnostic.path == link
        }));
    }
}

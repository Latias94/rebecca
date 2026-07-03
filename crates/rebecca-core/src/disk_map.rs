use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap};
#[cfg(windows)]
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

use crate::cleanup_advice::CleanupAdvice;
use crate::error::{RebeccaError, Result, ScanFailureKind};
use crate::plan::{EstimateProvenance, EstimateSource};
use crate::safety::is_reparse_like;
use crate::scan::{
    ScanBackendKind, ScanCancellationToken, ScanEngine, ScanEstimateCaveat, ScanEstimateConfidence,
};

pub const DEFAULT_DISK_MAP_TOP_LIMIT: usize = 20;
pub const DEFAULT_DISK_MAP_DIAGNOSTIC_LIMIT: usize = 100;
pub const DEFAULT_DISK_MAP_GROUP_LIMIT: usize = 20;

#[derive(Debug, Clone)]
pub struct DiskMapRequest {
    pub roots: Vec<PathBuf>,
    pub top_limit: usize,
    pub top_sort: DiskMapSortField,
    pub entry_filter: DiskMapEntryFilter,
    pub diagnostic_limit: usize,
    pub group_kinds: Vec<DiskMapGroupKind>,
    pub group_limit: usize,
    pub group_sort: DiskMapSortField,
    pub max_depth: Option<usize>,
    pub scan_backend: ScanBackendKind,
}

impl DiskMapRequest {
    pub fn new(roots: Vec<PathBuf>) -> Self {
        Self {
            roots,
            top_limit: DEFAULT_DISK_MAP_TOP_LIMIT,
            top_sort: DiskMapSortField::Logical,
            entry_filter: DiskMapEntryFilter::default(),
            diagnostic_limit: DEFAULT_DISK_MAP_DIAGNOSTIC_LIMIT,
            group_kinds: Vec::new(),
            group_limit: DEFAULT_DISK_MAP_GROUP_LIMIT,
            group_sort: DiskMapSortField::Logical,
            max_depth: None,
            scan_backend: ScanBackendKind::PortableRecursive,
        }
    }

    pub fn with_top_limit(mut self, top_limit: usize) -> Self {
        self.top_limit = top_limit;
        self
    }

    pub fn with_top_sort(mut self, top_sort: DiskMapSortField) -> Self {
        self.top_sort = top_sort;
        self
    }

    pub fn with_min_logical_bytes(mut self, min_logical_bytes: Option<u64>) -> Self {
        self.entry_filter.min_logical_bytes = min_logical_bytes;
        self
    }

    pub fn with_entry_kind(mut self, kind: Option<DiskMapEntryKind>) -> Self {
        self.entry_filter.kind = kind;
        self
    }

    pub fn with_path_contains(mut self, path_contains: Option<String>) -> Self {
        self.entry_filter.path_contains = path_contains
            .map(|value| value.trim().to_ascii_lowercase())
            .filter(|value| !value.is_empty());
        self
    }

    pub fn with_diagnostic_limit(mut self, diagnostic_limit: usize) -> Self {
        self.diagnostic_limit = diagnostic_limit;
        self
    }

    pub fn with_group_kinds(mut self, group_kinds: Vec<DiskMapGroupKind>) -> Self {
        self.group_kinds = group_kinds;
        self
    }

    pub fn with_group_limit(mut self, group_limit: usize) -> Self {
        self.group_limit = group_limit;
        self
    }

    pub fn with_group_sort(mut self, group_sort: DiskMapSortField) -> Self {
        self.group_sort = group_sort;
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiskMapEntryFilter {
    pub min_logical_bytes: Option<u64>,
    pub kind: Option<DiskMapEntryKind>,
    pub path_contains: Option<String>,
}

impl DiskMapEntryFilter {
    fn matches(&self, entry: &DiskMapEntry) -> bool {
        if self
            .min_logical_bytes
            .is_some_and(|minimum| entry.logical_bytes < minimum)
        {
            return false;
        }

        if self.kind.is_some_and(|kind| entry.kind != kind) {
            return false;
        }

        if let Some(needle) = &self.path_contains {
            let haystack = entry.path.to_string_lossy().to_ascii_lowercase();
            if !haystack.contains(needle) {
                return false;
            }
        }

        true
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiskMapReport {
    pub roots: Vec<DiskMapRoot>,
    pub totals: DiskMapMetrics,
    pub top_entries: Vec<DiskMapEntry>,
    pub groups: Vec<DiskMapGroup>,
    pub diagnostic_summary: DiskMapDiagnosticSummary,
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
    pub unique_logical_bytes: Option<u64>,
    pub unique_allocated_bytes: Option<u64>,
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
        self.unique_logical_bytes = add_optional_bytes(
            self.unique_logical_bytes,
            self.files,
            other.unique_logical_bytes,
            other.files,
        );
        self.unique_allocated_bytes = add_optional_bytes(
            self.unique_allocated_bytes,
            self.files,
            other.unique_allocated_bytes,
            other.files,
        );
        self.files = self.files.saturating_add(other.files);
        self.directories = self.directories.saturating_add(other.directories);
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DiskMapSortField {
    #[default]
    Logical,
    Allocated,
    Files,
    Unique,
}

impl DiskMapSortField {
    pub fn label(self) -> &'static str {
        match self {
            Self::Logical => "logical",
            Self::Allocated => "allocated",
            Self::Files => "files",
            Self::Unique => "unique",
        }
    }

    fn entry_value(self, entry: &DiskMapEntry) -> u64 {
        self.value(
            entry.logical_bytes,
            entry.allocated_bytes,
            entry.unique_logical_bytes,
            entry.files,
        )
    }

    fn metrics_value(self, metrics: &DiskMapMetrics) -> u64 {
        self.value(
            metrics.logical_bytes,
            metrics.allocated_bytes,
            metrics.unique_logical_bytes,
            metrics.files,
        )
    }

    fn value(
        self,
        logical_bytes: u64,
        allocated_bytes: Option<u64>,
        unique_logical_bytes: Option<u64>,
        files: u64,
    ) -> u64 {
        match self {
            Self::Logical => logical_bytes,
            Self::Allocated => allocated_bytes.unwrap_or(logical_bytes),
            Self::Files => files,
            Self::Unique => unique_logical_bytes.unwrap_or(logical_bytes),
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
    pub unique_logical_bytes: Option<u64>,
    pub unique_allocated_bytes: Option<u64>,
    pub files: u64,
    pub directories: u64,
    pub estimate_source: EstimateSource,
    #[serde(default, flatten)]
    pub estimate_provenance: EstimateProvenance,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cleanup_advice: Option<CleanupAdvice>,
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
            unique_logical_bytes: metrics.unique_logical_bytes,
            unique_allocated_bytes: metrics.unique_allocated_bytes,
            files: metrics.files,
            directories: metrics.directories,
            estimate_source: EstimateSource::FreshScan,
            estimate_provenance: estimate_provenance.clone(),
            cleanup_advice: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DiskMapGroupKind {
    Extension,
    Depth,
    Age,
}

impl DiskMapGroupKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Extension => "extension",
            Self::Depth => "depth",
            Self::Age => "age",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiskMapGroup {
    pub kind: DiskMapGroupKind,
    pub key: String,
    pub label: String,
    pub metrics: DiskMapMetrics,
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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiskMapDiagnosticSummary {
    pub total: u64,
    pub retained: u64,
    pub truncated: u64,
    pub by_kind: Vec<DiskMapDiagnosticKindSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiskMapDiagnosticKindSummary {
    pub kind: DiskMapDiagnosticKind,
    pub count: u64,
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
    let mut state = DiskMapInspectionState::new(request);
    let mut unique_files = DiskMapUniqueFiles::default();
    let scan_engine = ScanEngine::new();

    for root in &request.roots {
        check_cancelled(cancellation)?;
        unique_files.merge(inspect_root(
            root,
            request,
            cancellation,
            &scan_engine,
            &mut state,
        )?);
    }

    Ok(state.finish(unique_files))
}

#[derive(Debug)]
struct DiskMapInspectionState {
    report: DiskMapReport,
    top_entries: DiskMapTopEntries,
    groups: DiskMapGroupCollector,
    diagnostics: DiskMapDiagnostics,
}

impl DiskMapInspectionState {
    fn new(request: &DiskMapRequest) -> Self {
        Self {
            report: DiskMapReport::default(),
            top_entries: DiskMapTopEntries::new(
                request.top_limit,
                request.top_sort,
                request.entry_filter.clone(),
            ),
            groups: DiskMapGroupCollector::new(
                request.group_kinds.clone(),
                request.group_limit,
                SystemTime::now(),
                request.group_sort,
            ),
            diagnostics: DiskMapDiagnostics::new(request.diagnostic_limit),
        }
    }

    fn finish(mut self, unique_files: DiskMapUniqueFiles) -> DiskMapReport {
        unique_files.apply_to_metrics(&mut self.report.totals);
        self.report.top_entries = self.top_entries.into_sorted_entries();
        self.report.groups = self.groups.finish();
        self.diagnostics.finish(&mut self.report);
        self.report
    }

    fn backend_options(&self, request: &DiskMapRequest) -> DiskMapBackendOptions {
        DiskMapBackendOptions {
            top_limit: request.top_limit,
            top_sort: request.top_sort,
            entry_filter: request.entry_filter.clone(),
            max_depth: request.max_depth,
            group_kinds: request.group_kinds.clone(),
            group_limit: request.group_limit,
            group_now: self.groups.now(),
            group_sort: request.group_sort,
        }
    }
}

#[derive(Debug)]
struct DiskMapDiagnostics {
    limit: usize,
    total: u64,
    counts_by_kind: BTreeMap<DiskMapDiagnosticKind, u64>,
    samples: Vec<DiskMapDiagnosticSample>,
    sequence: u64,
}

impl DiskMapDiagnostics {
    fn new(limit: usize) -> Self {
        Self {
            limit,
            total: 0,
            counts_by_kind: BTreeMap::new(),
            samples: Vec::new(),
            sequence: 0,
        }
    }

    fn push(&mut self, diagnostic: DiskMapDiagnostic) {
        self.push_with_priority(diagnostic, false);
    }

    fn push_priority(&mut self, diagnostic: DiskMapDiagnostic) {
        self.push_with_priority(diagnostic, true);
    }

    fn extend(&mut self, diagnostics: Vec<DiskMapDiagnostic>) {
        for diagnostic in diagnostics {
            self.push(diagnostic);
        }
    }

    fn push_with_priority(&mut self, diagnostic: DiskMapDiagnostic, priority: bool) {
        self.total = self.total.saturating_add(1);
        *self.counts_by_kind.entry(diagnostic.kind).or_default() += 1;

        if self.limit == 0 {
            return;
        }

        let sample = DiskMapDiagnosticSample {
            priority,
            sequence: self.sequence,
            diagnostic,
        };
        self.sequence = self.sequence.saturating_add(1);

        if self.samples.len() < self.limit {
            self.samples.push(sample);
            return;
        }

        if priority && let Some(index) = self.samples.iter().rposition(|sample| !sample.priority) {
            self.samples[index] = sample;
        }
    }

    fn finish(self, report: &mut DiskMapReport) {
        let mut samples = self.samples;
        samples.sort_by(|left, right| {
            right
                .priority
                .cmp(&left.priority)
                .then_with(|| left.diagnostic.cmp(&right.diagnostic))
                .then_with(|| left.sequence.cmp(&right.sequence))
        });
        report.diagnostics = samples
            .into_iter()
            .map(|sample| sample.diagnostic)
            .collect();
        let retained = report.diagnostics.len() as u64;
        report.diagnostic_summary = DiskMapDiagnosticSummary {
            total: self.total,
            retained,
            truncated: self.total.saturating_sub(retained),
            by_kind: self
                .counts_by_kind
                .into_iter()
                .map(|(kind, count)| DiskMapDiagnosticKindSummary { kind, count })
                .collect(),
        };
    }
}

#[derive(Debug)]
struct DiskMapDiagnosticSample {
    priority: bool,
    sequence: u64,
    diagnostic: DiskMapDiagnostic,
}

struct DiskMapRootInspection<'a, W>
where
    W: DiskMapWalker,
{
    request: &'a DiskMapRequest,
    cancellation: &'a ScanCancellationToken,
    state: &'a mut DiskMapInspectionState,
    walker: &'a W,
}

impl<'a, W> DiskMapRootInspection<'a, W>
where
    W: DiskMapWalker,
{
    fn inspect(
        &mut self,
        root: &Path,
        provenance: EstimateProvenance,
        fallback_reason: Option<String>,
    ) -> Result<DiskMapUniqueFiles> {
        if let Some(reason) = &fallback_reason {
            self.state.diagnostics.push_priority(DiskMapDiagnostic::new(
                DiskMapDiagnosticKind::Fallback,
                root.to_path_buf(),
                reason.clone(),
            ));
        }

        let metadata = match self.walker.symlink_metadata(root) {
            Ok(metadata) => metadata,
            Err(RebeccaError::ScanFailed(failure)) if failure.kind == ScanFailureKind::NotFound => {
                push_root_skip(
                    &mut self.state.report,
                    root,
                    DiskMapDiagnosticKind::RootMissing,
                    "disk map root does not exist",
                    provenance,
                    &mut self.state.diagnostics,
                );
                return Ok(DiskMapUniqueFiles::default());
            }
            Err(err) => {
                push_root_skip(
                    &mut self.state.report,
                    root,
                    DiskMapDiagnosticKind::RootMetadataReadSkipped,
                    format!("disk map root metadata could not be read: {err}"),
                    provenance,
                    &mut self.state.diagnostics,
                );
                return Ok(DiskMapUniqueFiles::default());
            }
        };

        if self.walker.is_reparse_like(root, &metadata) {
            push_root_skip(
                &mut self.state.report,
                root,
                DiskMapDiagnosticKind::ReparsePointSkipped,
                "disk map root is a symlink or reparse point",
                provenance,
                &mut self.state.diagnostics,
            );
            return Ok(DiskMapUniqueFiles::default());
        }

        let mut semantic_caveats = DiskMapSemanticCaveats::default();
        let max_depth = self.request.max_depth.unwrap_or(usize::MAX);
        let root_result = match self.walker.metadata_kind(&metadata) {
            DiskMapMetadataKind::File => {
                let semantics = self.walker.metadata_semantics(&metadata);
                semantic_caveats.record(semantics);
                let result = DiskMapTraversalResult::file(
                    self.walker.metadata_len(&metadata),
                    self.walker.metadata_allocated_len(&metadata),
                    semantics,
                );
                self.state.groups.record_file(
                    root,
                    0,
                    result.metrics.logical_bytes,
                    result.metrics.allocated_bytes,
                    self.walker.metadata_modified_time(&metadata),
                    semantics,
                );
                let entry_provenance = estimate_provenance_with_entry_semantics(
                    &provenance,
                    semantics,
                    DiskMapEntryKind::File,
                );
                push_portable_entry(
                    root,
                    root.to_path_buf(),
                    DiskMapEntryKind::File,
                    0,
                    result.metrics,
                    &entry_provenance,
                    &mut self.state.top_entries,
                );
                result
            }
            DiskMapMetadataKind::Directory => match (DiskMapTraversal {
                root,
                cancellation: self.cancellation,
                max_depth,
                estimate_provenance: &provenance,
                top_entries: &mut self.state.top_entries,
                diagnostics: &mut self.state.diagnostics,
                walker: self.walker,
                semantic_caveats: &mut semantic_caveats,
                groups: &mut self.state.groups,
            })
            .inspect_root_directory()
            {
                Ok(metrics) => metrics,
                Err(err @ RebeccaError::OperationCancelled(_)) => return Err(err),
                Err(err) => {
                    push_root_skip(
                        &mut self.state.report,
                        root,
                        DiskMapDiagnosticKind::DirectoryReadSkipped,
                        format!("disk map root directory could not be read: {err}"),
                        provenance,
                        &mut self.state.diagnostics,
                    );
                    return Ok(DiskMapUniqueFiles::default());
                }
            },
            DiskMapMetadataKind::Other => DiskMapTraversalResult::default(),
        };

        let provenance = semantic_caveats.apply_to_root_provenance(provenance);
        self.state.report.totals.add(root_result.metrics);
        self.state.report.roots.push(DiskMapRoot {
            path: root.to_path_buf(),
            status: DiskMapRootStatus::Scanned,
            metrics: root_result.metrics,
            estimate_source: EstimateSource::FreshScan,
            estimate_provenance: provenance,
            reason: None,
        });
        Ok(root_result.unique_files)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiskMapMetadataKind {
    File,
    Directory,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct DiskMapFileIdentity {
    volume_serial_number: u64,
    file_index: u64,
}

impl DiskMapFileIdentity {
    pub(crate) const fn new(volume_serial_number: u64, file_index: u64) -> Self {
        Self {
            volume_serial_number,
            file_index,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct DiskMapMetadataSemantics {
    compressed: bool,
    sparse: bool,
    hardlink_count: Option<u32>,
    file_identity: Option<DiskMapFileIdentity>,
    reparse_like: bool,
}

impl DiskMapMetadataSemantics {
    pub(crate) fn with_file_identity(file_identity: DiskMapFileIdentity) -> Self {
        Self {
            file_identity: Some(file_identity),
            ..Self::default()
        }
    }

    fn with_reparse_like(mut self) -> Self {
        self.reparse_like = true;
        self
    }

    fn is_hardlinked(self) -> bool {
        self.hardlink_count.is_some_and(|count| count > 1)
    }
}

#[derive(Debug, Clone, Copy)]
struct DiskMapUniqueFileBytes {
    logical_bytes: u64,
    allocated_bytes: Option<u64>,
}

#[derive(Debug, Clone)]
struct DiskMapUniqueFiles {
    files: BTreeMap<DiskMapFileIdentity, DiskMapUniqueFileBytes>,
    unidentified_files: u64,
    unique_logical_bytes: u64,
    unique_allocated_bytes: Option<u64>,
}

impl Default for DiskMapUniqueFiles {
    fn default() -> Self {
        Self {
            files: BTreeMap::new(),
            unidentified_files: 0,
            unique_logical_bytes: 0,
            unique_allocated_bytes: Some(0),
        }
    }
}

impl DiskMapUniqueFiles {
    fn unavailable_for_files(files: u64) -> Self {
        Self {
            unidentified_files: files,
            ..Self::default()
        }
    }

    fn record_file(
        &mut self,
        identity: Option<DiskMapFileIdentity>,
        logical_bytes: u64,
        allocated_bytes: Option<u64>,
    ) {
        let Some(identity) = identity else {
            self.unidentified_files = self.unidentified_files.saturating_add(1);
            return;
        };

        if self.files.contains_key(&identity) {
            return;
        }

        self.files.insert(
            identity,
            DiskMapUniqueFileBytes {
                logical_bytes,
                allocated_bytes,
            },
        );
        self.unique_logical_bytes = self.unique_logical_bytes.saturating_add(logical_bytes);
        self.unique_allocated_bytes = match (self.unique_allocated_bytes, allocated_bytes) {
            (Some(left), Some(right)) => Some(left.saturating_add(right)),
            _ => None,
        };
    }

    fn merge(&mut self, other: Self) {
        self.unidentified_files = self
            .unidentified_files
            .saturating_add(other.unidentified_files);
        for (identity, bytes) in other.files {
            self.record_file(Some(identity), bytes.logical_bytes, bytes.allocated_bytes);
        }
    }

    fn apply_to_metrics(&self, metrics: &mut DiskMapMetrics) {
        metrics.unique_logical_bytes =
            (self.unidentified_files == 0).then_some(self.unique_logical_bytes);
        metrics.unique_allocated_bytes = if self.unidentified_files == 0 {
            self.unique_allocated_bytes
        } else {
            None
        };
    }
}

#[derive(Debug, Clone, Default)]
struct DiskMapTraversalResult {
    metrics: DiskMapMetrics,
    unique_files: DiskMapUniqueFiles,
}

impl DiskMapTraversalResult {
    fn file(
        logical_bytes: u64,
        allocated_bytes: Option<u64>,
        semantics: DiskMapMetadataSemantics,
    ) -> Self {
        let mut unique_files = DiskMapUniqueFiles::default();
        unique_files.record_file(semantics.file_identity, logical_bytes, allocated_bytes);
        let mut metrics = file_metrics(logical_bytes, allocated_bytes);
        unique_files.apply_to_metrics(&mut metrics);
        Self {
            metrics,
            unique_files,
        }
    }

    fn add_child(&mut self, child: Self) {
        self.metrics.add(child.metrics);
        self.unique_files.merge(child.unique_files);
        self.unique_files.apply_to_metrics(&mut self.metrics);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct DiskMapGroupMapKey {
    kind: DiskMapGroupKind,
    key: String,
}

#[derive(Debug, Clone)]
struct DiskMapGroupAccumulator {
    kind: DiskMapGroupKind,
    key: String,
    label: String,
    metrics: DiskMapMetrics,
    unique_files: DiskMapUniqueFiles,
}

impl DiskMapGroupAccumulator {
    fn new(kind: DiskMapGroupKind, key: String, label: String) -> Self {
        Self {
            kind,
            key,
            label,
            metrics: DiskMapMetrics::default(),
            unique_files: DiskMapUniqueFiles::default(),
        }
    }

    fn record_file(
        &mut self,
        logical_bytes: u64,
        allocated_bytes: Option<u64>,
        semantics: DiskMapMetadataSemantics,
    ) {
        self.metrics
            .add(file_metrics(logical_bytes, allocated_bytes));
        self.unique_files
            .record_file(semantics.file_identity, logical_bytes, allocated_bytes);
        self.unique_files.apply_to_metrics(&mut self.metrics);
    }

    fn merge(&mut self, other: Self) {
        self.metrics.add(other.metrics);
        self.unique_files.merge(other.unique_files);
        self.unique_files.apply_to_metrics(&mut self.metrics);
    }

    fn into_group(self) -> DiskMapGroup {
        DiskMapGroup {
            kind: self.kind,
            key: self.key,
            label: self.label,
            metrics: self.metrics,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct DiskMapGroupCollector {
    kinds: Vec<DiskMapGroupKind>,
    limit: usize,
    now: SystemTime,
    sort: DiskMapSortField,
    groups: BTreeMap<DiskMapGroupMapKey, DiskMapGroupAccumulator>,
}

impl DiskMapGroupCollector {
    pub(crate) fn new(
        mut kinds: Vec<DiskMapGroupKind>,
        limit: usize,
        now: SystemTime,
        sort: DiskMapSortField,
    ) -> Self {
        kinds.sort();
        kinds.dedup();
        Self {
            kinds,
            limit,
            now,
            sort,
            groups: BTreeMap::new(),
        }
    }

    pub(crate) fn now(&self) -> SystemTime {
        self.now
    }

    pub(crate) fn record_file(
        &mut self,
        path: &Path,
        depth: usize,
        logical_bytes: u64,
        allocated_bytes: Option<u64>,
        modified_time: Option<SystemTime>,
        semantics: DiskMapMetadataSemantics,
    ) {
        if self.kinds.is_empty() || self.limit == 0 {
            return;
        }

        for kind in self.kinds.clone() {
            let (key, label) = match kind {
                DiskMapGroupKind::Extension => disk_map_extension_group(path),
                DiskMapGroupKind::Depth => disk_map_depth_group(depth),
                DiskMapGroupKind::Age => disk_map_age_group(modified_time, self.now),
            };
            let map_key = DiskMapGroupMapKey {
                kind,
                key: key.clone(),
            };
            self.groups
                .entry(map_key)
                .or_insert_with(|| DiskMapGroupAccumulator::new(kind, key, label))
                .record_file(logical_bytes, allocated_bytes, semantics);
        }
    }

    pub(crate) fn merge(&mut self, other: Self) {
        for (map_key, accumulator) in other.groups {
            match self.groups.entry(map_key) {
                std::collections::btree_map::Entry::Occupied(mut entry) => {
                    entry.get_mut().merge(accumulator);
                }
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(accumulator);
                }
            }
        }
    }

    pub(crate) fn finish(self) -> Vec<DiskMapGroup> {
        let sort = self.sort;
        let mut groups = self
            .groups
            .into_values()
            .map(DiskMapGroupAccumulator::into_group)
            .collect::<Vec<_>>();
        groups.sort_by(|left, right| {
            sort.metrics_value(&right.metrics)
                .cmp(&sort.metrics_value(&left.metrics))
                .then_with(|| right.metrics.logical_bytes.cmp(&left.metrics.logical_bytes))
                .then_with(|| right.metrics.files.cmp(&left.metrics.files))
                .then_with(|| left.kind.cmp(&right.kind))
                .then_with(|| left.key.cmp(&right.key))
        });
        groups.truncate(self.limit);
        groups
    }
}

fn disk_map_extension_group(path: &Path) -> (String, String) {
    path.extension()
        .and_then(|extension| extension.to_str())
        .filter(|extension| !extension.is_empty())
        .map(|extension| {
            let key = format!(".{}", extension.to_ascii_lowercase());
            (key.clone(), key)
        })
        .unwrap_or_else(|| ("[no-extension]".to_string(), "No extension".to_string()))
}

fn disk_map_depth_group(depth: usize) -> (String, String) {
    (format!("depth-{depth}"), format!("Depth {depth}"))
}

fn disk_map_age_group(modified_time: Option<SystemTime>, now: SystemTime) -> (String, String) {
    let Some(modified_time) = modified_time else {
        return (
            "modified-unknown".to_string(),
            "Modified time unknown".to_string(),
        );
    };
    let age = now
        .duration_since(modified_time)
        .unwrap_or_else(|_| Duration::from_secs(0));
    let age_days = age.as_secs() / 86_400;
    match age_days {
        0..=7 => (
            "modified-7d".to_string(),
            "Modified within 7 days".to_string(),
        ),
        8..=30 => (
            "modified-30d".to_string(),
            "Modified within 30 days".to_string(),
        ),
        31..=90 => (
            "modified-90d".to_string(),
            "Modified within 90 days".to_string(),
        ),
        91..=365 => (
            "modified-365d".to_string(),
            "Modified within 365 days".to_string(),
        ),
        _ => (
            "modified-older".to_string(),
            "Modified more than 365 days ago".to_string(),
        ),
    }
}

#[derive(Debug, Clone, Default)]
struct DiskMapSemanticCaveats {
    compressed_files: u64,
    sparse_files: u64,
    hardlinked_files: u64,
    reparse_entries: u64,
}

impl DiskMapSemanticCaveats {
    fn record(&mut self, semantics: DiskMapMetadataSemantics) {
        if semantics.compressed {
            self.compressed_files = self.compressed_files.saturating_add(1);
        }
        if semantics.sparse {
            self.sparse_files = self.sparse_files.saturating_add(1);
        }
        if semantics.is_hardlinked() {
            self.hardlinked_files = self.hardlinked_files.saturating_add(1);
        }
        if semantics.reparse_like {
            self.reparse_entries = self.reparse_entries.saturating_add(1);
        }
    }

    fn apply_to_root_provenance(&self, mut provenance: EstimateProvenance) -> EstimateProvenance {
        if self.compressed_files > 0 {
            provenance.estimate_caveats.push(disk_map_caveat(
                "windows-native-compressed-file",
                format!(
                    "{} compressed file(s) were seen; allocated_bytes uses Windows-reported allocation and may be lower than logical_bytes",
                    self.compressed_files
                ),
            ));
        }
        if self.sparse_files > 0 {
            provenance.estimate_caveats.push(disk_map_caveat(
                "windows-native-sparse-file",
                format!(
                    "{} sparse file(s) were seen; allocated_bytes uses Windows-reported allocation and may be lower than logical_bytes",
                    self.sparse_files
                ),
            ));
        }
        if self.hardlinked_files > 0 {
            provenance.estimate_caveats.push(disk_map_caveat(
                "windows-native-hardlink-file",
                format!(
                    "{} file path(s) reported multiple hard links; path-ranked bytes may overstate unique physical bytes when another link points to the same file",
                    self.hardlinked_files
                ),
            ));
        }
        if self.reparse_entries > 0 {
            provenance.estimate_caveats.push(disk_map_caveat(
                "windows-native-reparse-skipped",
                format!(
                    "{} reparse point(s) were skipped; target allocation is not included",
                    self.reparse_entries
                ),
            ));
        }

        provenance
    }
}

fn estimate_provenance_with_entry_semantics(
    provenance: &EstimateProvenance,
    semantics: DiskMapMetadataSemantics,
    kind: DiskMapEntryKind,
) -> EstimateProvenance {
    let mut provenance = provenance.clone();
    if kind == DiskMapEntryKind::File && semantics.compressed {
        provenance.estimate_caveats.push(disk_map_caveat(
            "windows-native-compressed-file",
            "file is compressed; allocated_bytes uses Windows-reported allocation and may be lower than logical_bytes",
        ));
    }
    if kind == DiskMapEntryKind::File && semantics.sparse {
        provenance.estimate_caveats.push(disk_map_caveat(
            "windows-native-sparse-file",
            "file is sparse; allocated_bytes uses Windows-reported allocation and may be lower than logical_bytes",
        ));
    }
    if kind == DiskMapEntryKind::File && semantics.is_hardlinked() {
        let link_count = semantics.hardlink_count.unwrap_or(0);
        provenance.estimate_caveats.push(disk_map_caveat(
            "windows-native-hardlink-file",
            format!(
                "file reports {link_count} hard links; path-ranked bytes may overstate unique physical bytes when another link points to the same file"
            ),
        ));
    }
    if semantics.reparse_like {
        provenance.estimate_caveats.push(disk_map_caveat(
            "windows-native-reparse-skipped",
            "reparse point was skipped; target allocation is not included",
        ));
    }

    provenance
}

fn disk_map_caveat(code: impl Into<String>, message: impl Into<String>) -> ScanEstimateCaveat {
    ScanEstimateCaveat {
        code: code.into(),
        message: message.into(),
    }
}

#[derive(Debug)]
struct DiskMapWalkerEntry<M> {
    path: PathBuf,
    metadata: Option<M>,
}

impl<M> DiskMapWalkerEntry<M> {
    fn path(path: PathBuf) -> Self {
        Self {
            path,
            metadata: None,
        }
    }

    fn with_metadata(path: PathBuf, metadata: M) -> Self {
        Self {
            path,
            metadata: Some(metadata),
        }
    }
}

trait DiskMapWalker {
    type Metadata;
    type ReadDir;

    fn symlink_metadata(&self, path: &Path) -> Result<Self::Metadata>;
    fn is_reparse_like(&self, path: &Path, metadata: &Self::Metadata) -> bool;
    fn metadata_len(&self, metadata: &Self::Metadata) -> u64;
    fn metadata_allocated_len(&self, metadata: &Self::Metadata) -> Option<u64>;
    fn metadata_modified_time(&self, metadata: &Self::Metadata) -> Option<SystemTime>;
    fn metadata_semantics(&self, metadata: &Self::Metadata) -> DiskMapMetadataSemantics;
    fn metadata_kind(&self, metadata: &Self::Metadata) -> DiskMapMetadataKind;
    fn read_dir(&self, path: &Path, cancellation: &ScanCancellationToken) -> Result<Self::ReadDir>;
    fn next_entry(
        &self,
        entries: &mut Self::ReadDir,
    ) -> Option<Result<DiskMapWalkerEntry<Self::Metadata>>>;
}

#[derive(Debug, Default)]
struct FsPortableDiskMapWalker;

impl DiskMapWalker for FsPortableDiskMapWalker {
    type Metadata = std::fs::Metadata;
    type ReadDir = std::fs::ReadDir;

    fn symlink_metadata(&self, path: &Path) -> Result<Self::Metadata> {
        std::fs::symlink_metadata(path).map_err(|err| {
            RebeccaError::ScanFailed(crate::error::ScanFailure::from_io(
                path,
                crate::error::ScanFailurePhase::EntryMetadata,
                &err,
            ))
        })
    }

    fn is_reparse_like(&self, _path: &Path, metadata: &Self::Metadata) -> bool {
        is_reparse_like(metadata)
    }

    fn metadata_len(&self, metadata: &Self::Metadata) -> u64 {
        metadata.len()
    }

    fn metadata_allocated_len(&self, _metadata: &Self::Metadata) -> Option<u64> {
        None
    }

    fn metadata_modified_time(&self, metadata: &Self::Metadata) -> Option<SystemTime> {
        metadata.modified().ok()
    }

    fn metadata_semantics(&self, _metadata: &Self::Metadata) -> DiskMapMetadataSemantics {
        DiskMapMetadataSemantics::default()
    }

    fn metadata_kind(&self, metadata: &Self::Metadata) -> DiskMapMetadataKind {
        if metadata.is_file() {
            DiskMapMetadataKind::File
        } else if metadata.is_dir() {
            DiskMapMetadataKind::Directory
        } else {
            DiskMapMetadataKind::Other
        }
    }

    fn read_dir(
        &self,
        path: &Path,
        _cancellation: &ScanCancellationToken,
    ) -> Result<Self::ReadDir> {
        std::fs::read_dir(path).map_err(|err| {
            RebeccaError::ScanFailed(crate::error::ScanFailure::from_io(
                path,
                crate::error::ScanFailurePhase::DirectoryWalk,
                &err,
            ))
        })
    }

    fn next_entry(
        &self,
        entries: &mut Self::ReadDir,
    ) -> Option<Result<DiskMapWalkerEntry<Self::Metadata>>> {
        entries.next().map(|entry| {
            entry
                .map(|entry| DiskMapWalkerEntry::path(entry.path()))
                .map_err(|err| {
                    RebeccaError::ScanFailed(crate::error::ScanFailure::from_io(
                        Path::new("<directory-entry>"),
                        crate::error::ScanFailurePhase::DirectoryWalk,
                        &err,
                    ))
                })
        })
    }
}

#[cfg(windows)]
#[derive(Debug, Default)]
struct WindowsNativeDiskMapWalker;

#[cfg(windows)]
#[derive(Debug, Clone)]
struct WindowsNativeDiskMapMetadata {
    kind: DiskMapMetadataKind,
    len: u64,
    allocated_len: Option<u64>,
    modified_time: Option<SystemTime>,
    semantics: DiskMapMetadataSemantics,
    reparse_like: bool,
}

#[cfg(windows)]
impl WindowsNativeDiskMapMetadata {
    fn from_fs_metadata(path: &Path, metadata: &std::fs::Metadata) -> Self {
        let kind = if metadata.is_file() {
            DiskMapMetadataKind::File
        } else if metadata.is_dir() {
            DiskMapMetadataKind::Directory
        } else {
            DiskMapMetadataKind::Other
        };
        let reparse_like = is_reparse_like(metadata);
        let native_semantics =
            crate::scan::windows_native::WindowsNativeFileSemantics::from_path_and_attributes(
                path,
                match kind {
                    DiskMapMetadataKind::File => {
                        crate::scan::windows_native::WindowsNativeEntryKind::File
                    }
                    DiskMapMetadataKind::Directory | DiskMapMetadataKind::Other => {
                        crate::scan::windows_native::WindowsNativeEntryKind::Directory
                    }
                },
                metadata.file_attributes(),
                reparse_like,
            );
        Self {
            allocated_len: match kind {
                DiskMapMetadataKind::File => crate::scan::windows_native::file_allocated_size(path),
                DiskMapMetadataKind::Directory | DiskMapMetadataKind::Other => None,
            },
            kind,
            len: metadata.len(),
            modified_time: metadata.modified().ok(),
            semantics: DiskMapMetadataSemantics::from(native_semantics),
            reparse_like,
        }
    }

    fn from_native_entry(entry: &crate::scan::windows_native::WindowsNativeDirectoryEntry) -> Self {
        Self {
            kind: match entry.kind() {
                crate::scan::windows_native::WindowsNativeEntryKind::File => {
                    DiskMapMetadataKind::File
                }
                crate::scan::windows_native::WindowsNativeEntryKind::Directory => {
                    DiskMapMetadataKind::Directory
                }
            },
            len: entry.file_size(),
            allocated_len: entry.allocated_size(),
            modified_time: entry.modified_time(),
            semantics: DiskMapMetadataSemantics::from(entry.semantics()),
            reparse_like: entry.is_reparse_like(),
        }
    }
}

#[cfg(windows)]
impl From<crate::scan::windows_native::WindowsNativeFileSemantics> for DiskMapMetadataSemantics {
    fn from(value: crate::scan::windows_native::WindowsNativeFileSemantics) -> Self {
        Self {
            compressed: value.compressed,
            sparse: value.sparse,
            hardlink_count: value.hardlink_count,
            file_identity: value.file_id.map(|file_id| DiskMapFileIdentity {
                volume_serial_number: file_id.volume_serial_number as u64,
                file_index: file_id.file_index,
            }),
            reparse_like: false,
        }
    }
}

#[cfg(windows)]
impl DiskMapWalker for WindowsNativeDiskMapWalker {
    type Metadata = WindowsNativeDiskMapMetadata;
    type ReadDir = std::vec::IntoIter<crate::scan::windows_native::WindowsNativeDirectoryEntry>;

    fn symlink_metadata(&self, path: &Path) -> Result<Self::Metadata> {
        std::fs::symlink_metadata(path)
            .map(|metadata| WindowsNativeDiskMapMetadata::from_fs_metadata(path, &metadata))
            .map_err(|err| {
                RebeccaError::ScanFailed(crate::error::ScanFailure::from_io(
                    path,
                    crate::error::ScanFailurePhase::EntryMetadata,
                    &err,
                ))
            })
    }

    fn is_reparse_like(&self, _path: &Path, metadata: &Self::Metadata) -> bool {
        metadata.reparse_like
    }

    fn metadata_len(&self, metadata: &Self::Metadata) -> u64 {
        metadata.len
    }

    fn metadata_allocated_len(&self, metadata: &Self::Metadata) -> Option<u64> {
        metadata.allocated_len
    }

    fn metadata_modified_time(&self, metadata: &Self::Metadata) -> Option<SystemTime> {
        metadata.modified_time
    }

    fn metadata_semantics(&self, metadata: &Self::Metadata) -> DiskMapMetadataSemantics {
        if metadata.reparse_like {
            metadata.semantics.with_reparse_like()
        } else {
            metadata.semantics
        }
    }

    fn metadata_kind(&self, metadata: &Self::Metadata) -> DiskMapMetadataKind {
        metadata.kind
    }

    fn read_dir(&self, path: &Path, cancellation: &ScanCancellationToken) -> Result<Self::ReadDir> {
        crate::scan::windows_native::read_directory_entries(path, cancellation).map(Vec::into_iter)
    }

    fn next_entry(
        &self,
        entries: &mut Self::ReadDir,
    ) -> Option<Result<DiskMapWalkerEntry<Self::Metadata>>> {
        entries.next().map(|entry| {
            Ok(DiskMapWalkerEntry::with_metadata(
                entry.path().to_path_buf(),
                WindowsNativeDiskMapMetadata::from_native_entry(&entry),
            ))
        })
    }
}

struct DiskMapTraversal<'a, W>
where
    W: DiskMapWalker,
{
    root: &'a Path,
    cancellation: &'a ScanCancellationToken,
    max_depth: usize,
    estimate_provenance: &'a EstimateProvenance,
    top_entries: &'a mut DiskMapTopEntries,
    diagnostics: &'a mut DiskMapDiagnostics,
    walker: &'a W,
    semantic_caveats: &'a mut DiskMapSemanticCaveats,
    groups: &'a mut DiskMapGroupCollector,
}

impl<'a, W> DiskMapTraversal<'a, W>
where
    W: DiskMapWalker,
{
    fn inspect_root_directory(&mut self) -> Result<DiskMapTraversalResult> {
        let child_entries = self.read_sorted_child_entries(self.root)?;

        let mut result = DiskMapTraversalResult::default();
        for child in child_entries {
            check_cancelled(self.cancellation)?;
            let child_result = self.inspect_node(child, 1)?;
            result.add_child(child_result);
        }
        result.unique_files.apply_to_metrics(&mut result.metrics);
        Ok(result)
    }

    fn inspect_node(
        &mut self,
        entry: DiskMapWalkerEntry<W::Metadata>,
        depth: usize,
    ) -> Result<DiskMapTraversalResult> {
        let path = entry.path;
        let metadata = match entry.metadata {
            Some(metadata) => metadata,
            None => match self.walker.symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(err) => {
                    self.diagnostics.push(DiskMapDiagnostic::new(
                        DiskMapDiagnosticKind::MetadataReadSkipped,
                        path.clone(),
                        format!("disk map entry metadata could not be read: {err}"),
                    ));
                    return Ok(DiskMapTraversalResult::default());
                }
            },
        };
        let semantics = self.walker.metadata_semantics(&metadata);
        if self.walker.is_reparse_like(&path, &metadata) {
            let semantics = semantics.with_reparse_like();
            self.semantic_caveats.record(semantics);
            self.diagnostics.push(DiskMapDiagnostic::new(
                DiskMapDiagnosticKind::ReparsePointSkipped,
                path.clone(),
                "disk map entry is a symlink or reparse point",
            ));
            let entry_provenance = estimate_provenance_with_entry_semantics(
                self.estimate_provenance,
                semantics,
                DiskMapEntryKind::Other,
            );
            let result = DiskMapTraversalResult::default();
            self.push_entry_if_visible(
                &path,
                DiskMapEntryKind::Other,
                depth,
                result.metrics,
                &entry_provenance,
            );
            return Ok(result);
        }

        match self.walker.metadata_kind(&metadata) {
            DiskMapMetadataKind::File => {
                self.semantic_caveats.record(semantics);
                let result = DiskMapTraversalResult::file(
                    self.walker.metadata_len(&metadata),
                    self.walker.metadata_allocated_len(&metadata),
                    semantics,
                );
                self.groups.record_file(
                    &path,
                    depth,
                    result.metrics.logical_bytes,
                    result.metrics.allocated_bytes,
                    self.walker.metadata_modified_time(&metadata),
                    semantics,
                );
                let entry_provenance = estimate_provenance_with_entry_semantics(
                    self.estimate_provenance,
                    semantics,
                    DiskMapEntryKind::File,
                );
                self.push_entry_if_visible(
                    &path,
                    DiskMapEntryKind::File,
                    depth,
                    result.metrics,
                    &entry_provenance,
                );
                Ok(result)
            }
            DiskMapMetadataKind::Directory => self.inspect_directory_node(&path, depth),
            DiskMapMetadataKind::Other => {
                let result = DiskMapTraversalResult::default();
                self.push_entry_if_visible(
                    &path,
                    DiskMapEntryKind::Other,
                    depth,
                    result.metrics,
                    self.estimate_provenance,
                );
                Ok(result)
            }
        }
    }

    fn inspect_directory_node(
        &mut self,
        path: &Path,
        depth: usize,
    ) -> Result<DiskMapTraversalResult> {
        let child_entries = match self.read_sorted_child_entries(path) {
            Ok(child_paths) => child_paths,
            Err(err @ RebeccaError::OperationCancelled(_)) => return Err(err),
            Err(err) => {
                self.diagnostics.push(DiskMapDiagnostic::new(
                    DiskMapDiagnosticKind::DirectoryReadSkipped,
                    path.to_path_buf(),
                    format!("disk map directory could not be read: {err}"),
                ));
                return Ok(DiskMapTraversalResult::default());
            }
        };

        let mut result = DiskMapTraversalResult {
            metrics: DiskMapMetrics {
                logical_bytes: 0,
                allocated_bytes: None,
                unique_logical_bytes: Some(0),
                unique_allocated_bytes: Some(0),
                files: 0,
                directories: 1,
            },
            unique_files: DiskMapUniqueFiles::default(),
        };
        for child in child_entries {
            check_cancelled(self.cancellation)?;
            let child_result = self.inspect_node(child, depth.saturating_add(1))?;
            result.add_child(child_result);
        }
        result.unique_files.apply_to_metrics(&mut result.metrics);

        self.push_entry_if_visible(
            path,
            DiskMapEntryKind::Directory,
            depth,
            result.metrics,
            self.estimate_provenance,
        );
        Ok(result)
    }

    fn read_sorted_child_entries(
        &mut self,
        path: &Path,
    ) -> Result<Vec<DiskMapWalkerEntry<W::Metadata>>> {
        let mut entries = self.walker.read_dir(path, self.cancellation)?;
        let mut child_entries = Vec::new();
        while let Some(entry) = self.walker.next_entry(&mut entries) {
            check_cancelled(self.cancellation)?;
            match entry {
                Ok(entry) => child_entries.push(entry),
                Err(err @ RebeccaError::OperationCancelled(_)) => return Err(err),
                Err(err) => {
                    self.diagnostics.push(DiskMapDiagnostic::new(
                        DiskMapDiagnosticKind::DirectoryEntryReadSkipped,
                        path.to_path_buf(),
                        format!("disk map directory entry could not be read: {err}"),
                    ));
                }
            }
        }
        child_entries.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(child_entries)
    }

    fn push_entry_if_visible(
        &mut self,
        path: &Path,
        kind: DiskMapEntryKind,
        depth: usize,
        metrics: DiskMapMetrics,
        estimate_provenance: &EstimateProvenance,
    ) {
        push_portable_entry_if_visible(
            self.root,
            path.to_path_buf(),
            kind,
            depth,
            metrics,
            self.max_depth,
            estimate_provenance,
            self.top_entries,
        );
    }
}

fn inspect_root(
    root: &Path,
    request: &DiskMapRequest,
    cancellation: &ScanCancellationToken,
    scan_engine: &ScanEngine,
    state: &mut DiskMapInspectionState,
) -> Result<DiskMapUniqueFiles> {
    let walker = FsPortableDiskMapWalker;
    if request.scan_backend == ScanBackendKind::WindowsNative {
        match inspect_windows_native_root(root, request, cancellation, state) {
            Ok(unique_files) => return Ok(unique_files),
            Err(err) if disk_map_backend_error_can_fallback(&err) => {
                let fallback_reason =
                    format!("windows-native disk-map inventory was unavailable: {err}");
                return DiskMapRootInspection {
                    request,
                    cancellation,
                    state,
                    walker: &walker,
                }
                .inspect(
                    root,
                    portable_estimate_provenance(Some(fallback_reason.clone())),
                    Some(fallback_reason),
                );
            }
            Err(err) => return Err(err),
        }
    }

    if request.scan_backend == ScanBackendKind::WindowsNtfsMftExperimental {
        match scan_engine.inspect_windows_ntfs_mft_disk_map(
            root,
            state.backend_options(request),
            cancellation,
        ) {
            Ok(root_map) => {
                let unique_files =
                    DiskMapUniqueFiles::unavailable_for_files(root_map.metrics.files);
                push_backend_root(root, root_map, state);
                return Ok(unique_files);
            }
            Err(err) if disk_map_backend_error_can_fallback(&err) => {
                let fallback_reason = format!(
                    "windows-ntfs-mft-experimental disk-map inventory was unavailable: {err}"
                );
                return DiskMapRootInspection {
                    request,
                    cancellation,
                    state,
                    walker: &walker,
                }
                .inspect(
                    root,
                    portable_estimate_provenance(Some(fallback_reason.clone())),
                    Some(fallback_reason),
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
    DiskMapRootInspection {
        request,
        cancellation,
        state,
        walker: &walker,
    }
    .inspect(
        root,
        portable_estimate_provenance(fallback_reason.clone()),
        fallback_reason,
    )
}

#[cfg(windows)]
fn inspect_windows_native_root(
    root: &Path,
    request: &DiskMapRequest,
    cancellation: &ScanCancellationToken,
    state: &mut DiskMapInspectionState,
) -> Result<DiskMapUniqueFiles> {
    if let Some(reason) = crate::scan::windows_native::unsupported_path_reason(root) {
        return Err(RebeccaError::PlatformUnavailable(reason));
    }

    let walker = WindowsNativeDiskMapWalker;
    DiskMapRootInspection {
        request,
        cancellation,
        state,
        walker: &walker,
    }
    .inspect(
        root,
        EstimateProvenance::from_backend_confidence(
            ScanBackendKind::WindowsNative,
            ScanEstimateConfidence::Exact,
        ),
        None,
    )
}

#[cfg(not(windows))]
fn inspect_windows_native_root(
    _root: &Path,
    _request: &DiskMapRequest,
    _cancellation: &ScanCancellationToken,
    _state: &mut DiskMapInspectionState,
) -> Result<DiskMapUniqueFiles> {
    Err(RebeccaError::PlatformUnavailable(format!(
        "{} disk-map inventory is only available on Windows",
        ScanBackendKind::WindowsNative.label()
    )))
}

fn push_backend_root(
    root: &Path,
    root_map: DiskMapBackendRoot,
    state: &mut DiskMapInspectionState,
) {
    state.report.totals.add(root_map.metrics);
    for entry in root_map.top_entries {
        state.top_entries.push(entry);
    }
    state.groups.merge(root_map.groups);
    state.diagnostics.extend(root_map.diagnostics);
    state.report.roots.push(DiskMapRoot {
        path: root.to_path_buf(),
        status: DiskMapRootStatus::Scanned,
        metrics: root_map.metrics,
        estimate_source: EstimateSource::FreshScan,
        estimate_provenance: root_map.estimate_provenance,
        reason: None,
    });
}

fn file_metrics(logical_bytes: u64, allocated_bytes: Option<u64>) -> DiskMapMetrics {
    DiskMapMetrics {
        logical_bytes,
        allocated_bytes,
        unique_logical_bytes: None,
        unique_allocated_bytes: None,
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
    sort: DiskMapSortField,
    filter: DiskMapEntryFilter,
    heap: BinaryHeap<Reverse<DiskMapTopCandidate>>,
    sequence: u64,
}

impl DiskMapTopEntries {
    pub(crate) fn new(limit: usize, sort: DiskMapSortField, filter: DiskMapEntryFilter) -> Self {
        Self {
            limit,
            sort,
            filter,
            heap: BinaryHeap::with_capacity(limit),
            sequence: 0,
        }
    }

    pub(crate) fn push(&mut self, entry: DiskMapEntry) {
        if self.limit == 0 || !self.filter.matches(&entry) {
            return;
        }

        let candidate = DiskMapTopCandidate {
            rank: DiskMapTopRank::from_entry(&entry, self.sort),
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
    sort_value: u64,
    logical_bytes: u64,
    files: u64,
    directories: u64,
    reverse_path: Reverse<PathBuf>,
}

impl DiskMapTopRank {
    fn from_entry(entry: &DiskMapEntry, sort: DiskMapSortField) -> Self {
        Self {
            sort_value: sort.entry_value(entry),
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
    diagnostics: &mut DiskMapDiagnostics,
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
    diagnostics.push_priority(DiskMapDiagnostic::new(kind, root.to_path_buf(), detail));
}

pub(crate) struct DiskMapBackendRoot {
    pub(crate) metrics: DiskMapMetrics,
    pub(crate) top_entries: Vec<DiskMapEntry>,
    pub(crate) groups: DiskMapGroupCollector,
    pub(crate) diagnostics: Vec<DiskMapDiagnostic>,
    pub(crate) estimate_provenance: EstimateProvenance,
}

#[derive(Debug, Clone)]
pub(crate) struct DiskMapBackendOptions {
    pub(crate) top_limit: usize,
    pub(crate) top_sort: DiskMapSortField,
    pub(crate) entry_filter: DiskMapEntryFilter,
    pub(crate) max_depth: Option<usize>,
    pub(crate) group_kinds: Vec<DiskMapGroupKind>,
    pub(crate) group_limit: usize,
    pub(crate) group_now: SystemTime,
    pub(crate) group_sort: DiskMapSortField,
}

impl DiskMapBackendOptions {
    pub(crate) fn group_collector(&self) -> DiskMapGroupCollector {
        DiskMapGroupCollector::new(
            self.group_kinds.clone(),
            self.group_limit,
            self.group_now,
            self.group_sort,
        )
    }
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
    W: DiskMapWalker,
{
    let mut state = DiskMapInspectionState::new(request);
    let mut unique_files = DiskMapUniqueFiles::default();

    for root in &request.roots {
        check_cancelled(cancellation)?;
        unique_files.merge(
            DiskMapRootInspection {
                request,
                cancellation,
                state: &mut state,
                walker,
            }
            .inspect(root, portable_estimate_provenance(None), None)?,
        );
    }

    Ok(state.finish(unique_files))
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
        allocated_len: Option<u64>,
        modified_time: Option<SystemTime>,
        semantics: DiskMapMetadataSemantics,
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
                    allocated_len: None,
                    modified_time: None,
                    semantics: DiskMapMetadataSemantics::default(),
                }),
            );
            self
        }

        fn with_identified_file(
            mut self,
            path: impl Into<PathBuf>,
            len: u64,
            allocated_len: Option<u64>,
            identity: DiskMapFileIdentity,
        ) -> Self {
            self.metadata.insert(
                path.into(),
                Ok(FakeMetadata {
                    kind: FakeMetadataKind::File,
                    len,
                    allocated_len,
                    modified_time: None,
                    semantics: DiskMapMetadataSemantics {
                        file_identity: Some(identity),
                        ..DiskMapMetadataSemantics::default()
                    },
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
                    allocated_len: None,
                    modified_time: None,
                    semantics: DiskMapMetadataSemantics::default(),
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
                    allocated_len: None,
                    modified_time: None,
                    semantics: DiskMapMetadataSemantics::default(),
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
                    allocated_len: None,
                    modified_time: None,
                    semantics: DiskMapMetadataSemantics::default(),
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

    impl DiskMapWalker for FakeDiskMapWalker {
        type Metadata = FakeMetadata;
        type ReadDir = VecDeque<FakeEntry>;

        fn symlink_metadata(&self, path: &Path) -> Result<Self::Metadata> {
            match self.metadata.get(path) {
                Some(Ok(metadata)) => Ok(metadata.clone()),
                Some(Err(message)) => Err(RebeccaError::ScanFailed(
                    crate::error::ScanFailure::from_io(
                        path,
                        crate::error::ScanFailurePhase::EntryMetadata,
                        &io::Error::other(*message),
                    ),
                )),
                None => Err(RebeccaError::ScanFailed(
                    crate::error::ScanFailure::from_io(
                        path,
                        crate::error::ScanFailurePhase::EntryMetadata,
                        &io::Error::new(io::ErrorKind::NotFound, "missing"),
                    ),
                )),
            }
        }

        fn is_reparse_like(&self, path: &Path, _metadata: &Self::Metadata) -> bool {
            self.reparse_paths.iter().any(|candidate| candidate == path)
        }

        fn metadata_len(&self, metadata: &Self::Metadata) -> u64 {
            metadata.len
        }

        fn metadata_allocated_len(&self, metadata: &Self::Metadata) -> Option<u64> {
            metadata.allocated_len
        }

        fn metadata_modified_time(&self, metadata: &Self::Metadata) -> Option<SystemTime> {
            metadata.modified_time
        }

        fn metadata_semantics(&self, metadata: &Self::Metadata) -> DiskMapMetadataSemantics {
            metadata.semantics
        }

        fn metadata_kind(&self, metadata: &Self::Metadata) -> DiskMapMetadataKind {
            match metadata.kind {
                FakeMetadataKind::File => DiskMapMetadataKind::File,
                FakeMetadataKind::Directory => DiskMapMetadataKind::Directory,
                FakeMetadataKind::Other => DiskMapMetadataKind::Other,
            }
        }

        fn read_dir(
            &self,
            path: &Path,
            _cancellation: &ScanCancellationToken,
        ) -> Result<Self::ReadDir> {
            match self.directories.get(path) {
                Some(Ok(entries)) => Ok(entries.clone()),
                Some(Err(message)) => Err(RebeccaError::ScanFailed(
                    crate::error::ScanFailure::from_io(
                        path,
                        crate::error::ScanFailurePhase::DirectoryWalk,
                        &io::Error::other(*message),
                    ),
                )),
                None => Err(RebeccaError::ScanFailed(
                    crate::error::ScanFailure::from_io(
                        path,
                        crate::error::ScanFailurePhase::DirectoryWalk,
                        &io::Error::new(io::ErrorKind::NotFound, "missing dir"),
                    ),
                )),
            }
        }

        fn next_entry(
            &self,
            entries: &mut Self::ReadDir,
        ) -> Option<Result<DiskMapWalkerEntry<Self::Metadata>>> {
            entries.pop_front().map(|entry| match entry {
                FakeEntry::Path(path) => Ok(DiskMapWalkerEntry::path(path)),
                FakeEntry::Error(message) => Err(RebeccaError::ScanFailed(
                    crate::error::ScanFailure::from_io(
                        Path::new("<fake-entry>"),
                        crate::error::ScanFailurePhase::DirectoryWalk,
                        &io::Error::other(message),
                    ),
                )),
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

    #[test]
    fn disk_map_file_identity_deduplicates_unique_bytes_across_siblings() {
        let root = PathBuf::from("C:\\root");
        let left = root.join("left");
        let right = root.join("right");
        let first = left.join("shared.bin");
        let second = right.join("shared.bin");
        let identity = DiskMapFileIdentity {
            volume_serial_number: 7,
            file_index: 42,
        };
        let walker = FakeDiskMapWalker::default()
            .with_directory(
                &root,
                [
                    FakeEntry::Path(left.clone()),
                    FakeEntry::Path(right.clone()),
                ],
            )
            .with_directory(&left, [FakeEntry::Path(first.clone())])
            .with_directory(&right, [FakeEntry::Path(second.clone())])
            .with_identified_file(&first, 4, Some(4096), identity)
            .with_identified_file(&second, 4, Some(4096), identity);

        let report = inspect_map_with_walker_for_test(
            &DiskMapRequest::new(vec![root.clone()]).with_top_limit(10),
            &ScanCancellationToken::new(),
            &walker,
        )
        .unwrap();

        assert_eq!(report.totals.logical_bytes, 8);
        assert_eq!(report.totals.allocated_bytes, Some(8192));
        assert_eq!(report.totals.unique_logical_bytes, Some(4));
        assert_eq!(report.totals.unique_allocated_bytes, Some(4096));
        assert_eq!(report.roots[0].metrics.unique_logical_bytes, Some(4));
        assert_eq!(report.roots[0].metrics.unique_allocated_bytes, Some(4096));
        assert!(report.top_entries.iter().any(|entry| {
            entry.path == first
                && entry.logical_bytes == 4
                && entry.unique_logical_bytes == Some(4)
                && entry.unique_allocated_bytes == Some(4096)
        }));
        assert!(report.top_entries.iter().any(|entry| {
            entry.path == left
                && entry.logical_bytes == 4
                && entry.unique_logical_bytes == Some(4)
                && entry.unique_allocated_bytes == Some(4096)
        }));
    }

    #[test]
    fn portable_map_bounds_raw_diagnostics_but_counts_all_failures() {
        let root = PathBuf::from("C:\\root");
        let failed_paths = (0..5)
            .map(|index| root.join(format!("missing-{index}.bin")))
            .collect::<Vec<_>>();
        let entries = failed_paths
            .iter()
            .cloned()
            .map(FakeEntry::Path)
            .collect::<Vec<_>>();
        let walker = failed_paths.iter().fold(
            FakeDiskMapWalker::default().with_directory(&root, entries),
            |walker, path| walker.with_metadata_error(path, "raced away"),
        );
        let report = inspect_map_with_walker_for_test(
            &DiskMapRequest::new(vec![root])
                .with_top_limit(0)
                .with_diagnostic_limit(2),
            &ScanCancellationToken::new(),
            &walker,
        )
        .unwrap();

        assert_eq!(report.diagnostics.len(), 2);
        assert_eq!(report.diagnostic_summary.total, 5);
        assert_eq!(report.diagnostic_summary.retained, 2);
        assert_eq!(report.diagnostic_summary.truncated, 3);
        assert_eq!(
            report.diagnostic_summary.by_kind,
            vec![DiskMapDiagnosticKindSummary {
                kind: DiskMapDiagnosticKind::MetadataReadSkipped,
                count: 5,
            }]
        );
    }

    #[test]
    fn disk_map_diagnostics_retains_priority_samples_when_full() {
        let child = PathBuf::from("C:\\root\\child");
        let fallback = PathBuf::from("C:\\root");
        let mut diagnostics = DiskMapDiagnostics::new(1);
        diagnostics.push(DiskMapDiagnostic::new(
            DiskMapDiagnosticKind::MetadataReadSkipped,
            child,
            "child failed",
        ));
        diagnostics.push_priority(DiskMapDiagnostic::new(
            DiskMapDiagnosticKind::Fallback,
            fallback.clone(),
            "backend fallback",
        ));
        let mut report = DiskMapReport::default();
        diagnostics.finish(&mut report);

        assert_eq!(report.diagnostics.len(), 1);
        assert_eq!(report.diagnostics[0].kind, DiskMapDiagnosticKind::Fallback);
        assert_eq!(report.diagnostics[0].path, fallback);
        assert_eq!(report.diagnostic_summary.total, 2);
        assert_eq!(report.diagnostic_summary.retained, 1);
        assert_eq!(report.diagnostic_summary.truncated, 1);
        assert_eq!(
            report
                .diagnostic_summary
                .by_kind
                .iter()
                .map(|summary| (summary.kind, summary.count))
                .collect::<Vec<_>>(),
            vec![
                (DiskMapDiagnosticKind::MetadataReadSkipped, 1),
                (DiskMapDiagnosticKind::Fallback, 1),
            ]
        );
    }

    #[test]
    fn disk_map_diagnostic_limit_zero_keeps_summary_only() {
        let mut diagnostics = DiskMapDiagnostics::new(0);
        diagnostics.push(DiskMapDiagnostic::new(
            DiskMapDiagnosticKind::MetadataReadSkipped,
            PathBuf::from("C:\\root\\child"),
            "child failed",
        ));
        let mut report = DiskMapReport::default();
        diagnostics.finish(&mut report);

        assert!(report.diagnostics.is_empty());
        assert_eq!(report.diagnostic_summary.total, 1);
        assert_eq!(report.diagnostic_summary.retained, 0);
        assert_eq!(report.diagnostic_summary.truncated, 1);
    }

    #[test]
    fn disk_map_semantic_caveats_are_reported_once_per_code() {
        let mut semantic_caveats = DiskMapSemanticCaveats::default();
        semantic_caveats.record(DiskMapMetadataSemantics {
            compressed: true,
            sparse: true,
            hardlink_count: Some(2),
            reparse_like: true,
            ..DiskMapMetadataSemantics::default()
        });

        let provenance =
            semantic_caveats.apply_to_root_provenance(EstimateProvenance::from_backend_confidence(
                ScanBackendKind::WindowsNative,
                ScanEstimateConfidence::Exact,
            ));
        let codes = provenance
            .estimate_caveats
            .iter()
            .map(|caveat| caveat.code.as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            codes,
            vec![
                "windows-native-compressed-file",
                "windows-native-sparse-file",
                "windows-native-hardlink-file",
                "windows-native-reparse-skipped",
            ]
        );

        let entry_provenance = estimate_provenance_with_entry_semantics(
            &EstimateProvenance::from_backend_confidence(
                ScanBackendKind::WindowsNative,
                ScanEstimateConfidence::Exact,
            ),
            DiskMapMetadataSemantics {
                compressed: true,
                sparse: true,
                hardlink_count: Some(3),
                reparse_like: false,
                ..DiskMapMetadataSemantics::default()
            },
            DiskMapEntryKind::File,
        );
        assert_eq!(entry_provenance.estimate_caveats.len(), 3);
    }
}

use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap};
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
#[cfg(windows)]
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

use crate::cleanup_advice::CleanupAdvice;
use crate::error::{RebeccaError, Result, ScanFailureKind};
pub use crate::inventory::{
    InventoryDiagnostic as DiskMapDiagnostic, InventoryDiagnosticKind as DiskMapDiagnosticKind,
    InventoryDiagnosticKindSummary as DiskMapDiagnosticKindSummary,
    InventoryDiagnosticSummary as DiskMapDiagnosticSummary, InventoryEntryKind as DiskMapEntryKind,
    InventoryGroup as DiskMapGroup, InventoryGroupKind as DiskMapGroupKind,
    InventoryMetrics as DiskMapMetrics, InventorySortField as DiskMapSortField,
};
use crate::plan::{EstimateProvenance, EstimateSource};
use crate::progress::{
    InspectProgressCounterKind, InspectProgressEvent, InspectProgressOptions,
    InspectProgressResult, InspectProgressRootStatus, PowerOfTwoProgressSampler,
};
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
    pub ntfs_mft_manifest_cache_root: Option<PathBuf>,
    pub progress_options: InspectProgressOptions,
    pub metadata_profile: DiskMapMetadataProfile,
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
            ntfs_mft_manifest_cache_root: None,
            progress_options: InspectProgressOptions::target(),
            metadata_profile: DiskMapMetadataProfile::default(),
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

    pub fn with_ntfs_mft_manifest_cache_root(mut self, cache_root: impl Into<PathBuf>) -> Self {
        self.ntfs_mft_manifest_cache_root = Some(cache_root.into());
        self
    }

    pub fn with_progress_options(mut self, progress_options: InspectProgressOptions) -> Self {
        self.progress_options = progress_options;
        self
    }

    pub fn with_metadata_profile(mut self, metadata_profile: DiskMapMetadataProfile) -> Self {
        self.metadata_profile = metadata_profile;
        self
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DiskMapMetadataProfile {
    LogicalOnly,
    Allocated,
    Unique,
    AgeAndGrouping,
    #[default]
    FullEvidence,
}

impl DiskMapMetadataProfile {
    pub const fn label(self) -> &'static str {
        match self {
            Self::LogicalOnly => "logical-only",
            Self::Allocated => "allocated",
            Self::Unique => "unique",
            Self::AgeAndGrouping => "age-and-grouping",
            Self::FullEvidence => "full-evidence",
        }
    }

    pub const fn collects_allocated_bytes(self) -> bool {
        !matches!(self, Self::LogicalOnly)
    }

    pub const fn collects_file_identity(self) -> bool {
        matches!(
            self,
            Self::Unique | Self::AgeAndGrouping | Self::FullEvidence
        )
    }

    pub const fn collects_modified_time(self) -> bool {
        matches!(self, Self::AgeAndGrouping | Self::FullEvidence)
    }

    pub const fn collects_semantic_caveats(self) -> bool {
        matches!(self, Self::FullEvidence)
    }

    pub const fn collects_walker_semantics(self) -> bool {
        self.collects_file_identity() || self.collects_semantic_caveats()
    }

    pub(crate) fn allocated_bytes(self, allocated_bytes: Option<u64>) -> Option<u64> {
        self.collects_allocated_bytes()
            .then_some(allocated_bytes)
            .flatten()
    }

    #[cfg(all(windows, feature = "ntfs"))]
    pub(crate) fn allocated_bytes_with(
        self,
        allocated_bytes: impl FnOnce() -> Option<u64>,
    ) -> Option<u64> {
        if self.collects_allocated_bytes() {
            allocated_bytes()
        } else {
            None
        }
    }

    pub(crate) fn modified_time(self, modified_time: Option<SystemTime>) -> Option<SystemTime> {
        self.collects_modified_time()
            .then_some(modified_time)
            .flatten()
    }

    #[cfg(all(windows, feature = "ntfs"))]
    pub(crate) fn modified_time_with(
        self,
        modified_time: impl FnOnce() -> Option<SystemTime>,
    ) -> Option<SystemTime> {
        if self.collects_modified_time() {
            modified_time()
        } else {
            None
        }
    }

    pub(crate) fn project_semantics(
        self,
        mut semantics: DiskMapMetadataSemantics,
    ) -> DiskMapMetadataSemantics {
        if !self.collects_file_identity() {
            semantics.file_identity = None;
            semantics.hardlink_count = None;
        }
        if !self.collects_semantic_caveats() {
            semantics.compressed = false;
            semantics.sparse = false;
            if !self.collects_file_identity() {
                semantics.hardlink_count = None;
            }
        }
        semantics
    }

    pub(crate) fn apply_to_metrics(self, metrics: &mut DiskMapMetrics) {
        if !self.collects_allocated_bytes() {
            metrics.allocated_bytes = None;
            metrics.unique_allocated_bytes = None;
        }
        if !self.collects_file_identity() {
            metrics.unique_logical_bytes = None;
            metrics.unique_allocated_bytes = None;
        }
    }

    pub(crate) fn apply_to_provenance(
        self,
        mut provenance: EstimateProvenance,
    ) -> EstimateProvenance {
        if self == Self::FullEvidence {
            return provenance;
        }

        provenance.estimate_caveats.push(disk_map_caveat(
            "metadata-profile",
            format!(
                "disk-map metadata profile '{}' skipped {}",
                self.label(),
                self.skipped_evidence_label()
            ),
        ));
        provenance
    }

    fn skipped_evidence_label(self) -> &'static str {
        match self {
            Self::LogicalOnly => {
                "allocated bytes, unique-file identity, modified-time, and semantic caveat evidence"
            }
            Self::Allocated => "unique-file identity, modified-time, and semantic caveat evidence",
            Self::Unique => "modified-time and semantic caveat evidence",
            Self::AgeAndGrouping => "semantic caveat evidence",
            Self::FullEvidence => "no metadata evidence",
        }
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

pub fn inspect_map(
    request: &DiskMapRequest,
    cancellation: &ScanCancellationToken,
) -> Result<DiskMapReport> {
    inspect_map_with_progress(request, cancellation, |_| Ok(()))
}

pub fn inspect_map_with_progress<F>(
    request: &DiskMapRequest,
    cancellation: &ScanCancellationToken,
    mut progress: F,
) -> Result<DiskMapReport>
where
    F: for<'event> FnMut(InspectProgressEvent<'event>) -> InspectProgressResult,
{
    let mut state = DiskMapInspectionState::new(request);
    let mut unique_files = DiskMapUniqueFiles::default();
    let scan_engine = scan_engine_for_disk_map(request);
    let root_count = request.roots.len();

    for (root_index, root) in request.roots.iter().enumerate() {
        check_cancelled(cancellation)?;
        progress(InspectProgressEvent::RootStarted {
            root_index,
            root_count,
            root,
            backend: request.scan_backend,
        })?;
        unique_files.merge(inspect_root(
            root,
            root_index,
            root_count,
            request,
            cancellation,
            &scan_engine,
            &mut state,
            &mut progress,
        )?);
    }

    let report = state.finish(unique_files);
    progress(InspectProgressEvent::Finalizing {
        roots: report.roots.len(),
        logical_bytes: report.totals.logical_bytes,
        files: report.totals.files,
        directories: report.totals.directories,
    })?;
    Ok(report)
}

fn scan_engine_for_disk_map(request: &DiskMapRequest) -> ScanEngine {
    #[cfg(all(windows, feature = "ntfs"))]
    {
        if let Some(cache_root) = &request.ntfs_mft_manifest_cache_root {
            return ScanEngine::with_ntfs_mft_manifest_cache_root(cache_root.clone());
        }
    }

    #[cfg(not(all(windows, feature = "ntfs")))]
    {
        let _ = &request.ntfs_mft_manifest_cache_root;
    }

    ScanEngine::new()
}

#[derive(Debug)]
struct DiskMapInspectionState {
    report: DiskMapReport,
    top_entries: DiskMapTopEntries,
    groups: DiskMapGroupCollector,
    diagnostics: DiskMapDiagnostics,
    metadata_profile: DiskMapMetadataProfile,
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
            metadata_profile: request.metadata_profile,
        }
    }

    fn finish(mut self, unique_files: DiskMapUniqueFiles) -> DiskMapReport {
        unique_files.apply_to_metrics(&mut self.report.totals);
        self.metadata_profile
            .apply_to_metrics(&mut self.report.totals);
        self.report.top_entries = self.top_entries.into_sorted_entries();
        self.report.groups = self.groups.finish();
        self.diagnostics.finish(&mut self.report);
        self.report
    }

    #[cfg(all(windows, feature = "ntfs"))]
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
            metadata_profile: request.metadata_profile,
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

    #[cfg(all(windows, feature = "ntfs"))]
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

struct DiskMapRootInspection<'a, W, F>
where
    W: DiskMapWalker,
    F: for<'event> FnMut(InspectProgressEvent<'event>) -> InspectProgressResult,
{
    request: &'a DiskMapRequest,
    cancellation: &'a ScanCancellationToken,
    state: &'a mut DiskMapInspectionState,
    walker: &'a W,
    root_index: usize,
    root_count: usize,
    progress: &'a mut F,
}

impl<'a, W, F> DiskMapRootInspection<'a, W, F>
where
    W: DiskMapWalker,
    F: for<'event> FnMut(InspectProgressEvent<'event>) -> InspectProgressResult,
{
    fn inspect(
        &mut self,
        root: &Path,
        provenance: EstimateProvenance,
        fallback_reason: Option<String>,
    ) -> Result<DiskMapUniqueFiles> {
        let provenance = self
            .request
            .metadata_profile
            .apply_to_provenance(provenance);
        if let Some(reason) = &fallback_reason {
            (self.progress)(InspectProgressEvent::BackendFallback {
                root,
                backend: self.request.scan_backend,
                reason,
            })?;
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
                self.emit_root_finished(
                    root,
                    InspectProgressRootStatus::Skipped,
                    DiskMapMetrics::default(),
                )?;
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
                self.emit_root_finished(
                    root,
                    InspectProgressRootStatus::Skipped,
                    DiskMapMetrics::default(),
                )?;
                return Ok(DiskMapUniqueFiles::default());
            }
        };

        let reparse_like = self.walker.is_reparse_like(root, &metadata);
        if reparse_like {
            push_root_skip(
                &mut self.state.report,
                root,
                DiskMapDiagnosticKind::ReparsePointSkipped,
                "disk map root is a symlink or reparse point",
                provenance,
                &mut self.state.diagnostics,
            );
            self.emit_root_finished(
                root,
                InspectProgressRootStatus::Skipped,
                DiskMapMetrics::default(),
            )?;
            return Ok(DiskMapUniqueFiles::default());
        }

        let mut semantic_caveats = DiskMapSemanticCaveats::default();
        let max_depth = self.request.max_depth.unwrap_or(usize::MAX);
        let root_result = match self.walker.metadata_kind(&metadata) {
            DiskMapMetadataKind::File => {
                let semantics = profiled_metadata_semantics(
                    self.walker,
                    &metadata,
                    self.request.metadata_profile,
                    reparse_like,
                );
                semantic_caveats.record(semantics);
                let result = DiskMapTraversalResult::file(
                    self.walker.metadata_len(&metadata),
                    profiled_allocated_len(self.walker, &metadata, self.request.metadata_profile),
                    semantics,
                    self.request.metadata_profile,
                );
                self.state.groups.record_file(
                    root,
                    0,
                    result.metrics.logical_bytes,
                    result.metrics.allocated_bytes,
                    profiled_modified_time(self.walker, &metadata, self.request.metadata_profile),
                    semantics,
                );
                if self.request.progress_options.includes_file_events() {
                    (self.progress)(InspectProgressEvent::FileMeasured {
                        root,
                        target_path: root,
                        path: root,
                        file_size: result.metrics.logical_bytes,
                        files_scanned: 1,
                        bytes_scanned: result.metrics.logical_bytes,
                    })?;
                }
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
                progress: self.progress,
                progress_options: self.request.progress_options,
                metadata_profile: self.request.metadata_profile,
                progress_totals: DiskMapTraversalProgress::default(),
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
        self.emit_root_finished(
            root,
            InspectProgressRootStatus::Scanned,
            root_result.metrics,
        )?;
        Ok(root_result.unique_files)
    }

    fn emit_root_finished(
        &mut self,
        root: &Path,
        status: InspectProgressRootStatus,
        metrics: DiskMapMetrics,
    ) -> Result<()> {
        (self.progress)(InspectProgressEvent::RootFinished {
            root_index: self.root_index,
            root_count: self.root_count,
            root,
            status,
            logical_bytes: metrics.logical_bytes,
            files: metrics.files,
            directories: metrics.directories,
        })
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

#[cfg(all(windows, feature = "ntfs"))]
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
    #[cfg(all(windows, feature = "ntfs"))]
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
    #[cfg(all(windows, feature = "ntfs"))]
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
        metadata_profile: DiskMapMetadataProfile,
    ) -> Self {
        let mut unique_files = DiskMapUniqueFiles::default();
        unique_files.record_file(semantics.file_identity, logical_bytes, allocated_bytes);
        let mut metrics = file_metrics(logical_bytes, allocated_bytes);
        unique_files.apply_to_metrics(&mut metrics);
        metadata_profile.apply_to_metrics(&mut metrics);
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

    #[cfg(all(windows, feature = "ntfs"))]
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

    #[cfg(all(windows, feature = "ntfs"))]
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
                DiskMapGroupKind::Type => disk_map_type_group(DiskMapEntryKind::File),
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

    pub(crate) fn record_directory(&mut self) {
        if self.kinds.is_empty() || self.limit == 0 || !self.kinds.contains(&DiskMapGroupKind::Type)
        {
            return;
        }

        let (key, label) = disk_map_type_group(DiskMapEntryKind::Directory);
        let map_key = DiskMapGroupMapKey {
            kind: DiskMapGroupKind::Type,
            key: key.clone(),
        };
        let accumulator = self
            .groups
            .entry(map_key)
            .or_insert_with(|| DiskMapGroupAccumulator::new(DiskMapGroupKind::Type, key, label));
        accumulator.metrics.directories = accumulator.metrics.directories.saturating_add(1);
    }

    #[cfg(all(windows, feature = "ntfs"))]
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
        let mut retained_by_kind = BTreeMap::<DiskMapGroupKind, usize>::new();
        groups.retain(|group| {
            let retained = retained_by_kind.entry(group.kind).or_default();
            if *retained >= self.limit {
                return false;
            }
            *retained += 1;
            true
        });
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

fn disk_map_type_group(kind: DiskMapEntryKind) -> (String, String) {
    let key = kind.label().to_string();
    let label = match kind {
        DiskMapEntryKind::File => "Files",
        DiskMapEntryKind::Directory => "Directories",
        DiskMapEntryKind::Other => "Other entries",
    };
    (key, label.to_string())
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
                "compressed-file",
                format!(
                    "{} compressed file(s) were seen; allocated_bytes may be lower than logical_bytes",
                    self.compressed_files
                ),
            ));
        }
        if self.sparse_files > 0 {
            provenance.estimate_caveats.push(disk_map_caveat(
                "sparse-file",
                format!(
                    "{} sparse file(s) were seen; allocated_bytes may be lower than logical_bytes",
                    self.sparse_files
                ),
            ));
        }
        if self.hardlinked_files > 0 {
            provenance.estimate_caveats.push(disk_map_caveat(
                "hardlink-file",
                format!(
                    "{} file path(s) reported multiple hard links; path-ranked bytes may overstate unique physical bytes when another link points to the same file",
                    self.hardlinked_files
                ),
            ));
        }
        if self.reparse_entries > 0 {
            provenance.estimate_caveats.push(disk_map_caveat(
                "reparse-skipped",
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
            "compressed-file",
            "file is compressed; allocated_bytes may be lower than logical_bytes",
        ));
    }
    if kind == DiskMapEntryKind::File && semantics.sparse {
        provenance.estimate_caveats.push(disk_map_caveat(
            "sparse-file",
            "file is sparse; allocated_bytes may be lower than logical_bytes",
        ));
    }
    if kind == DiskMapEntryKind::File && semantics.is_hardlinked() {
        let link_count = semantics.hardlink_count.unwrap_or(0);
        provenance.estimate_caveats.push(disk_map_caveat(
            "hardlink-file",
            format!(
                "file reports {link_count} hard links; path-ranked bytes may overstate unique physical bytes when another link points to the same file"
            ),
        ));
    }
    if semantics.reparse_like {
        provenance.estimate_caveats.push(disk_map_caveat(
            "reparse-skipped",
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

    #[cfg(windows)]
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

fn profiled_metadata_semantics<W>(
    walker: &W,
    metadata: &W::Metadata,
    metadata_profile: DiskMapMetadataProfile,
    reparse_like: bool,
) -> DiskMapMetadataSemantics
where
    W: DiskMapWalker,
{
    let semantics = if metadata_profile.collects_walker_semantics() {
        walker.metadata_semantics(metadata)
    } else {
        DiskMapMetadataSemantics::default()
    };
    let semantics = metadata_profile.project_semantics(semantics);
    if reparse_like {
        semantics.with_reparse_like()
    } else {
        semantics
    }
}

fn profiled_allocated_len<W>(
    walker: &W,
    metadata: &W::Metadata,
    metadata_profile: DiskMapMetadataProfile,
) -> Option<u64>
where
    W: DiskMapWalker,
{
    if metadata_profile.collects_allocated_bytes() {
        metadata_profile.allocated_bytes(walker.metadata_allocated_len(metadata))
    } else {
        None
    }
}

fn profiled_modified_time<W>(
    walker: &W,
    metadata: &W::Metadata,
    metadata_profile: DiskMapMetadataProfile,
) -> Option<SystemTime>
where
    W: DiskMapWalker,
{
    if metadata_profile.collects_modified_time() {
        metadata_profile.modified_time(walker.metadata_modified_time(metadata))
    } else {
        None
    }
}

#[derive(Debug, Default)]
struct FsPortableDiskMapWalker;

#[cfg(unix)]
fn portable_allocated_len(metadata: &std::fs::Metadata) -> Option<u64> {
    metadata.blocks().checked_mul(512)
}

#[cfg(not(unix))]
fn portable_allocated_len(_metadata: &std::fs::Metadata) -> Option<u64> {
    None
}

#[cfg(unix)]
fn portable_metadata_semantics(metadata: &std::fs::Metadata) -> DiskMapMetadataSemantics {
    if !metadata.is_file() {
        return DiskMapMetadataSemantics::default();
    }

    let allocated_len = portable_allocated_len(metadata);
    let mut semantics = DiskMapMetadataSemantics {
        file_identity: Some(DiskMapFileIdentity {
            volume_serial_number: metadata.dev(),
            file_index: metadata.ino(),
        }),
        ..DiskMapMetadataSemantics::default()
    };
    semantics.sparse = allocated_len.is_some_and(|allocated| allocated < metadata.len());
    semantics.hardlink_count = Some(metadata.nlink().min(u64::from(u32::MAX)) as u32);
    semantics
}

#[cfg(not(unix))]
fn portable_metadata_semantics(_metadata: &std::fs::Metadata) -> DiskMapMetadataSemantics {
    DiskMapMetadataSemantics::default()
}

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

    fn metadata_allocated_len(&self, metadata: &Self::Metadata) -> Option<u64> {
        portable_allocated_len(metadata)
    }

    fn metadata_modified_time(&self, metadata: &Self::Metadata) -> Option<SystemTime> {
        metadata.modified().ok()
    }

    fn metadata_semantics(&self, metadata: &Self::Metadata) -> DiskMapMetadataSemantics {
        portable_metadata_semantics(metadata)
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
#[derive(Debug)]
struct WindowsNativeDiskMapWalker {
    metadata_profile: DiskMapMetadataProfile,
}

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
    fn from_fs_metadata(
        path: &Path,
        metadata: &std::fs::Metadata,
        metadata_profile: DiskMapMetadataProfile,
    ) -> Self {
        let kind = if metadata.is_file() {
            DiskMapMetadataKind::File
        } else if metadata.is_dir() {
            DiskMapMetadataKind::Directory
        } else {
            DiskMapMetadataKind::Other
        };
        let reparse_like = is_reparse_like(metadata);
        let native_semantics = if metadata_profile.collects_walker_semantics() {
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
            )
        } else {
            Default::default()
        };
        Self {
            allocated_len: metadata_profile
                .collects_allocated_bytes()
                .then(|| match kind {
                    DiskMapMetadataKind::File => {
                        crate::scan::windows_native::file_allocated_size(path)
                    }
                    DiskMapMetadataKind::Directory | DiskMapMetadataKind::Other => None,
                })
                .flatten(),
            kind,
            len: metadata.len(),
            modified_time: metadata_profile
                .collects_modified_time()
                .then(|| metadata.modified().ok())
                .flatten(),
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
            .map(|metadata| {
                WindowsNativeDiskMapMetadata::from_fs_metadata(
                    path,
                    &metadata,
                    self.metadata_profile,
                )
            })
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
        crate::scan::windows_native::read_directory_entries(
            path,
            cancellation,
            crate::scan::windows_native::WindowsNativeDiskMapMetadataOptions::new(
                self.metadata_profile.collects_allocated_bytes(),
                self.metadata_profile.collects_modified_time(),
                self.metadata_profile.collects_walker_semantics(),
            ),
        )
        .map(Vec::into_iter)
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

struct DiskMapTraversal<'a, W, F>
where
    W: DiskMapWalker,
    F: for<'event> FnMut(InspectProgressEvent<'event>) -> InspectProgressResult,
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
    progress: &'a mut F,
    progress_options: InspectProgressOptions,
    metadata_profile: DiskMapMetadataProfile,
    progress_totals: DiskMapTraversalProgress,
}

impl<'a, W, F> DiskMapTraversal<'a, W, F>
where
    W: DiskMapWalker,
    F: for<'event> FnMut(InspectProgressEvent<'event>) -> InspectProgressResult,
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
        let reparse_like = self.walker.is_reparse_like(&path, &metadata);
        let semantics = profiled_metadata_semantics(
            self.walker,
            &metadata,
            self.metadata_profile,
            reparse_like,
        );
        if reparse_like {
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
                    profiled_allocated_len(self.walker, &metadata, self.metadata_profile),
                    semantics,
                    self.metadata_profile,
                );
                self.groups.record_file(
                    &path,
                    depth,
                    result.metrics.logical_bytes,
                    result.metrics.allocated_bytes,
                    profiled_modified_time(self.walker, &metadata, self.metadata_profile),
                    semantics,
                );
                self.record_file_progress(&path, result.metrics)?;
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
        self.groups.record_directory();
        self.record_directory_progress()?;
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

    fn record_file_progress(&mut self, path: &Path, metrics: DiskMapMetrics) -> Result<()> {
        self.progress_totals.metrics.files = self
            .progress_totals
            .metrics
            .files
            .saturating_add(metrics.files);
        self.progress_totals.metrics.logical_bytes = self
            .progress_totals
            .metrics
            .logical_bytes
            .saturating_add(metrics.logical_bytes);

        if self.progress_options.includes_file_events() {
            (self.progress)(InspectProgressEvent::FileMeasured {
                root: self.root,
                target_path: self.root,
                path,
                file_size: metrics.logical_bytes,
                files_scanned: self.progress_totals.metrics.files,
                bytes_scanned: self.progress_totals.metrics.logical_bytes,
            })?;
        }
        self.emit_sampled_counter(InspectProgressCounterKind::Files)?;
        self.emit_sampled_counter(InspectProgressCounterKind::Bytes)
    }

    fn record_directory_progress(&mut self) -> Result<()> {
        self.progress_totals.metrics.directories =
            self.progress_totals.metrics.directories.saturating_add(1);
        self.emit_sampled_counter(InspectProgressCounterKind::Directories)
    }

    fn emit_sampled_counter(&mut self, counter: InspectProgressCounterKind) -> Result<()> {
        let value = match counter {
            InspectProgressCounterKind::Files => self.progress_totals.metrics.files,
            InspectProgressCounterKind::Directories => self.progress_totals.metrics.directories,
            InspectProgressCounterKind::Bytes => self.progress_totals.metrics.logical_bytes,
            InspectProgressCounterKind::Records => 0,
        };
        if value == 0 || !self.progress_totals.sampler.should_emit(counter, value) {
            return Ok(());
        }

        (self.progress)(InspectProgressEvent::TraversalProgress {
            root: self.root,
            counter,
            value,
            logical_bytes: self.progress_totals.metrics.logical_bytes,
            files: self.progress_totals.metrics.files,
            directories: self.progress_totals.metrics.directories,
        })
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

#[derive(Debug, Clone, Default)]
struct DiskMapTraversalProgress {
    metrics: DiskMapMetrics,
    sampler: PowerOfTwoProgressSampler,
}

#[expect(
    clippy::too_many_arguments,
    reason = "backend dispatch carries root identity, shared inspection state, cancellation, and progress sink"
)]
fn inspect_root<F>(
    root: &Path,
    root_index: usize,
    root_count: usize,
    request: &DiskMapRequest,
    cancellation: &ScanCancellationToken,
    scan_engine: &ScanEngine,
    state: &mut DiskMapInspectionState,
    progress: &mut F,
) -> Result<DiskMapUniqueFiles>
where
    F: for<'event> FnMut(InspectProgressEvent<'event>) -> InspectProgressResult,
{
    #[cfg(not(all(windows, feature = "ntfs")))]
    let _ = scan_engine;

    let walker = FsPortableDiskMapWalker;
    if request.scan_backend == ScanBackendKind::WindowsNative {
        match inspect_windows_native_root(
            root,
            root_index,
            root_count,
            request,
            cancellation,
            state,
            progress,
        ) {
            Ok(unique_files) => return Ok(unique_files),
            Err(err) if disk_map_backend_error_can_fallback(&err) => {
                let fallback_reason =
                    format!("windows-native disk-map inventory was unavailable: {err}");
                return DiskMapRootInspection {
                    request,
                    cancellation,
                    state,
                    walker: &walker,
                    root_index,
                    root_count,
                    progress,
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
        #[cfg(all(windows, feature = "ntfs"))]
        {
            match scan_engine.inspect_windows_ntfs_mft_disk_map_with_progress(
                root,
                state.backend_options(request),
                cancellation,
                progress,
            ) {
                Ok(root_map) => {
                    let unique_files =
                        DiskMapUniqueFiles::unavailable_for_files(root_map.metrics.files);
                    progress(InspectProgressEvent::RootFinished {
                        root_index,
                        root_count,
                        root,
                        status: InspectProgressRootStatus::Scanned,
                        logical_bytes: root_map.metrics.logical_bytes,
                        files: root_map.metrics.files,
                        directories: root_map.metrics.directories,
                    })?;
                    push_backend_report(root, root_map, state);
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
                        root_index,
                        root_count,
                        progress,
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

        #[cfg(not(all(windows, feature = "ntfs")))]
        {
            let err = crate::scan::windows_ntfs_mft_unavailable_error("disk-map inventory");
            let fallback_reason =
                format!("windows-ntfs-mft-experimental disk-map inventory was unavailable: {err}");
            return DiskMapRootInspection {
                request,
                cancellation,
                state,
                walker: &walker,
                root_index,
                root_count,
                progress,
            }
            .inspect(
                root,
                portable_estimate_provenance(Some(fallback_reason.clone())),
                Some(fallback_reason),
            );
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
        root_index,
        root_count,
        progress,
    }
    .inspect(
        root,
        portable_estimate_provenance(fallback_reason.clone()),
        fallback_reason,
    )
}

#[cfg(windows)]
fn inspect_windows_native_root<F>(
    root: &Path,
    root_index: usize,
    root_count: usize,
    request: &DiskMapRequest,
    cancellation: &ScanCancellationToken,
    state: &mut DiskMapInspectionState,
    progress: &mut F,
) -> Result<DiskMapUniqueFiles>
where
    F: for<'event> FnMut(InspectProgressEvent<'event>) -> InspectProgressResult,
{
    if let Some(reason) = crate::scan::windows_native::unsupported_path_reason(root) {
        return Err(RebeccaError::PlatformUnavailable(reason));
    }

    let walker = WindowsNativeDiskMapWalker {
        metadata_profile: request.metadata_profile,
    };
    DiskMapRootInspection {
        request,
        cancellation,
        state,
        walker: &walker,
        root_index,
        root_count,
        progress,
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
fn inspect_windows_native_root<F>(
    _root: &Path,
    _root_index: usize,
    _root_count: usize,
    _request: &DiskMapRequest,
    _cancellation: &ScanCancellationToken,
    _state: &mut DiskMapInspectionState,
    _progress: &mut F,
) -> Result<DiskMapUniqueFiles>
where
    F: for<'event> FnMut(InspectProgressEvent<'event>) -> InspectProgressResult,
{
    Err(RebeccaError::PlatformUnavailable(format!(
        "{} disk-map inventory is only available on Windows",
        ScanBackendKind::WindowsNative.label()
    )))
}

#[cfg(all(windows, feature = "ntfs"))]
fn push_backend_report(
    root: &Path,
    report: DiskMapBackendReport,
    state: &mut DiskMapInspectionState,
) {
    state.report.totals.add(report.metrics);
    for entry in report.top_entries {
        state.top_entries.push(entry);
    }
    state.groups.merge(report.groups);
    state.diagnostics.extend(report.diagnostics);
    state.report.roots.push(DiskMapRoot {
        path: root.to_path_buf(),
        status: DiskMapRootStatus::Scanned,
        metrics: report.metrics,
        estimate_source: EstimateSource::FreshScan,
        estimate_provenance: report.estimate_provenance,
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
            sort_value: sort.value(
                entry.logical_bytes,
                entry.allocated_bytes,
                entry.unique_logical_bytes,
                entry.files,
            ),
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

#[cfg(all(windows, feature = "ntfs"))]
pub(crate) struct DiskMapBackendReport {
    pub(crate) metrics: DiskMapMetrics,
    pub(crate) top_entries: Vec<DiskMapEntry>,
    pub(crate) groups: DiskMapGroupCollector,
    pub(crate) diagnostics: Vec<DiskMapDiagnostic>,
    pub(crate) estimate_provenance: EstimateProvenance,
}

#[cfg(all(windows, feature = "ntfs"))]
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
    pub(crate) metadata_profile: DiskMapMetadataProfile,
}

#[cfg(all(windows, feature = "ntfs"))]
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
    let mut noop_progress = |_event: InspectProgressEvent<'_>| Ok(());
    inspect_map_with_walker_and_progress_for_test(request, cancellation, walker, &mut noop_progress)
}

#[cfg(test)]
fn inspect_map_with_walker_and_progress_for_test<W, F>(
    request: &DiskMapRequest,
    cancellation: &ScanCancellationToken,
    walker: &W,
    progress: &mut F,
) -> Result<DiskMapReport>
where
    W: DiskMapWalker,
    F: for<'event> FnMut(InspectProgressEvent<'event>) -> InspectProgressResult,
{
    let mut state = DiskMapInspectionState::new(request);
    let mut unique_files = DiskMapUniqueFiles::default();

    for (root_index, root) in request.roots.iter().enumerate() {
        check_cancelled(cancellation)?;
        progress(InspectProgressEvent::RootStarted {
            root_index,
            root_count: request.roots.len(),
            root,
            backend: request.scan_backend,
        })?;
        unique_files.merge(
            DiskMapRootInspection {
                request,
                cancellation,
                state: &mut state,
                walker,
                root_index,
                root_count: request.roots.len(),
                progress,
            }
            .inspect(root, portable_estimate_provenance(None), None)?,
        );
    }

    let report = state.finish(unique_files);
    progress(InspectProgressEvent::Finalizing {
        roots: report.roots.len(),
        logical_bytes: report.totals.logical_bytes,
        files: report.totals.files,
        directories: report.totals.directories,
    })?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, VecDeque};
    use std::io;

    use super::*;

    #[test]
    fn disk_map_request_can_carry_ntfs_mft_manifest_cache_root() {
        let cache_root = PathBuf::from(r"C:\cache\rebecca");
        let request = DiskMapRequest::new(vec![PathBuf::from(r"C:\target")])
            .with_ntfs_mft_manifest_cache_root(cache_root.clone());

        assert_eq!(request.ntfs_mft_manifest_cache_root, Some(cache_root));
    }

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
    fn disk_map_progress_sink_preserves_report() {
        let root = PathBuf::from("C:\\root");
        let file = root.join("readable.bin");
        let walker = FakeDiskMapWalker::default()
            .with_directory(&root, [FakeEntry::Path(file.clone())])
            .with_file(&file, 7);
        let request = DiskMapRequest::new(vec![root])
            .with_top_limit(10)
            .with_progress_options(InspectProgressOptions::file());
        let cancellation = ScanCancellationToken::new();
        let expected = inspect_map_with_walker_for_test(&request, &cancellation, &walker).unwrap();
        let mut event_kinds = Vec::new();
        let observed = {
            let mut progress = |event: InspectProgressEvent<'_>| -> InspectProgressResult {
                event_kinds.push(match event {
                    InspectProgressEvent::RootStarted { .. } => "root-started",
                    InspectProgressEvent::RootFinished { .. } => "root-finished",
                    InspectProgressEvent::FileMeasured { .. } => "file-measured",
                    InspectProgressEvent::TraversalProgress { .. } => "traversal-progress",
                    InspectProgressEvent::Finalizing { .. } => "finalizing",
                    _ => "other",
                });
                Ok(())
            };

            inspect_map_with_walker_and_progress_for_test(
                &request,
                &cancellation,
                &walker,
                &mut progress,
            )
            .unwrap()
        };

        assert_eq!(observed, expected);
        assert!(event_kinds.contains(&"root-started"));
        assert!(event_kinds.contains(&"file-measured"));
        assert!(event_kinds.contains(&"traversal-progress"));
        assert!(event_kinds.contains(&"root-finished"));
        assert!(event_kinds.contains(&"finalizing"));
    }

    #[test]
    fn disk_map_target_progress_omits_file_measured_events() {
        let root = PathBuf::from("C:\\root");
        let file = root.join("readable.bin");
        let walker = FakeDiskMapWalker::default()
            .with_directory(&root, [FakeEntry::Path(file.clone())])
            .with_file(&file, 7);
        let request = DiskMapRequest::new(vec![root]).with_top_limit(10);
        let cancellation = ScanCancellationToken::new();
        let mut event_kinds = Vec::new();

        let mut progress = |event: InspectProgressEvent<'_>| -> InspectProgressResult {
            event_kinds.push(match event {
                InspectProgressEvent::RootStarted { .. } => "root-started",
                InspectProgressEvent::RootFinished { .. } => "root-finished",
                InspectProgressEvent::FileMeasured { .. } => "file-measured",
                InspectProgressEvent::TraversalProgress { .. } => "traversal-progress",
                InspectProgressEvent::Finalizing { .. } => "finalizing",
                _ => "other",
            });
            Ok(())
        };

        inspect_map_with_walker_and_progress_for_test(
            &request,
            &cancellation,
            &walker,
            &mut progress,
        )
        .unwrap();

        assert!(!event_kinds.contains(&"file-measured"));
        assert!(event_kinds.contains(&"traversal-progress"));
        assert!(event_kinds.contains(&"root-finished"));
        assert!(event_kinds.contains(&"finalizing"));
    }

    #[test]
    fn disk_map_logical_only_profile_omits_allocated_and_unique_claims() {
        let root = PathBuf::from("C:\\root");
        let left = root.join("left.bin");
        let right = root.join("right.bin");
        let identity = DiskMapFileIdentity {
            volume_serial_number: 11,
            file_index: 22,
        };
        let walker = FakeDiskMapWalker::default()
            .with_directory(
                &root,
                [
                    FakeEntry::Path(left.clone()),
                    FakeEntry::Path(right.clone()),
                ],
            )
            .with_identified_file(&left, 4, Some(4096), identity)
            .with_identified_file(&right, 4, Some(4096), identity);
        let request = DiskMapRequest::new(vec![root])
            .with_top_limit(10)
            .with_metadata_profile(DiskMapMetadataProfile::LogicalOnly);

        let report =
            inspect_map_with_walker_for_test(&request, &ScanCancellationToken::new(), &walker)
                .unwrap();

        assert_eq!(report.totals.logical_bytes, 8);
        assert_eq!(report.totals.allocated_bytes, None);
        assert_eq!(report.totals.unique_logical_bytes, None);
        assert_eq!(report.totals.unique_allocated_bytes, None);
        assert!(report.top_entries.iter().all(|entry| {
            entry.allocated_bytes.is_none()
                && entry.unique_logical_bytes.is_none()
                && entry.unique_allocated_bytes.is_none()
        }));
        assert!(
            report.roots[0]
                .estimate_provenance
                .estimate_caveats
                .iter()
                .any(|caveat| caveat.code == "metadata-profile")
        );
    }

    #[test]
    fn disk_map_profiles_omit_unique_totals_for_empty_trees_without_identity_collection() {
        let root = PathBuf::from("C:\\root");
        let walker = FakeDiskMapWalker::default().with_directory(&root, []);

        for metadata_profile in [
            DiskMapMetadataProfile::LogicalOnly,
            DiskMapMetadataProfile::Allocated,
        ] {
            let request = DiskMapRequest::new(vec![root.clone()])
                .with_top_limit(10)
                .with_metadata_profile(metadata_profile);

            let report =
                inspect_map_with_walker_for_test(&request, &ScanCancellationToken::new(), &walker)
                    .unwrap();

            assert_eq!(report.totals.logical_bytes, 0);
            assert_eq!(report.totals.unique_logical_bytes, None);
            assert_eq!(report.totals.unique_allocated_bytes, None);
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
                "compressed-file",
                "sparse-file",
                "hardlink-file",
                "reparse-skipped",
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

use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{RebeccaError, Result};
use crate::inventory::INVENTORY_DIAGNOSTIC_REASON_SUMMARY_LIMIT;
use crate::plan::{EstimateProvenance, EstimateSource};
use crate::progress::{
    InspectProgressCacheEvent, InspectProgressEvent, InspectProgressOptions, InspectProgressResult,
    InspectProgressRootStatus,
};
use crate::safety::is_reparse_like;
use crate::scan::{
    ScanBackendKind, ScanCancellationToken, ScanEngine, ScanProgressEvent, ScanReport,
};
use crate::scan_cache::{ScanCacheCompatibility, ScanCacheLookup, ScanCachePolicy, ScanCacheStore};

pub const DEFAULT_SPACE_INSIGHT_TOP_LIMIT: usize = 10;
pub const DEFAULT_SPACE_INSIGHT_DIAGNOSTIC_LIMIT: usize = 100;

#[derive(Debug, Clone)]
pub struct SpaceInsightRequest {
    pub roots: Vec<PathBuf>,
    pub top_limit: usize,
    pub diagnostic_limit: usize,
    pub scan_backend: ScanBackendKind,
    pub scan_cache: Option<SpaceInsightScanCache>,
}

impl SpaceInsightRequest {
    pub fn new(roots: Vec<PathBuf>) -> Self {
        Self {
            roots,
            top_limit: DEFAULT_SPACE_INSIGHT_TOP_LIMIT,
            diagnostic_limit: DEFAULT_SPACE_INSIGHT_DIAGNOSTIC_LIMIT,
            scan_backend: ScanBackendKind::PortableRecursive,
            scan_cache: None,
        }
    }

    pub fn with_top_limit(mut self, top_limit: usize) -> Self {
        self.top_limit = top_limit;
        self
    }

    pub fn with_diagnostic_limit(mut self, diagnostic_limit: usize) -> Self {
        self.diagnostic_limit = diagnostic_limit;
        self
    }

    pub fn with_scan_backend(mut self, scan_backend: ScanBackendKind) -> Self {
        self.scan_backend = scan_backend;
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
    pub diagnostic_summary: SpaceInsightDiagnosticSummary,
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
    #[serde(default, flatten)]
    pub estimate_provenance: EstimateProvenance,
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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpaceInsightDiagnosticSummary {
    pub total: u64,
    pub retained: u64,
    pub truncated: u64,
    pub by_kind: Vec<SpaceInsightDiagnosticKindSummary>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub top_reasons: Vec<SpaceInsightDiagnosticReasonSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpaceInsightDiagnosticKindSummary {
    pub kind: SpaceInsightDiagnosticKind,
    pub count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpaceInsightDiagnosticReasonSummary {
    pub kind: SpaceInsightDiagnosticKind,
    pub count: u64,
    pub detail: String,
    pub sample_path: PathBuf,
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
    inspect_space_with_progress(
        request,
        cancellation,
        InspectProgressOptions::target(),
        |_| Ok(()),
    )
}

pub fn inspect_space_with_progress<F>(
    request: &SpaceInsightRequest,
    cancellation: &ScanCancellationToken,
    progress_options: InspectProgressOptions,
    mut progress: F,
) -> Result<SpaceInsightReport>
where
    F: for<'event> FnMut(InspectProgressEvent<'event>) -> InspectProgressResult,
{
    let mut report = SpaceInsightReport::default();
    let mut top_entries = SpaceInsightTopEntries::new(request.top_limit);
    let mut diagnostics = SpaceInsightDiagnostics::new(request.diagnostic_limit);
    let scan_engine = ScanEngine::new();
    let root_count = request.roots.len();

    for (root_index, root) in request.roots.iter().enumerate() {
        check_cancelled(cancellation)?;
        progress(InspectProgressEvent::RootStarted {
            root_index,
            root_count,
            root,
            backend: request.scan_backend,
        })?;
        inspect_root(
            root,
            root_index,
            root_count,
            request,
            cancellation,
            &scan_engine,
            &mut report,
            &mut top_entries,
            &mut diagnostics,
            progress_options,
            &mut progress,
        )?;
    }

    report.top_entries = top_entries.into_sorted_entries();
    diagnostics.finish(&mut report);
    progress(InspectProgressEvent::Finalizing {
        roots: report.roots.len(),
        logical_bytes: report.totals.estimated_bytes,
        files: report.totals.files,
        directories: report.totals.directories,
    })?;
    Ok(report)
}

#[derive(Debug)]
struct SpaceInsightDiagnostics {
    limit: usize,
    total: u64,
    counts_by_kind: BTreeMap<SpaceInsightDiagnosticKind, u64>,
    reasons: BTreeMap<SpaceInsightDiagnosticReasonKey, SpaceInsightDiagnosticReasonSummary>,
    samples: Vec<SpaceInsightDiagnosticSample>,
    sequence: u64,
}

impl SpaceInsightDiagnostics {
    fn new(limit: usize) -> Self {
        Self {
            limit,
            total: 0,
            counts_by_kind: BTreeMap::new(),
            reasons: BTreeMap::new(),
            samples: Vec::new(),
            sequence: 0,
        }
    }

    fn push(&mut self, diagnostic: SpaceInsightDiagnostic) {
        self.total = self.total.saturating_add(1);
        *self.counts_by_kind.entry(diagnostic.kind).or_default() += 1;
        let reason_key = SpaceInsightDiagnosticReasonKey::from_diagnostic(&diagnostic);
        self.reasons
            .entry(reason_key)
            .and_modify(|summary| summary.count = summary.count.saturating_add(1))
            .or_insert_with(|| SpaceInsightDiagnosticReasonSummary {
                kind: diagnostic.kind,
                count: 1,
                detail: diagnostic.detail.clone(),
                sample_path: diagnostic.path.clone(),
            });

        if self.limit == 0 {
            return;
        }

        let sample = SpaceInsightDiagnosticSample {
            sequence: self.sequence,
            diagnostic,
        };
        self.sequence = self.sequence.saturating_add(1);

        if self.samples.len() < self.limit {
            self.samples.push(sample);
        }
    }

    fn finish(self, report: &mut SpaceInsightReport) {
        let mut samples = self.samples;
        samples.sort_by(|left, right| {
            left.diagnostic
                .cmp(&right.diagnostic)
                .then_with(|| left.sequence.cmp(&right.sequence))
        });
        report.diagnostics = samples
            .into_iter()
            .map(|sample| sample.diagnostic)
            .collect();
        let retained = report.diagnostics.len() as u64;
        let mut top_reasons = self.reasons.into_values().collect::<Vec<_>>();
        top_reasons.sort_by(|left, right| {
            right
                .count
                .cmp(&left.count)
                .then_with(|| left.kind.cmp(&right.kind))
                .then_with(|| left.detail.cmp(&right.detail))
                .then_with(|| left.sample_path.cmp(&right.sample_path))
        });
        top_reasons.truncate(INVENTORY_DIAGNOSTIC_REASON_SUMMARY_LIMIT);
        report.diagnostic_summary = SpaceInsightDiagnosticSummary {
            total: self.total,
            retained,
            truncated: self.total.saturating_sub(retained),
            by_kind: self
                .counts_by_kind
                .into_iter()
                .map(|(kind, count)| SpaceInsightDiagnosticKindSummary { kind, count })
                .collect(),
            top_reasons,
        };
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct SpaceInsightDiagnosticReasonKey {
    kind: SpaceInsightDiagnosticKind,
    detail: String,
}

impl SpaceInsightDiagnosticReasonKey {
    fn from_diagnostic(diagnostic: &SpaceInsightDiagnostic) -> Self {
        Self {
            kind: diagnostic.kind,
            detail: diagnostic.detail.clone(),
        }
    }
}

#[derive(Debug)]
struct SpaceInsightDiagnosticSample {
    sequence: u64,
    diagnostic: SpaceInsightDiagnostic,
}

#[expect(
    clippy::too_many_arguments,
    reason = "space root inspection updates shared report, top-entry, diagnostic, and progress state"
)]
fn inspect_root<F>(
    root: &Path,
    root_index: usize,
    root_count: usize,
    request: &SpaceInsightRequest,
    cancellation: &ScanCancellationToken,
    scan_engine: &ScanEngine,
    report: &mut SpaceInsightReport,
    top_entries: &mut SpaceInsightTopEntries,
    diagnostics: &mut SpaceInsightDiagnostics,
    progress_options: InspectProgressOptions,
    progress: &mut F,
) -> Result<()>
where
    F: for<'event> FnMut(InspectProgressEvent<'event>) -> InspectProgressResult,
{
    let metadata = match std::fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            push_root_skip(
                report,
                root,
                SpaceInsightDiagnosticKind::RootMissing,
                "space inspection root does not exist",
                diagnostics,
            );
            progress(InspectProgressEvent::RootFinished {
                root_index,
                root_count,
                root,
                status: InspectProgressRootStatus::Skipped,
                logical_bytes: 0,
                files: 0,
                directories: 0,
            })?;
            return Ok(());
        }
        Err(err) => {
            push_root_skip(
                report,
                root,
                SpaceInsightDiagnosticKind::RootMetadataReadSkipped,
                format!("space inspection root metadata could not be read: {err}"),
                diagnostics,
            );
            progress(InspectProgressEvent::RootFinished {
                root_index,
                root_count,
                root,
                status: InspectProgressRootStatus::Skipped,
                logical_bytes: 0,
                files: 0,
                directories: 0,
            })?;
            return Ok(());
        }
    };

    if !metadata.is_dir() {
        push_root_skip(
            report,
            root,
            SpaceInsightDiagnosticKind::RootNotDirectory,
            "space inspection root is not a directory",
            diagnostics,
        );
        progress(InspectProgressEvent::RootFinished {
            root_index,
            root_count,
            root,
            status: InspectProgressRootStatus::Skipped,
            logical_bytes: 0,
            files: 0,
            directories: 0,
        })?;
        return Ok(());
    }

    if is_reparse_like(&metadata) {
        push_root_skip(
            report,
            root,
            SpaceInsightDiagnosticKind::ReparsePointSkipped,
            "space inspection root is a symlink or reparse point",
            diagnostics,
        );
        progress(InspectProgressEvent::RootFinished {
            root_index,
            root_count,
            root,
            status: InspectProgressRootStatus::Skipped,
            logical_bytes: 0,
            files: 0,
            directories: 0,
        })?;
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
                diagnostics,
            );
            progress(InspectProgressEvent::RootFinished {
                root_index,
                root_count,
                root,
                status: InspectProgressRootStatus::Skipped,
                logical_bytes: 0,
                files: 0,
                directories: 0,
            })?;
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
                diagnostics.push(SpaceInsightDiagnostic::new(
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

    for (entry_index, path) in entry_paths.into_iter().enumerate() {
        check_cancelled(cancellation)?;
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(err) => {
                diagnostics.push(SpaceInsightDiagnostic::new(
                    SpaceInsightDiagnosticKind::MetadataReadSkipped,
                    path,
                    format!("space inspection entry metadata could not be read: {err}"),
                ));
                continue;
            }
        };
        if is_reparse_like(&metadata) {
            diagnostics.push(SpaceInsightDiagnostic::new(
                SpaceInsightDiagnosticKind::ReparsePointSkipped,
                path,
                "space inspection entry is a symlink or reparse point",
            ));
            continue;
        }

        let entry_index = entry_index as u64;
        progress(InspectProgressEvent::EntryStarted {
            root,
            path: &path,
            entry_index,
            backend: request.scan_backend,
        })?;

        match inspect_entry(
            root,
            &path,
            metadata,
            request,
            cancellation,
            scan_engine,
            progress_options,
            progress,
        ) {
            Ok(entry) => {
                root_metrics.add_report(ScanReport {
                    bytes_scanned: entry.estimated_bytes,
                    files_scanned: entry.files,
                    directories_scanned: entry.directories,
                });
                progress(InspectProgressEvent::EntryMeasured {
                    root,
                    path: &entry.path,
                    entry_index,
                    logical_bytes: entry.estimated_bytes,
                    files: entry.files,
                    directories: entry.directories,
                })?;
                top_entries.push(entry);
            }
            Err(err) if space_entry_error_should_abort(&err) => return Err(err),
            Err(err) => diagnostics.push(SpaceInsightDiagnostic::new(
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
    progress(InspectProgressEvent::RootFinished {
        root_index,
        root_count,
        root,
        status: InspectProgressRootStatus::Scanned,
        logical_bytes: root_metrics.estimated_bytes,
        files: root_metrics.files,
        directories: root_metrics.directories,
    })?;
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

#[expect(
    clippy::too_many_arguments,
    reason = "space entry measurement carries root context, backend request, cancellation, and progress sink"
)]
fn inspect_entry<F>(
    root: &Path,
    path: &Path,
    metadata: std::fs::Metadata,
    request: &SpaceInsightRequest,
    cancellation: &ScanCancellationToken,
    scan_engine: &ScanEngine,
    progress_options: InspectProgressOptions,
    progress: &mut F,
) -> Result<SpaceInsightEntry>
where
    F: for<'event> FnMut(InspectProgressEvent<'event>) -> InspectProgressResult,
{
    let kind = if metadata.is_file() {
        SpaceInsightEntryKind::File
    } else if metadata.is_dir() {
        SpaceInsightEntryKind::Directory
    } else {
        SpaceInsightEntryKind::Other
    };

    let measurement = measure_entry(
        root,
        path,
        request,
        cancellation,
        scan_engine,
        progress_options,
        progress,
    )?;
    Ok(SpaceInsightEntry {
        path: path.to_path_buf(),
        root: root.to_path_buf(),
        kind,
        estimated_bytes: measurement.report.bytes_scanned,
        files: measurement.report.files_scanned,
        directories: measurement.report.directories_scanned,
        estimate_source: measurement.estimate_source,
        estimate_provenance: measurement.estimate_provenance,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SpaceInsightMeasurement {
    report: ScanReport,
    estimate_source: EstimateSource,
    estimate_provenance: EstimateProvenance,
}

fn measure_entry<F>(
    root: &Path,
    path: &Path,
    request: &SpaceInsightRequest,
    cancellation: &ScanCancellationToken,
    scan_engine: &ScanEngine,
    progress_options: InspectProgressOptions,
    progress: &mut F,
) -> Result<SpaceInsightMeasurement>
where
    F: for<'event> FnMut(InspectProgressEvent<'event>) -> InspectProgressResult,
{
    let mut cache_miss_reason = None;
    if let Some(scan_cache) = &request.scan_cache {
        let compatibility = ScanCacheCompatibility::logical_bytes(request.scan_backend);
        match scan_cache.store.load_with_policy_and_compatibility(
            path,
            scan_cache.policy,
            compatibility,
        ) {
            ScanCacheLookup::Hit(hit) => {
                progress(InspectProgressEvent::CacheEvent {
                    path,
                    event: InspectProgressCacheEvent::Hit,
                    reason: None,
                    estimated_bytes: Some(hit.report.bytes_scanned),
                })?;
                let mut evidence = hit.backend_evidence;
                evidence.record_cache_event("scan-cache", "hit", None);
                return Ok(SpaceInsightMeasurement {
                    report: hit.report,
                    estimate_source: EstimateSource::ScanCache,
                    estimate_provenance: EstimateProvenance::from_backend_confidence_and_source(
                        hit.backend,
                        hit.confidence,
                        hit.backend_source,
                    )
                    .with_backend_evidence(evidence),
                });
            }
            ScanCacheLookup::Miss(outcome) => {
                progress(InspectProgressEvent::CacheEvent {
                    path,
                    event: InspectProgressCacheEvent::Miss,
                    reason: Some(outcome.reason.label()),
                    estimated_bytes: None,
                })?;
                cache_miss_reason = Some(outcome.reason);
            }
        }
    }

    let emit_file_events = progress_options.includes_file_events();
    let mut progress_error = None;
    let measured_result =
        scan_engine.measure_scan_with_backend(path, cancellation, request.scan_backend, |event| {
            if !emit_file_events || progress_error.is_some() {
                return;
            }
            let ScanProgressEvent::FileMeasured {
                path: file_path,
                file_size,
                files_scanned,
                bytes_scanned,
            } = event;
            if let Err(err) = progress(InspectProgressEvent::FileMeasured {
                root,
                target_path: path,
                path: file_path,
                file_size,
                files_scanned,
                bytes_scanned,
            }) {
                progress_error = Some(err);
                cancellation.cancel();
            }
        });
    if let Some(err) = progress_error {
        return Err(err);
    }
    let measured_scan = measured_result?;
    let report = measured_scan.report;
    let mut estimate_backend_evidence = measured_scan.backend_evidence.clone();
    if let Some(reason) = cache_miss_reason {
        estimate_backend_evidence.record_cache_event(
            "scan-cache",
            "miss",
            Some(reason.label().to_string()),
        );
    }
    if let Some(scan_cache) = &request.scan_cache
        && let Err(err) = scan_cache.store.store_measured_scan_with_policy(
            path,
            measured_scan.clone(),
            scan_cache.policy,
        )
    {
        tracing::debug!(
            path = %path.display(),
            error = %err,
            "inspect scan cache write skipped"
        );
        estimate_backend_evidence.record_cache_event(
            "scan-cache",
            "write-skipped",
            Some("write-failed".to_string()),
        );
        progress(InspectProgressEvent::CacheEvent {
            path,
            event: InspectProgressCacheEvent::WriteSkipped,
            reason: Some("write-failed"),
            estimated_bytes: None,
        })?;
    }
    let mut estimate_provenance = EstimateProvenance::from_measured_scan(&measured_scan);
    estimate_provenance.estimate_backend_evidence = estimate_backend_evidence;

    Ok(SpaceInsightMeasurement {
        report,
        estimate_source: EstimateSource::FreshScan,
        estimate_provenance,
    })
}

fn space_entry_error_should_abort(err: &RebeccaError) -> bool {
    matches!(
        err,
        RebeccaError::OperationCancelled(_) | RebeccaError::Io(_) | RebeccaError::Json(_)
    )
}

fn push_root_skip(
    report: &mut SpaceInsightReport,
    root: &Path,
    kind: SpaceInsightDiagnosticKind,
    detail: impl Into<String>,
    diagnostics: &mut SpaceInsightDiagnostics,
) {
    let detail = detail.into();
    report.roots.push(SpaceInsightRoot {
        path: root.to_path_buf(),
        status: SpaceInsightRootStatus::Skipped,
        metrics: SpaceInsightMetrics::default(),
        reason: Some(detail.clone()),
    });
    diagnostics.push(SpaceInsightDiagnostic::new(
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

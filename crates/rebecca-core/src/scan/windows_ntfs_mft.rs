use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::mem::{offset_of, size_of};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::path::{Component, Path, PathBuf, Prefix};
use std::ptr;
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rayon::{ThreadPool, prelude::*};
use rebecca_ntfs::{
    MftIndex, MftIndexEntry, MftRecordBatch, MftRecordReader, NtfsDirectoryEntry,
    NtfsFileReference, NtfsParsedRecord, NtfsRecordSet, NtfsStreamGeometry, NtfsStreamSource,
    ParseCaveat, PhysicalMetrics, PhysicalMetricsAccumulator, SubtreeSummary,
    resolve_record_with_stream_source,
};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_ACCESS_DENIED, ERROR_HANDLE_EOF, ERROR_INVALID_PARAMETER, ERROR_MORE_DATA,
    HANDLE, WIN32_ERROR,
};
use windows::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, CreateFileW, FILE_BEGIN, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_FLAG_OPEN_REPARSE_POINT, FILE_FLAG_SEQUENTIAL_SCAN, FILE_FLAGS_AND_ATTRIBUTES,
    FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_MODE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    GetDriveTypeW, GetFileInformationByHandle, GetVolumeInformationW, OPEN_EXISTING, ReadFile,
    SYNCHRONIZE, SetFilePointerEx,
};
use windows::Win32::System::IO::DeviceIoControl;
use windows::Win32::System::Ioctl::{
    FSCTL_GET_NTFS_FILE_RECORD, FSCTL_GET_NTFS_VOLUME_DATA, FSCTL_GET_RETRIEVAL_POINTERS,
    NTFS_FILE_RECORD_INPUT_BUFFER, NTFS_FILE_RECORD_OUTPUT_BUFFER, NTFS_VOLUME_DATA_BUFFER,
    RETRIEVAL_POINTERS_BUFFER, RETRIEVAL_POINTERS_BUFFER_0, STARTING_VCN_INPUT_BUFFER,
};
use windows::core::{Error as WindowsError, HRESULT, PCWSTR};

use crate::disk_map::{
    DiskMapBackendOptions, DiskMapBackendRoot, DiskMapEntry, DiskMapEntryKind, DiskMapFileIdentity,
    DiskMapGroupCollector, DiskMapMetadataSemantics, DiskMapMetrics, DiskMapTopEntries,
};
use crate::error::{RebeccaError, Result, ScanFailure, ScanFailurePhase};
use crate::parallelism::{bounded_parallelism_budget, run_scoped_parallel_work};
use crate::plan::{EstimateProvenance, EstimateSource};
use crate::safety::is_reparse_like;

use super::backend::{
    MeasuredScan, ScanBackend, ScanBackendKind, ScanEstimateConfidence, ScanRequest,
};
use super::progress::{ScanProgressEvent, check_not_cancelled};
use super::{ScanCancellationToken, ScanReport};

const EXPERIMENTAL_NTFS_MFT_BACKEND_LABEL: &str = "windows-ntfs-mft-experimental";
const NTFS_FILE_SYSTEM_NAME: &str = "NTFS";
const DRIVE_FIXED: u32 = 3;
const FILE_REFERENCE_LOW_MASK: u64 = 0x0000_FFFF_FFFF_FFFF;
const SEQUENTIAL_MFT_SOURCE_LABEL: &str = "sequential";
const FSCTL_RECORD_SOURCE_LABEL: &str = "fsctl-record";
const TARGETED_MFT_SOURCE_LABEL: &str = "targeted-fsctl";
const SEQUENTIAL_MFT_CHUNK_BYTES: usize = 8 * 1024 * 1024;
const SEQUENTIAL_MFT_PARSE_WINDOW_CHUNKS: usize = 8;
const TARGETED_MFT_MAX_RECORDS: usize = 1_000_000;
const TARGETED_MFT_MAX_DEPTH: usize = 512;
const NTFS_FILE_RECORD_OUTPUT_HEADER_BYTES: usize = size_of::<i64>() + size_of::<u32>();
const MAX_RETRIEVAL_POINTER_BUFFER_BYTES: usize = 16 * 1024 * 1024;
const MAX_MFT_PARSE_ERROR_CAVEAT_SAMPLES: usize = 8;
const MAX_MFT_ESTIMATE_CAVEAT_SAMPLES_PER_CODE: usize = 8;
const MFT_CAVEAT_SUMMARY_CODE: &str = "mft-caveat-summary";
const LIVE_NTFS_MFT_INDEX_TIMEOUT_ENV: &str = "REBECCA_NTFS_MFT_INDEX_TIMEOUT_SECONDS";
const LIVE_NTFS_MFT_INDEX_TIMINGS_ENV: &str = "REBECCA_NTFS_MFT_INDEX_TIMINGS";
const LIVE_NTFS_MFT_FULL_INDEX_FALLBACK_ENV: &str = "REBECCA_NTFS_MFT_FULL_INDEX_FALLBACK";
const DEFAULT_LIVE_NTFS_MFT_INDEX_TIMEOUT: Duration = Duration::from_secs(20);
const MFT_BUILD_TIMING_CAVEAT_CODE: &str = "mft-index-build-timing";

static MFT_PARSE_THREAD_POOL: OnceLock<ThreadPool> = OnceLock::new();

#[derive(Debug, Clone, Copy)]
struct NtfsMftBuildBudget {
    started_at: Instant,
    timeout: Option<Duration>,
}

impl NtfsMftBuildBudget {
    fn new(timeout: Option<Duration>) -> Self {
        Self {
            started_at: Instant::now(),
            timeout,
        }
    }
}

#[derive(Debug, Default)]
struct NtfsMftBuildMonitorState {
    active_stage: Option<(NtfsMftBuildStage, Instant)>,
    timings: BTreeMap<NtfsMftBuildStage, Duration>,
}

#[derive(Debug)]
struct NtfsMftBuildMonitor {
    budget: NtfsMftBuildBudget,
    state: RefCell<NtfsMftBuildMonitorState>,
    emit_timing_caveat: bool,
}

impl NtfsMftBuildMonitor {
    fn from_environment() -> Self {
        Self::new(
            live_ntfs_mft_index_timeout(),
            live_ntfs_mft_index_timings_enabled(),
        )
    }

    fn new(timeout: Option<Duration>, emit_timing_caveat: bool) -> Self {
        Self {
            budget: NtfsMftBuildBudget::new(timeout),
            state: RefCell::new(NtfsMftBuildMonitorState::default()),
            emit_timing_caveat,
        }
    }

    #[cfg(test)]
    fn measure<R>(
        &self,
        stage: NtfsMftBuildStage,
        operation: impl FnOnce() -> Result<R>,
    ) -> Result<R> {
        self.measure_inner(stage, operation, || Ok(()))
    }

    fn measure_checked<R>(
        &self,
        stage: NtfsMftBuildStage,
        cancellation: &ScanCancellationToken,
        operation: impl FnOnce() -> Result<R>,
    ) -> Result<R> {
        self.check(cancellation)?;
        self.measure_inner(stage, operation, || self.check(cancellation))
    }

    fn measure_inner<R>(
        &self,
        stage: NtfsMftBuildStage,
        operation: impl FnOnce() -> Result<R>,
        after_success: impl FnOnce() -> Result<()>,
    ) -> Result<R> {
        let started_at = Instant::now();
        let previous_stage = {
            let mut state = self.state.borrow_mut();
            state.active_stage.replace((stage, started_at))
        };
        let result = operation();
        let elapsed = started_at.elapsed();
        let after_success = if result.is_ok() {
            after_success()
        } else {
            Ok(())
        };
        {
            let mut state = self.state.borrow_mut();
            let total = state.timings.entry(stage).or_default();
            *total = total.saturating_add(elapsed);
            state.active_stage = previous_stage;
        }
        match result {
            Ok(value) => {
                after_success?;
                Ok(value)
            }
            Err(err) => Err(err),
        }
    }

    fn check(&self, cancellation: &ScanCancellationToken) -> Result<()> {
        check_not_cancelled(cancellation)?;
        if !self.is_timed_out() {
            return Ok(());
        }
        let Some(timeout) = self.budget.timeout else {
            return Ok(());
        };

        let state = self.state.borrow();
        let stage = state
            .active_stage
            .map(|(stage, started_at)| {
                format!(
                    " while {}; stage_elapsed={}ms",
                    stage.label(),
                    started_at.elapsed().as_millis()
                )
            })
            .unwrap_or_default();
        let timings = format_timing_summary(&state.timings)
            .map(|summary| format!("; completed_timings={summary}"))
            .unwrap_or_default();

        Err(RebeccaError::PlatformUnavailable(format!(
            "{EXPERIMENTAL_NTFS_MFT_BACKEND_LABEL} live volume index build timed out after {}s{stage}{timings}; tune {LIVE_NTFS_MFT_INDEX_TIMEOUT_ENV} to increase the budget or set it to 0 to disable this guard",
            timeout.as_secs()
        )))
    }

    fn is_timed_out(&self) -> bool {
        self.budget
            .timeout
            .is_some_and(|timeout| self.budget.started_at.elapsed() >= timeout)
    }

    fn timing_caveat(&self) -> Option<ParseCaveat> {
        if !self.emit_timing_caveat {
            return None;
        }
        self.timing_summary().map(|summary| {
            ParseCaveat::new(
                MFT_BUILD_TIMING_CAVEAT_CODE,
                format!("live NTFS/MFT index build timings: {summary}"),
            )
        })
    }

    fn timing_summary(&self) -> Option<String> {
        let state = self.state.borrow();
        format_timing_summary(&state.timings)
    }

    #[cfg(test)]
    fn expired_for_test(timeout: Duration) -> Self {
        let mut monitor = Self::new(Some(timeout), false);
        let now = Instant::now();
        let elapsed = timeout + Duration::from_secs(1);
        monitor.budget.started_at = now.checked_sub(elapsed).unwrap_or(now);
        monitor
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum NtfsMftBuildStage {
    OpenVolume,
    ReadVolumeData,
    SequentialOpenMftData,
    SequentialReadRetrievalPointers,
    SequentialReadMftBytes,
    SequentialParseRecords,
    FsctlReadParseRecords,
    TargetedReadRecord,
    TargetedResolveRecord,
    TargetedTraverseSubtree,
    ResolveIndexAllocations,
    BuildMftIndex,
}

impl NtfsMftBuildStage {
    fn label(self) -> &'static str {
        match self {
            Self::OpenVolume => "open-volume",
            Self::ReadVolumeData => "read-volume-data",
            Self::SequentialOpenMftData => "sequential-open-mft-data",
            Self::SequentialReadRetrievalPointers => "sequential-read-retrieval-pointers",
            Self::SequentialReadMftBytes => "sequential-read-mft-bytes",
            Self::SequentialParseRecords => "sequential-parse-records",
            Self::FsctlReadParseRecords => "fsctl-read-parse-records",
            Self::TargetedReadRecord => "targeted-read-record",
            Self::TargetedResolveRecord => "targeted-resolve-record",
            Self::TargetedTraverseSubtree => "targeted-traverse-subtree",
            Self::ResolveIndexAllocations => "resolve-index-allocations",
            Self::BuildMftIndex => "build-mft-index",
        }
    }
}

fn format_timing_summary(timings: &BTreeMap<NtfsMftBuildStage, Duration>) -> Option<String> {
    if timings.is_empty() {
        return None;
    }
    Some(
        timings
            .iter()
            .map(|(stage, duration)| format!("{}={}ms", stage.label(), duration.as_millis()))
            .collect::<Vec<_>>()
            .join(", "),
    )
}

fn live_ntfs_mft_index_timeout() -> Option<Duration> {
    let Some(raw) = std::env::var_os(LIVE_NTFS_MFT_INDEX_TIMEOUT_ENV) else {
        return Some(DEFAULT_LIVE_NTFS_MFT_INDEX_TIMEOUT);
    };

    let raw = raw.to_string_lossy();
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Some(DEFAULT_LIVE_NTFS_MFT_INDEX_TIMEOUT);
    }

    match trimmed.parse::<u64>() {
        Ok(0) => None,
        Ok(seconds) => Some(Duration::from_secs(seconds)),
        Err(_) => Some(DEFAULT_LIVE_NTFS_MFT_INDEX_TIMEOUT),
    }
}

fn live_ntfs_mft_index_timings_enabled() -> bool {
    std::env::var_os(LIVE_NTFS_MFT_INDEX_TIMINGS_ENV).is_some_and(|raw| {
        let raw = raw.to_string_lossy();
        let trimmed = raw.trim();
        !trimmed.is_empty() && trimmed != "0"
    })
}

fn live_ntfs_mft_full_index_fallback_enabled() -> bool {
    std::env::var_os(LIVE_NTFS_MFT_FULL_INDEX_FALLBACK_ENV).is_some_and(|raw| {
        let raw = raw.to_string_lossy();
        let trimmed = raw.trim();
        !trimmed.is_empty() && trimmed != "0"
    })
}

fn check_mft_build_progress(
    cancellation: &ScanCancellationToken,
    monitor: &NtfsMftBuildMonitor,
) -> Result<()> {
    monitor.check(cancellation)
}

#[derive(Debug, Default)]
pub(super) struct WindowsNtfsMftIndexCache {
    volumes: Mutex<BTreeMap<String, CachedNtfsVolumeIndexSlot>>,
    volume_changed: Condvar,
}

impl WindowsNtfsMftIndexCache {
    fn load_or_build(
        &self,
        capabilities: &NtfsVolumeCapabilities,
        cancellation: &ScanCancellationToken,
    ) -> Result<Arc<CachedNtfsVolumeIndex>> {
        let cache_key = capabilities.cache_key();

        let mut volumes = self.lock_volumes()?;
        loop {
            check_not_cancelled(cancellation)?;
            match volumes.get(&cache_key) {
                Some(CachedNtfsVolumeIndexSlot::Ready(index)) => return Ok(Arc::clone(index)),
                Some(CachedNtfsVolumeIndexSlot::Unavailable(reason)) => {
                    return Err(RebeccaError::PlatformUnavailable(reason.clone()));
                }
                Some(CachedNtfsVolumeIndexSlot::Building) => {
                    volumes = self.wait_for_volume_update(volumes)?;
                }
                None => {
                    volumes.insert(cache_key.clone(), CachedNtfsVolumeIndexSlot::Building);
                    break;
                }
            }
        }
        drop(volumes);

        let build_result = CachedNtfsVolumeIndex::build(capabilities, cancellation);
        let mut volumes = self.lock_volumes()?;
        let result = match build_result {
            Ok(index) => {
                let index = Arc::new(index);
                volumes.insert(
                    cache_key,
                    CachedNtfsVolumeIndexSlot::Ready(Arc::clone(&index)),
                );
                Ok(index)
            }
            Err(err) => {
                if let Some(reason) = cacheable_index_failure(&err) {
                    volumes.insert(cache_key, CachedNtfsVolumeIndexSlot::Unavailable(reason));
                } else {
                    volumes.remove(&cache_key);
                }
                Err(err)
            }
        };
        self.volume_changed.notify_all();
        result
    }

    fn lock_volumes(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, BTreeMap<String, CachedNtfsVolumeIndexSlot>>> {
        self.volumes.lock().map_err(|_| {
            RebeccaError::PlatformUnavailable(format!(
                "{EXPERIMENTAL_NTFS_MFT_BACKEND_LABEL} volume index cache is unavailable"
            ))
        })
    }

    fn wait_for_volume_update<'a>(
        &self,
        volumes: std::sync::MutexGuard<'a, BTreeMap<String, CachedNtfsVolumeIndexSlot>>,
    ) -> Result<std::sync::MutexGuard<'a, BTreeMap<String, CachedNtfsVolumeIndexSlot>>> {
        let (volumes, _) = self
            .volume_changed
            .wait_timeout(volumes, Duration::from_millis(250))
            .map_err(|_| {
                RebeccaError::PlatformUnavailable(format!(
                    "{EXPERIMENTAL_NTFS_MFT_BACKEND_LABEL} volume index cache is unavailable"
                ))
            })?;
        Ok(volumes)
    }
}

#[derive(Debug)]
enum CachedNtfsVolumeIndexSlot {
    Ready(Arc<CachedNtfsVolumeIndex>),
    Unavailable(String),
    Building,
}

fn cacheable_index_failure(err: &RebeccaError) -> Option<String> {
    match err {
        RebeccaError::PlatformUnavailable(reason) => Some(reason.clone()),
        _ => None,
    }
}

#[derive(Debug)]
struct CachedNtfsVolumeIndex {
    mft_index: MftIndex,
    source_label: &'static str,
    caveats: Vec<ParseCaveat>,
}

impl CachedNtfsVolumeIndex {
    fn build(
        capabilities: &NtfsVolumeCapabilities,
        cancellation: &ScanCancellationToken,
    ) -> Result<Self> {
        let monitor = NtfsMftBuildMonitor::from_environment();
        check_mft_build_progress(cancellation, &monitor)?;
        let volume =
            monitor.measure_checked(NtfsMftBuildStage::OpenVolume, cancellation, || {
                LiveNtfsVolume::open(capabilities)
            })?;
        let volume_data =
            monitor.measure_checked(NtfsMftBuildStage::ReadVolumeData, cancellation, || {
                volume.ntfs_volume_data()
            })?;
        let geometry = NtfsRecordGeometry::from_volume_data(&volume.device_path, &volume_data)?;
        let records = volume.read_mft_records(&volume_data, cancellation, &monitor)?;
        let source_label = records.source_label;
        let mut stream_source = LiveNtfsIndexStreamSource {
            volume: &volume,
            cancellation,
            monitor: &monitor,
        };
        let (mft_index, mut caveats) = build_mft_index_from_records(
            records,
            geometry,
            &mut stream_source,
            cancellation,
            &monitor,
        )?;
        if let Some(caveat) = monitor.timing_caveat() {
            caveats.push(caveat);
        }
        Ok(Self {
            mft_index,
            source_label,
            caveats,
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct WindowsNtfsMftScanBackend<'a> {
    cache: &'a WindowsNtfsMftIndexCache,
}

impl<'a> WindowsNtfsMftScanBackend<'a> {
    pub(super) const fn new(cache: &'a WindowsNtfsMftIndexCache) -> Self {
        Self { cache }
    }
}

impl ScanBackend for WindowsNtfsMftScanBackend<'_> {
    fn kind(&self) -> ScanBackendKind {
        ScanBackendKind::WindowsNtfsMftExperimental
    }

    fn measure_path_with_progress<F>(
        &self,
        request: ScanRequest<'_>,
        _progress: F,
    ) -> Result<MeasuredScan>
    where
        F: for<'a> FnMut(ScanProgressEvent<'a>),
    {
        check_not_cancelled(request.cancellation)?;
        let metadata = root_metadata(request.path)?;
        if is_reparse_like(&metadata) {
            return Err(RebeccaError::SafetyBlocked(
                "symlink or reparse point traversal is disabled".to_string(),
            ));
        }

        let capabilities = NtfsVolumeCapabilities::resolve(request.path)?;
        let target_identity = FileIdentity::from_path(request.path)?;
        if target_identity.volume_serial != capabilities.volume_serial {
            return Err(RebeccaError::PlatformUnavailable(format!(
                "{EXPERIMENTAL_NTFS_MFT_BACKEND_LABEL} target volume identity changed while resolving {}",
                request.path.display()
            )));
        }

        let target_record_id = target_identity.file_reference.record_id;
        let (summary, source_label, shared_caveats) = match build_targeted_mft_summary(
            &capabilities,
            target_identity.file_reference,
            request.cancellation,
        ) {
            Ok((summary, caveats)) => (summary, TARGETED_MFT_SOURCE_LABEL, caveats),
            Err(err)
                if live_ntfs_mft_full_index_fallback_enabled()
                    && mft_record_source_error_can_fallback(&err) =>
            {
                let index = self
                    .cache
                    .load_or_build(&capabilities, request.cancellation)?;
                let Some(_) = index.mft_index.get(target_record_id) else {
                    return Err(RebeccaError::PlatformUnavailable(format!(
                        "{EXPERIMENTAL_NTFS_MFT_BACKEND_LABEL} could not map {} to MFT record {}",
                        request.path.display(),
                        target_record_id
                    )));
                };
                let mut summary = index.mft_index.aggregate_subtree(target_record_id);
                summary.caveats.push(ParseCaveat::new(
                    "mft-targeted-full-index-fallback",
                    format!(
                        "targeted NTFS/MFT traversal was unavailable ({err}); full-volume MFT index fallback was enabled by {LIVE_NTFS_MFT_FULL_INDEX_FALLBACK_ENV}"
                    ),
                ));
                (summary, index.source_label, index.caveats.clone())
            }
            Err(err) => return Err(err),
        };
        let report = ScanReport {
            bytes_scanned: summary.bytes,
            files_scanned: summary.files,
            directories_scanned: summary.directories,
        };
        let measured = MeasuredScan::exact(report, self.kind())
            .with_backend_source(mft_backend_source_label(source_label));

        Ok(with_bounded_mft_caveats(
            measured,
            shared_caveats.into_iter().chain(summary.caveats),
        ))
    }
}

pub(super) fn inspect_disk_map(
    cache: &WindowsNtfsMftIndexCache,
    path: &Path,
    options: DiskMapBackendOptions,
    cancellation: &ScanCancellationToken,
) -> Result<DiskMapBackendRoot> {
    check_not_cancelled(cancellation)?;
    let metadata = root_metadata(path)?;
    if is_reparse_like(&metadata) {
        return Err(RebeccaError::SafetyBlocked(
            "symlink or reparse point traversal is disabled".to_string(),
        ));
    }

    let capabilities = NtfsVolumeCapabilities::resolve(path)?;
    let target_identity = FileIdentity::from_path(path)?;
    if target_identity.volume_serial != capabilities.volume_serial {
        return Err(RebeccaError::PlatformUnavailable(format!(
            "{EXPERIMENTAL_NTFS_MFT_BACKEND_LABEL} target volume identity changed while resolving {}",
            path.display()
        )));
    }

    let target_record_id = target_identity.file_reference.record_id;
    if !is_volume_root_path(path, &capabilities.root_path)
        && !live_ntfs_mft_full_index_fallback_enabled()
    {
        return build_targeted_mft_disk_map(
            &capabilities,
            target_identity.file_reference,
            path,
            options,
            cancellation,
        );
    }

    let index = cache.load_or_build(&capabilities, cancellation)?;
    let target_entry = index
        .mft_index
        .get(target_record_id)
        .cloned()
        .ok_or_else(|| {
            RebeccaError::PlatformUnavailable(format!(
                "{EXPERIMENTAL_NTFS_MFT_BACKEND_LABEL} could not map {} to MFT record {}",
                path.display(),
                target_record_id
            ))
        })?;

    let mut top_entries = DiskMapTopEntries::new(
        options.top_limit,
        options.top_sort,
        options.entry_filter.clone(),
    );
    let mut groups = options.group_collector();
    let mut caveats = Vec::new();
    let mut visited_directories = BTreeSet::new();
    let max_depth = options.max_depth.unwrap_or(usize::MAX);
    let backend_source = mft_backend_source_label(index.source_label);
    let entry_provenance = EstimateProvenance::from_backend_confidence_and_source(
        ScanBackendKind::WindowsNtfsMftExperimental,
        ScanEstimateConfidence::Exact,
        Some(backend_source.clone()),
    );
    let aggregate = if target_entry.is_directory {
        caveats.extend(target_entry.caveats.clone());
        extend_mft_directory_edge_caveats(&index.mft_index, target_record_id, &mut caveats);
        visited_directories.insert(target_record_id);
        let mut aggregate = PhysicalMetricsAccumulator::default();
        for edge in index
            .mft_index
            .child_edges(target_record_id)
            .cloned()
            .collect::<Vec<_>>()
        {
            check_not_cancelled(cancellation)?;
            let Some(child) = index.mft_index.get(edge.child.record_id).cloned() else {
                continue;
            };
            let child_path = path.join(&edge.name);
            let child_aggregate = collect_mft_disk_map_entry(
                &index.mft_index,
                path,
                child_path,
                child,
                1,
                max_depth,
                &entry_provenance,
                &mut visited_directories,
                &mut caveats,
                &mut top_entries,
                &mut groups,
                capabilities.volume_serial,
                cancellation,
            )?;
            aggregate.absorb_child(child_aggregate);
        }
        aggregate
    } else {
        collect_mft_disk_map_entry(
            &index.mft_index,
            path,
            path.to_path_buf(),
            target_entry,
            0,
            max_depth,
            &entry_provenance,
            &mut visited_directories,
            &mut caveats,
            &mut top_entries,
            &mut groups,
            capabilities.volume_serial,
            cancellation,
        )?
    };
    let metrics = disk_map_metrics_from_physical(aggregate.into_metrics());

    let measured = MeasuredScan::exact(
        ScanReport {
            bytes_scanned: metrics.logical_bytes,
            files_scanned: metrics.files,
            directories_scanned: metrics.directories,
        },
        ScanBackendKind::WindowsNtfsMftExperimental,
    )
    .with_backend_source(backend_source);
    let measured =
        with_bounded_mft_caveats(measured, index.caveats.clone().into_iter().chain(caveats));

    Ok(DiskMapBackendRoot {
        metrics,
        top_entries: top_entries.into_sorted_entries(),
        groups,
        diagnostics: Vec::new(),
        estimate_provenance: EstimateProvenance::from_measured_scan(&measured),
    })
}

#[expect(
    clippy::too_many_arguments,
    reason = "recursive traversal carries bounded report state"
)]
fn collect_mft_disk_map_entry(
    index: &MftIndex,
    root: &Path,
    path: PathBuf,
    entry: MftIndexEntry,
    depth: usize,
    max_depth: usize,
    estimate_provenance: &EstimateProvenance,
    visited: &mut BTreeSet<u64>,
    caveats: &mut Vec<ParseCaveat>,
    top_entries: &mut DiskMapTopEntries,
    groups: &mut DiskMapGroupCollector,
    volume_serial_number: u64,
    cancellation: &ScanCancellationToken,
) -> Result<PhysicalMetricsAccumulator> {
    check_not_cancelled(cancellation)?;
    caveats.extend(entry.caveats.clone());
    if entry.is_reparse_point {
        caveats.push(ParseCaveat::new(
            "reparse-point-skipped",
            format!("record {} is a reparse point", entry.reference.record_id),
        ));
        return Ok(PhysicalMetricsAccumulator::default());
    }

    let mut aggregate = PhysicalMetricsAccumulator::default();
    if entry.is_directory {
        if !visited.insert(entry.reference.record_id) {
            caveats.push(ParseCaveat::new(
                "mft-index-cycle-skipped",
                format!(
                    "directory record {} appeared more than once in a subtree",
                    entry.reference.record_id
                ),
            ));
            return Ok(PhysicalMetricsAccumulator::default());
        }
        extend_mft_directory_edge_caveats(index, entry.reference.record_id, caveats);
        aggregate.record_directory();
    } else {
        aggregate.record_file_path(
            entry.reference.record_id,
            entry.logical_size,
            entry.allocated_size,
        );
    }

    if entry.is_directory {
        for edge in index
            .child_edges(entry.reference.record_id)
            .cloned()
            .collect::<Vec<_>>()
        {
            let Some(child) = index.get(edge.child.record_id).cloned() else {
                caveats.push(ParseCaveat::new(
                    "missing-record",
                    format!(
                        "record {} is not present in the MFT index",
                        edge.child.record_id
                    ),
                ));
                continue;
            };
            let child_path = path.join(&edge.name);
            let child_aggregate = collect_mft_disk_map_entry(
                index,
                root,
                child_path,
                child,
                depth.saturating_add(1),
                max_depth,
                estimate_provenance,
                visited,
                caveats,
                top_entries,
                groups,
                volume_serial_number,
                cancellation,
            )?;
            aggregate.absorb_child(child_aggregate);
        }
    }

    if !entry.is_directory {
        groups.record_file(
            &path,
            depth,
            entry.logical_size,
            entry.allocated_size,
            ntfs_filetime_to_system_time(Some(entry.modified_windows_filetime)),
            DiskMapMetadataSemantics::with_file_identity(DiskMapFileIdentity::new(
                volume_serial_number,
                entry.reference.record_id,
            )),
        );
    }

    if depth <= max_depth {
        let metrics = disk_map_metrics_from_physical(aggregate.metrics());
        top_entries.push(DiskMapEntry {
            path,
            root: root.to_path_buf(),
            kind: mft_disk_map_entry_kind(&entry),
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
        });
    }

    Ok(aggregate)
}

fn extend_mft_directory_edge_caveats(
    index: &MftIndex,
    parent_record_id: u64,
    caveats: &mut Vec<ParseCaveat>,
) {
    for edge in index.directory_edges(parent_record_id) {
        caveats.extend(edge.caveats.clone());
    }
}

fn mft_disk_map_entry_kind(entry: &MftIndexEntry) -> DiskMapEntryKind {
    if entry.is_directory {
        DiskMapEntryKind::Directory
    } else {
        DiskMapEntryKind::File
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NtfsVolumeCapabilities {
    root_path: PathBuf,
    device_path: String,
    mft_data_path: String,
    volume_serial: u64,
}

impl NtfsVolumeCapabilities {
    fn resolve(path: &Path) -> Result<Self> {
        let volume_paths = VolumePaths::from_path(path)?;
        if drive_type(&volume_paths.root_path) != DRIVE_FIXED {
            return Err(RebeccaError::PlatformUnavailable(format!(
                "{EXPERIMENTAL_NTFS_MFT_BACKEND_LABEL} only indexes local fixed NTFS volumes"
            )));
        }

        let info = volume_information(&volume_paths.root_path)?;
        if !info
            .file_system_name
            .eq_ignore_ascii_case(NTFS_FILE_SYSTEM_NAME)
        {
            return Err(RebeccaError::PlatformUnavailable(format!(
                "{EXPERIMENTAL_NTFS_MFT_BACKEND_LABEL} requires NTFS; {} uses {}",
                volume_paths.root_path.display(),
                info.file_system_name
            )));
        }

        Ok(Self {
            root_path: volume_paths.root_path,
            device_path: volume_paths.device_path,
            mft_data_path: volume_paths.mft_data_path,
            volume_serial: u64::from(info.volume_serial),
        })
    }

    fn cache_key(&self) -> String {
        format!("{}:{}", self.device_path, self.volume_serial)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VolumePaths {
    root_path: PathBuf,
    device_path: String,
    mft_data_path: String,
}

impl VolumePaths {
    fn from_path(path: &Path) -> Result<Self> {
        if !path.is_absolute() {
            return Err(RebeccaError::PlatformUnavailable(format!(
                "{EXPERIMENTAL_NTFS_MFT_BACKEND_LABEL} requires an absolute local path"
            )));
        }

        let Some(Component::Prefix(prefix)) = path.components().next() else {
            return Err(RebeccaError::PlatformUnavailable(format!(
                "{EXPERIMENTAL_NTFS_MFT_BACKEND_LABEL} could not resolve a drive root for {}",
                path.display()
            )));
        };

        let drive = match prefix.kind() {
            Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) => drive,
            Prefix::UNC(..)
            | Prefix::VerbatimUNC(..)
            | Prefix::DeviceNS(_)
            | Prefix::Verbatim(_) => {
                return Err(RebeccaError::PlatformUnavailable(format!(
                    "{EXPERIMENTAL_NTFS_MFT_BACKEND_LABEL} does not index UNC or device namespace paths"
                )));
            }
        };
        let drive = char::from(drive).to_ascii_uppercase();

        Ok(Self {
            root_path: PathBuf::from(format!("{drive}:\\")),
            device_path: format!("\\\\.\\{drive}:"),
            mft_data_path: format!("\\\\?\\{drive}:\\$MFT::$DATA"),
        })
    }
}

fn is_volume_root_path(path: &Path, expected_root: &Path) -> bool {
    if path == expected_root {
        return true;
    }

    let mut components = path.components();
    let is_drive_prefix = matches!(
        components.next(),
        Some(Component::Prefix(prefix))
            if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_))
    );
    is_drive_prefix
        && matches!(components.next(), Some(Component::RootDir))
        && components.next().is_none()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VolumeInformation {
    volume_serial: u32,
    file_system_name: String,
}

fn volume_information(root_path: &Path) -> Result<VolumeInformation> {
    let root = wide_null(root_path.as_os_str());
    let mut volume_serial = 0_u32;
    let mut file_system_name = [0_u16; 32];
    unsafe {
        GetVolumeInformationW(
            PCWSTR(root.as_ptr()),
            None,
            Some(&mut volume_serial),
            None,
            None,
            Some(&mut file_system_name),
        )
    }
    .map_err(|err| {
        RebeccaError::PlatformUnavailable(format!(
            "{EXPERIMENTAL_NTFS_MFT_BACKEND_LABEL} could not inspect volume {}: {}",
            root_path.display(),
            err.message()
        ))
    })?;

    Ok(VolumeInformation {
        volume_serial,
        file_system_name: wide_buffer_to_string(&file_system_name),
    })
}

fn drive_type(root_path: &Path) -> u32 {
    let root = wide_null(root_path.as_os_str());
    unsafe { GetDriveTypeW(PCWSTR(root.as_ptr())) }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileIdentity {
    volume_serial: u64,
    file_reference: NtfsFileReference,
}

impl FileIdentity {
    fn from_path(path: &Path) -> Result<Self> {
        let file = std::fs::OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES.0)
            .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0 | FILE_SHARE_DELETE.0)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0)
            .open(path)
            .map_err(|err| {
                RebeccaError::ScanFailed(ScanFailure::from_io(
                    path,
                    ScanFailurePhase::RootMetadata,
                    &err,
                ))
            })?;
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }.map_err(
            |err| {
                RebeccaError::PlatformUnavailable(format!(
                    "{EXPERIMENTAL_NTFS_MFT_BACKEND_LABEL} could not read file identity for {}: {}",
                    path.display(),
                    err.message()
                ))
            },
        )?;
        Ok(Self {
            volume_serial: u64::from(info.dwVolumeSerialNumber),
            file_reference: file_reference_from_number(
                (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
            ),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedNtfsRecords {
    source_label: &'static str,
    records: Vec<NtfsParsedRecord>,
    caveats: Vec<ParseCaveat>,
}

trait MftRecordSource {
    fn label(&self) -> &'static str;

    fn read_records(
        &self,
        volume_data: &NTFS_VOLUME_DATA_BUFFER,
        cancellation: &ScanCancellationToken,
        monitor: &NtfsMftBuildMonitor,
    ) -> Result<ParsedNtfsRecords>;
}

fn read_mft_records_from_sources(
    sources: &[&dyn MftRecordSource],
    volume_data: &NTFS_VOLUME_DATA_BUFFER,
    cancellation: &ScanCancellationToken,
    monitor: &NtfsMftBuildMonitor,
) -> Result<ParsedNtfsRecords> {
    let mut fallback_errors = Vec::new();

    for source in sources {
        check_mft_build_progress(cancellation, monitor)?;
        match source.read_records(volume_data, cancellation, monitor) {
            Ok(mut records) => {
                records.source_label = source.label();
                records.caveats.extend(
                    fallback_errors
                        .drain(..)
                        .map(|reason| ParseCaveat::new("mft-record-source-fallback", reason)),
                );
                return Ok(records);
            }
            Err(err) if monitor.is_timed_out() => return Err(err),
            Err(err) if mft_record_source_error_can_fallback(&err) => {
                fallback_errors.push(format!("{}: {err}", source.label()));
            }
            Err(err) => return Err(err),
        }
    }

    Err(RebeccaError::PlatformUnavailable(format!(
        "{EXPERIMENTAL_NTFS_MFT_BACKEND_LABEL} record sources are unavailable: {}",
        fallback_errors.join("; ")
    )))
}

fn mft_record_source_error_can_fallback(err: &RebeccaError) -> bool {
    matches!(
        err,
        RebeccaError::PlatformUnavailable(_) | RebeccaError::ScanFailed(_)
    )
}

fn mft_backend_source_label(source_label: &str) -> String {
    format!("{EXPERIMENTAL_NTFS_MFT_BACKEND_LABEL}-{source_label}")
}

#[derive(Debug, Default)]
struct MftParseErrorCaveats {
    total: usize,
    samples: Vec<ParseCaveat>,
}

impl MftParseErrorCaveats {
    fn record(&mut self, record_id: u64, error: impl fmt::Display) {
        self.total = self.total.saturating_add(1);
        if self.samples.len() < MAX_MFT_PARSE_ERROR_CAVEAT_SAMPLES {
            self.samples.push(ParseCaveat::new(
                "mft-record-parse-error",
                format!("record {record_id} could not be parsed: {error}"),
            ));
        }
    }

    fn append_to(self, caveats: &mut Vec<ParseCaveat>) {
        if self.total == 0 {
            return;
        }

        let sample_count = self.samples.len();
        caveats.extend(self.samples);
        let omitted = self.total.saturating_sub(sample_count);
        if omitted > 0 {
            caveats.push(ParseCaveat::new(
                "mft-record-parse-error-summary",
                format!(
                    "{omitted} additional MFT records could not be parsed; parse-error samples were capped at {sample_count}"
                ),
            ));
        }
    }
}

#[derive(Debug, Default)]
struct BoundedMftCaveatBucket {
    total: usize,
    samples: Vec<String>,
}

fn with_bounded_mft_caveats<I>(mut measured: MeasuredScan, caveats: I) -> MeasuredScan
where
    I: IntoIterator<Item = ParseCaveat>,
{
    let mut buckets: BTreeMap<String, BoundedMftCaveatBucket> = BTreeMap::new();
    for caveat in caveats {
        let bucket = buckets.entry(caveat.code).or_default();
        bucket.total = bucket.total.saturating_add(1);
        if bucket.samples.len() < MAX_MFT_ESTIMATE_CAVEAT_SAMPLES_PER_CODE {
            bucket.samples.push(caveat.message);
        }
    }

    for (code, bucket) in buckets {
        let sample_count = bucket.samples.len();
        let omitted = bucket.total.saturating_sub(sample_count);
        for message in bucket.samples {
            measured = measured.with_caveat(code.clone(), message);
        }
        if omitted > 0 {
            measured = measured.with_caveat(
                MFT_CAVEAT_SUMMARY_CODE,
                format!(
                    "{omitted} additional '{code}' caveats were omitted from this estimate; samples are capped at {MAX_MFT_ESTIMATE_CAVEAT_SAMPLES_PER_CODE} per caveat code"
                ),
            );
        }
    }

    measured
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct NtfsRecordGeometry {
    record_size: usize,
    sector_size: usize,
    bytes_per_cluster: u64,
    max_record_count: u64,
}

impl NtfsRecordGeometry {
    fn from_volume_data(device_path: &str, volume_data: &NTFS_VOLUME_DATA_BUFFER) -> Result<Self> {
        let record_size = usize::try_from(volume_data.BytesPerFileRecordSegment).unwrap_or(0);
        let sector_size = usize::try_from(volume_data.BytesPerSector).unwrap_or(0);
        let bytes_per_cluster = u64::from(volume_data.BytesPerCluster);
        let mft_valid_data_length = u64::try_from(volume_data.MftValidDataLength).unwrap_or(0);

        if record_size == 0 || sector_size == 0 || bytes_per_cluster == 0 {
            return Err(RebeccaError::PlatformUnavailable(format!(
                "{EXPERIMENTAL_NTFS_MFT_BACKEND_LABEL} received invalid NTFS record geometry from {device_path}"
            )));
        }
        if !record_size.is_multiple_of(sector_size) {
            return Err(RebeccaError::PlatformUnavailable(format!(
                "{EXPERIMENTAL_NTFS_MFT_BACKEND_LABEL} received unaligned NTFS record geometry from {device_path}"
            )));
        }
        if mft_valid_data_length == 0 {
            return Err(RebeccaError::PlatformUnavailable(format!(
                "{EXPERIMENTAL_NTFS_MFT_BACKEND_LABEL} received empty NTFS MFT metadata from {device_path}"
            )));
        }

        let max_record_count = mft_valid_data_length.saturating_div(record_size as u64);
        if max_record_count == 0 {
            return Err(RebeccaError::PlatformUnavailable(format!(
                "{EXPERIMENTAL_NTFS_MFT_BACKEND_LABEL} NTFS MFT metadata is smaller than one file record on {device_path}"
            )));
        }

        Ok(Self {
            record_size,
            sector_size,
            bytes_per_cluster,
            max_record_count,
        })
    }

    fn stream_geometry(self) -> NtfsStreamGeometry {
        NtfsStreamGeometry::new(self.bytes_per_cluster, self.sector_size)
    }
}

fn build_mft_index_from_records<S>(
    records: ParsedNtfsRecords,
    geometry: NtfsRecordGeometry,
    source: &mut S,
    cancellation: &ScanCancellationToken,
    monitor: &NtfsMftBuildMonitor,
) -> Result<(MftIndex, Vec<ParseCaveat>)>
where
    S: NtfsStreamSource,
{
    check_mft_build_progress(cancellation, monitor)?;
    let record_set = monitor.measure_checked(
        NtfsMftBuildStage::ResolveIndexAllocations,
        cancellation,
        || {
            Ok(NtfsRecordSet::resolve_with_stream_source(
                records.records,
                geometry.stream_geometry(),
                source,
            ))
        },
    )?;
    check_mft_build_progress(cancellation, monitor)?;
    let index = monitor.measure_checked(NtfsMftBuildStage::BuildMftIndex, cancellation, || {
        Ok(MftIndex::from_record_set(record_set))
    })?;
    check_mft_build_progress(cancellation, monitor)?;
    Ok((index, records.caveats))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TargetedMftTraversalLimits {
    max_records: usize,
    max_depth: usize,
}

impl Default for TargetedMftTraversalLimits {
    fn default() -> Self {
        Self {
            max_records: TARGETED_MFT_MAX_RECORDS,
            max_depth: TARGETED_MFT_MAX_DEPTH,
        }
    }
}

trait TargetedMftRecordResolver {
    fn resolve_record(&mut self, reference: NtfsFileReference) -> Result<Option<NtfsParsedRecord>>;
}

fn build_targeted_mft_summary(
    capabilities: &NtfsVolumeCapabilities,
    target_reference: NtfsFileReference,
    cancellation: &ScanCancellationToken,
) -> Result<(SubtreeSummary, Vec<ParseCaveat>)> {
    let monitor = NtfsMftBuildMonitor::from_environment();
    check_mft_build_progress(cancellation, &monitor)?;
    let volume = monitor.measure_checked(NtfsMftBuildStage::OpenVolume, cancellation, || {
        LiveNtfsVolume::open(capabilities)
    })?;
    let volume_data =
        monitor.measure_checked(NtfsMftBuildStage::ReadVolumeData, cancellation, || {
            volume.ntfs_volume_data()
        })?;
    let geometry = NtfsRecordGeometry::from_volume_data(&volume.device_path, &volume_data)?;
    let mut resolver = LiveNtfsTargetRecordResolver::new(&volume, geometry, cancellation, &monitor);
    let mut stream_source = LiveNtfsIndexStreamSource {
        volume: &volume,
        cancellation,
        monitor: &monitor,
    };
    let mut traversal = TargetedMftTraversal {
        resolver: &mut resolver,
        stream_source: &mut stream_source,
        geometry,
        cancellation,
        monitor: &monitor,
        limits: TargetedMftTraversalLimits::default(),
    };
    let summary = monitor.measure_checked(
        NtfsMftBuildStage::TargetedTraverseSubtree,
        cancellation,
        || traversal.aggregate_subtree(target_reference),
    )?;
    let mut caveats = Vec::new();
    resolver.append_parse_caveats(&mut caveats);
    if let Some(caveat) = monitor.timing_caveat() {
        caveats.push(caveat);
    }
    Ok((summary, caveats))
}

fn build_targeted_mft_disk_map(
    capabilities: &NtfsVolumeCapabilities,
    target_reference: NtfsFileReference,
    root_path: &Path,
    options: DiskMapBackendOptions,
    cancellation: &ScanCancellationToken,
) -> Result<DiskMapBackendRoot> {
    let monitor = NtfsMftBuildMonitor::from_environment();
    check_mft_build_progress(cancellation, &monitor)?;
    let volume = monitor.measure_checked(NtfsMftBuildStage::OpenVolume, cancellation, || {
        LiveNtfsVolume::open(capabilities)
    })?;
    let volume_data =
        monitor.measure_checked(NtfsMftBuildStage::ReadVolumeData, cancellation, || {
            volume.ntfs_volume_data()
        })?;
    let geometry = NtfsRecordGeometry::from_volume_data(&volume.device_path, &volume_data)?;
    let mut resolver = LiveNtfsTargetRecordResolver::new(&volume, geometry, cancellation, &monitor);
    let mut stream_source = LiveNtfsIndexStreamSource {
        volume: &volume,
        cancellation,
        monitor: &monitor,
    };
    let entry_provenance = EstimateProvenance::from_backend_confidence_and_source(
        ScanBackendKind::WindowsNtfsMftExperimental,
        ScanEstimateConfidence::Exact,
        Some(mft_backend_source_label(TARGETED_MFT_SOURCE_LABEL)),
    );
    let mut traversal = TargetedMftTraversal {
        resolver: &mut resolver,
        stream_source: &mut stream_source,
        geometry,
        cancellation,
        monitor: &monitor,
        limits: TargetedMftTraversalLimits::default(),
    };
    let targeted_map = monitor.measure_checked(
        NtfsMftBuildStage::TargetedTraverseSubtree,
        cancellation,
        || {
            traversal.collect_disk_map(
                target_reference,
                root_path,
                &options,
                capabilities.volume_serial,
                &entry_provenance,
            )
        },
    )?;
    let mut caveats = targeted_map.caveats.clone();
    resolver.append_parse_caveats(&mut caveats);
    if let Some(caveat) = monitor.timing_caveat() {
        caveats.push(caveat);
    }

    let measured = MeasuredScan::exact(
        ScanReport {
            bytes_scanned: targeted_map.metrics.logical_bytes,
            files_scanned: targeted_map.metrics.files,
            directories_scanned: targeted_map.metrics.directories,
        },
        ScanBackendKind::WindowsNtfsMftExperimental,
    )
    .with_backend_source(mft_backend_source_label(TARGETED_MFT_SOURCE_LABEL));
    let measured = with_bounded_mft_caveats(measured, caveats);

    Ok(DiskMapBackendRoot {
        metrics: targeted_map.metrics,
        top_entries: targeted_map.top_entries,
        groups: targeted_map.groups,
        diagnostics: Vec::new(),
        estimate_provenance: EstimateProvenance::from_measured_scan(&measured),
    })
}

struct LiveNtfsTargetRecordResolver<'a> {
    volume: &'a LiveNtfsVolume,
    geometry: NtfsRecordGeometry,
    cancellation: &'a ScanCancellationToken,
    monitor: &'a NtfsMftBuildMonitor,
    records: BTreeMap<u64, NtfsParsedRecord>,
    parse_errors: MftParseErrorCaveats,
}

impl<'a> LiveNtfsTargetRecordResolver<'a> {
    fn new(
        volume: &'a LiveNtfsVolume,
        geometry: NtfsRecordGeometry,
        cancellation: &'a ScanCancellationToken,
        monitor: &'a NtfsMftBuildMonitor,
    ) -> Self {
        Self {
            volume,
            geometry,
            cancellation,
            monitor,
            records: BTreeMap::new(),
            parse_errors: MftParseErrorCaveats::default(),
        }
    }

    fn append_parse_caveats(self, caveats: &mut Vec<ParseCaveat>) {
        self.parse_errors.append_to(caveats);
    }

    fn read_targeted_file_record(
        &self,
        reference: NtfsFileReference,
    ) -> Result<Option<(u64, Vec<u8>)>> {
        match self
            .volume
            .read_file_record(file_reference_number(reference), self.geometry.record_size)
        {
            Ok(record) => Ok(record),
            Err(err) if windows_error_matches(&err, ERROR_HANDLE_EOF) => Ok(None),
            Err(err) if windows_error_matches(&err, ERROR_INVALID_PARAMETER) => Ok(None),
            Err(err) => Err(RebeccaError::PlatformUnavailable(format!(
                "{EXPERIMENTAL_NTFS_MFT_BACKEND_LABEL} {TARGETED_MFT_SOURCE_LABEL} could not read MFT record {} from {}: {}",
                reference.record_id,
                self.volume.device_path,
                err.message()
            ))),
        }
    }
}

impl TargetedMftRecordResolver for LiveNtfsTargetRecordResolver<'_> {
    fn resolve_record(&mut self, reference: NtfsFileReference) -> Result<Option<NtfsParsedRecord>> {
        check_mft_build_progress(self.cancellation, self.monitor)?;
        if let Some(record) = self.records.get(&reference.record_id) {
            return Ok(Some(record.clone()));
        }
        if reference.record_id >= self.geometry.max_record_count {
            return Ok(None);
        }

        let read = self.monitor.measure_checked(
            NtfsMftBuildStage::TargetedReadRecord,
            self.cancellation,
            || self.read_targeted_file_record(reference),
        )?;
        let Some((record_id, raw_record)) = read else {
            return Ok(None);
        };

        let parsed_record_id = low_file_reference_number(record_id);
        if parsed_record_id != reference.record_id {
            return Ok(None);
        }
        match NtfsParsedRecord::parse_fsctl_file_record(
            parsed_record_id,
            &raw_record,
            self.geometry.sector_size,
        ) {
            Ok(record) => {
                self.records.insert(parsed_record_id, record.clone());
                Ok(Some(record))
            }
            Err(err) => {
                let message = err.to_string();
                let record_len = raw_record.len();
                let signature = record_signature_hex(&raw_record);
                self.parse_errors.record(parsed_record_id, err);
                Err(RebeccaError::PlatformUnavailable(format!(
                    "{EXPERIMENTAL_NTFS_MFT_BACKEND_LABEL} {TARGETED_MFT_SOURCE_LABEL} could not parse targeted MFT record {parsed_record_id} ({record_len} bytes, signature {signature}): {message}"
                )))
            }
        }
    }
}

fn record_signature_hex(record: &[u8]) -> String {
    record
        .iter()
        .take(4)
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join("")
}

struct TargetedMftTraversal<'a, R, S>
where
    R: TargetedMftRecordResolver,
    S: NtfsStreamSource,
{
    resolver: &'a mut R,
    stream_source: &'a mut S,
    geometry: NtfsRecordGeometry,
    cancellation: &'a ScanCancellationToken,
    monitor: &'a NtfsMftBuildMonitor,
    limits: TargetedMftTraversalLimits,
}

impl<R, S> TargetedMftTraversal<'_, R, S>
where
    R: TargetedMftRecordResolver,
    S: NtfsStreamSource,
{
    fn aggregate_subtree(&mut self, root: NtfsFileReference) -> Result<SubtreeSummary> {
        let mut summary = SubtreeSummary::default();
        let mut stack = vec![TargetedTraversalNode {
            reference: root,
            depth: 0,
            directory_entry: None,
        }];
        let mut visited = std::collections::BTreeSet::new();
        let mut traversal_attempts = 0_usize;

        while let Some(node) = stack.pop() {
            check_mft_build_progress(self.cancellation, self.monitor)?;
            if traversal_attempts >= self.limits.max_records {
                return Err(targeted_mft_unavailable(format!(
                    "targeted traversal exceeded the {} record candidate budget",
                    self.limits.max_records
                )));
            }
            traversal_attempts = traversal_attempts.saturating_add(1);

            let Some(record) = self.resolver.resolve_record(node.reference)? else {
                if node.directory_entry.is_none() {
                    return Err(targeted_mft_unavailable(format!(
                        "target root record {} could not be resolved",
                        node.reference.record_id
                    )));
                }
                summary.caveats.push(ParseCaveat::new(
                    "missing-record",
                    format!(
                        "record {} is not present in the targeted MFT traversal",
                        node.reference.record_id
                    ),
                ));
                continue;
            };
            if reference_sequence_mismatches(node.reference, record.reference) {
                summary.caveats.push(ParseCaveat::new(
                    "directory-index-child-sequence-mismatch",
                    format!(
                        "targeted traversal expected record {} sequence {:?}, but current sequence is {:?}",
                        node.reference.record_id,
                        node.reference.sequence_number,
                        record.reference.sequence_number
                    ),
                ));
                continue;
            }
            if !visited.insert(record.reference.record_id) {
                summary.caveats.push(ParseCaveat::new(
                    "mft-targeted-record-already-counted",
                    format!(
                        "record {} appeared more than once in targeted subtree traversal",
                        record.reference.record_id
                    ),
                ));
                continue;
            }

            let record = self.resolve_record(record)?;
            summary.caveats.extend(record.caveats.clone());
            if let Some(directory_entry) = &node.directory_entry {
                self.push_directory_entry_parent_caveat(&record, directory_entry, &mut summary);
            }
            if record.non_dos_file_name_count() > 1 {
                summary.caveats.push(ParseCaveat::new(
                    "hardlink-path-candidates",
                    format!(
                        "record {} has multiple non-DOS file names; targeted traversal counted the record once",
                        record.reference.record_id
                    ),
                ));
            }
            if !record.in_use {
                continue;
            }
            if record.is_reparse_point {
                summary.caveats.push(ParseCaveat::new(
                    "reparse-point-skipped",
                    format!("record {} is a reparse point", record.reference.record_id),
                ));
                continue;
            }

            if record.is_directory {
                summary.directories = summary.directories.saturating_add(1);
                if node.depth >= self.limits.max_depth && !record.directory_entries.is_empty() {
                    return Err(targeted_mft_unavailable(format!(
                        "targeted traversal reached depth {} below directory record {}",
                        node.depth, record.reference.record_id
                    )));
                }
                self.push_directory_children(&record, node.depth, &mut stack, &mut summary);
            } else {
                let files_before = summary.files;
                summary.files = summary.files.saturating_add(1);
                summary.bytes = summary.bytes.saturating_add(record.cleanup_logical_size());
                summary.allocated_bytes = add_file_allocated_bytes(
                    summary.allocated_bytes,
                    files_before,
                    record.cleanup_allocated_size(),
                );
            }
        }

        Ok(summary)
    }

    fn collect_disk_map(
        &mut self,
        root: NtfsFileReference,
        root_path: &Path,
        options: &DiskMapBackendOptions,
        volume_serial_number: u64,
        entry_provenance: &EstimateProvenance,
    ) -> Result<TargetedDiskMap> {
        let mut state = TargetedDiskMapState::new(options);
        let root_node = TargetedDiskMapNode {
            reference: root,
            path: root_path.to_path_buf(),
            depth: 0,
            directory_entry: None,
        };
        let context = TargetedDiskMapContext {
            root_path,
            max_visible_depth: options.max_depth.unwrap_or(usize::MAX),
            volume_serial_number,
            entry_provenance,
        };
        let aggregate = self.collect_disk_map_record(root_node, false, &context, &mut state)?;

        Ok(TargetedDiskMap {
            metrics: disk_map_metrics_from_physical(aggregate.into_metrics()),
            top_entries: state.top_entries.into_sorted_entries(),
            groups: state.groups,
            caveats: state.caveats,
        })
    }

    fn collect_disk_map_record(
        &mut self,
        node: TargetedDiskMapNode,
        include_root_directory: bool,
        context: &TargetedDiskMapContext<'_>,
        state: &mut TargetedDiskMapState,
    ) -> Result<PhysicalMetricsAccumulator> {
        check_mft_build_progress(self.cancellation, self.monitor)?;
        state.record_attempt(self.limits.max_records)?;

        let Some(record) = self.resolver.resolve_record(node.reference)? else {
            if node.directory_entry.is_none() {
                return Err(targeted_mft_unavailable(format!(
                    "target root record {} could not be resolved",
                    node.reference.record_id
                )));
            }
            state.caveats.push(ParseCaveat::new(
                "missing-record",
                format!(
                    "record {} is not present in the targeted MFT traversal",
                    node.reference.record_id
                ),
            ));
            return Ok(PhysicalMetricsAccumulator::default());
        };
        if reference_sequence_mismatches(node.reference, record.reference) {
            state.caveats.push(ParseCaveat::new(
                "directory-index-child-sequence-mismatch",
                format!(
                    "targeted traversal expected record {} sequence {:?}, but current sequence is {:?}",
                    node.reference.record_id,
                    node.reference.sequence_number,
                    record.reference.sequence_number
                ),
            ));
            return Ok(PhysicalMetricsAccumulator::default());
        }

        let record = self.resolve_record(record)?;
        state.caveats.extend(record.caveats.clone());
        if let Some(directory_entry) = &node.directory_entry {
            self.push_directory_entry_parent_caveat_to(
                &record,
                directory_entry,
                &mut state.caveats,
            );
        }
        if record.non_dos_file_name_count() > 1 {
            state.caveats.push(ParseCaveat::new(
                "hardlink-path-candidates",
                format!(
                    "record {} has multiple non-DOS file names; targeted disk-map path metrics preserve visible names and unique metrics count the physical record once",
                    record.reference.record_id
                ),
            ));
        }
        if !record.in_use {
            return Ok(PhysicalMetricsAccumulator::default());
        }
        if record.is_reparse_point {
            state.caveats.push(ParseCaveat::new(
                "reparse-point-skipped",
                format!("record {} is a reparse point", record.reference.record_id),
            ));
            return Ok(PhysicalMetricsAccumulator::default());
        }

        let mut aggregate = PhysicalMetricsAccumulator::default();
        if record.is_directory {
            if !state.visited_directories.insert(record.reference.record_id) {
                state.caveats.push(ParseCaveat::new(
                    "mft-targeted-directory-already-visited",
                    format!(
                        "directory record {} appeared more than once in targeted disk-map traversal",
                        record.reference.record_id
                    ),
                ));
                return Ok(PhysicalMetricsAccumulator::default());
            }
            if node.depth >= self.limits.max_depth && !record.directory_entries.is_empty() {
                return Err(targeted_mft_unavailable(format!(
                    "targeted traversal reached depth {} below directory record {}",
                    node.depth, record.reference.record_id
                )));
            }
            if include_root_directory || node.directory_entry.is_some() {
                aggregate.record_directory();
            }
            self.collect_disk_map_children(
                &record,
                &node.path,
                node.depth,
                context,
                state,
                &mut aggregate,
            )?;
        } else {
            aggregate.record_file_path(
                record.reference.record_id,
                record.cleanup_logical_size(),
                record.cleanup_allocated_size(),
            );
        }

        if !record.is_directory {
            state.groups.record_file(
                &node.path,
                node.depth,
                record.cleanup_logical_size(),
                record.cleanup_allocated_size(),
                ntfs_filetime_to_system_time(
                    record
                        .primary_file_name()
                        .map(|file_name| file_name.modified_windows_filetime),
                ),
                DiskMapMetadataSemantics::with_file_identity(DiskMapFileIdentity::new(
                    context.volume_serial_number,
                    record.reference.record_id,
                )),
            );
        }

        let should_push_entry =
            !record.is_directory || include_root_directory || node.directory_entry.is_some();
        if should_push_entry && node.depth <= context.max_visible_depth {
            let metrics = disk_map_metrics_from_physical(aggregate.metrics());
            state.top_entries.push(DiskMapEntry {
                path: node.path,
                root: context.root_path.to_path_buf(),
                kind: if record.is_directory {
                    DiskMapEntryKind::Directory
                } else {
                    DiskMapEntryKind::File
                },
                depth: node.depth,
                logical_bytes: metrics.logical_bytes,
                allocated_bytes: metrics.allocated_bytes,
                unique_logical_bytes: metrics.unique_logical_bytes,
                unique_allocated_bytes: metrics.unique_allocated_bytes,
                files: metrics.files,
                directories: metrics.directories,
                estimate_source: EstimateSource::FreshScan,
                estimate_provenance: context.entry_provenance.clone(),
                cleanup_advice: None,
            });
        }

        Ok(aggregate)
    }

    fn collect_disk_map_children(
        &mut self,
        record: &NtfsParsedRecord,
        parent_path: &Path,
        parent_depth: usize,
        context: &TargetedDiskMapContext<'_>,
        state: &mut TargetedDiskMapState,
        aggregate: &mut PhysicalMetricsAccumulator,
    ) -> Result<()> {
        for entry in &record.directory_entries {
            if is_dos_directory_entry(entry) {
                continue;
            }
            if entry.parent.record_id != record.reference.record_id {
                state.caveats.push(ParseCaveat::new(
                    "directory-index-parent-mismatch",
                    format!(
                        "$I30 entry '{}' declares parent {}, but was stored on directory {}",
                        entry.name, entry.parent.record_id, record.reference.record_id
                    ),
                ));
                continue;
            }
            if reference_sequence_mismatches(entry.parent, record.reference) {
                state.caveats.push(ParseCaveat::new(
                    "parent-sequence-mismatch",
                    format!(
                        "$I30 entry '{}' references parent {} sequence {:?}, but current sequence is {:?}",
                        entry.name,
                        entry.parent.record_id,
                        entry.parent.sequence_number,
                        record.reference.sequence_number
                    ),
                ));
                continue;
            }

            let child = TargetedDiskMapNode {
                reference: entry.child,
                path: parent_path.join(&entry.name),
                depth: parent_depth.saturating_add(1),
                directory_entry: Some(entry.clone()),
            };
            let child_aggregate = self.collect_disk_map_record(child, true, context, state)?;
            aggregate.absorb_child(child_aggregate);
        }

        Ok(())
    }

    fn push_directory_entry_parent_caveat(
        &self,
        record: &NtfsParsedRecord,
        directory_entry: &NtfsDirectoryEntry,
        summary: &mut SubtreeSummary,
    ) {
        self.push_directory_entry_parent_caveat_to(record, directory_entry, &mut summary.caveats);
    }

    fn push_directory_entry_parent_caveat_to(
        &self,
        record: &NtfsParsedRecord,
        directory_entry: &NtfsDirectoryEntry,
        caveats: &mut Vec<ParseCaveat>,
    ) {
        let parent_edge_exists = record.names.iter().any(|name| {
            !matches!(name.namespace, rebecca_ntfs::FileNameNamespace::Dos)
                && name.parent == directory_entry.parent
                && name.name.eq_ignore_ascii_case(&directory_entry.name)
        });
        if !parent_edge_exists {
            caveats.push(ParseCaveat::new(
                "directory-index-parent-map-fallback",
                format!(
                    "$I30 entry '{}' was used because it is not present in $FILE_NAME parent edges for directory {}",
                    directory_entry.name, directory_entry.parent.record_id
                ),
            ));
        }
    }

    fn resolve_record(&mut self, record: NtfsParsedRecord) -> Result<NtfsParsedRecord> {
        let resolver = &mut self.resolver;
        self.monitor.measure_checked(
            NtfsMftBuildStage::TargetedResolveRecord,
            self.cancellation,
            || {
                resolve_record_with_stream_source(
                    record,
                    self.geometry.stream_geometry(),
                    self.stream_source,
                    |reference| resolver.resolve_record(reference),
                )
            },
        )
    }

    fn push_directory_children(
        &mut self,
        record: &NtfsParsedRecord,
        depth: usize,
        stack: &mut Vec<TargetedTraversalNode>,
        summary: &mut SubtreeSummary,
    ) {
        for entry in &record.directory_entries {
            if is_dos_directory_entry(entry) {
                continue;
            }
            if entry.parent.record_id != record.reference.record_id {
                summary.caveats.push(ParseCaveat::new(
                    "directory-index-parent-mismatch",
                    format!(
                        "$I30 entry '{}' declares parent {}, but was stored on directory {}",
                        entry.name, entry.parent.record_id, record.reference.record_id
                    ),
                ));
                continue;
            }
            if reference_sequence_mismatches(entry.parent, record.reference) {
                summary.caveats.push(ParseCaveat::new(
                    "parent-sequence-mismatch",
                    format!(
                        "$I30 entry '{}' references parent {} sequence {:?}, but current sequence is {:?}",
                        entry.name,
                        entry.parent.record_id,
                        entry.parent.sequence_number,
                        record.reference.sequence_number
                    ),
                ));
                continue;
            }
            stack.push(TargetedTraversalNode {
                reference: entry.child,
                depth: depth.saturating_add(1),
                directory_entry: Some(entry.clone()),
            });
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TargetedTraversalNode {
    reference: NtfsFileReference,
    depth: usize,
    directory_entry: Option<NtfsDirectoryEntry>,
}

#[derive(Debug)]
struct TargetedDiskMap {
    metrics: DiskMapMetrics,
    top_entries: Vec<DiskMapEntry>,
    groups: DiskMapGroupCollector,
    caveats: Vec<ParseCaveat>,
}

fn disk_map_metrics_from_physical(metrics: PhysicalMetrics) -> DiskMapMetrics {
    DiskMapMetrics {
        logical_bytes: metrics.logical_bytes,
        allocated_bytes: metrics.allocated_bytes,
        unique_logical_bytes: (metrics.files > 0).then_some(metrics.unique_logical_bytes),
        unique_allocated_bytes: metrics.unique_allocated_bytes,
        files: metrics.files,
        directories: metrics.directories,
    }
}

#[derive(Debug, Clone, Copy)]
struct TargetedDiskMapContext<'a> {
    root_path: &'a Path,
    max_visible_depth: usize,
    volume_serial_number: u64,
    entry_provenance: &'a EstimateProvenance,
}

#[derive(Debug)]
struct TargetedDiskMapState {
    visited_directories: BTreeSet<u64>,
    traversal_attempts: usize,
    top_entries: DiskMapTopEntries,
    groups: DiskMapGroupCollector,
    caveats: Vec<ParseCaveat>,
}

impl TargetedDiskMapState {
    fn new(options: &DiskMapBackendOptions) -> Self {
        Self {
            visited_directories: BTreeSet::new(),
            traversal_attempts: 0,
            top_entries: DiskMapTopEntries::new(
                options.top_limit,
                options.top_sort,
                options.entry_filter.clone(),
            ),
            groups: options.group_collector(),
            caveats: Vec::new(),
        }
    }

    fn record_attempt(&mut self, max_records: usize) -> Result<()> {
        if self.traversal_attempts >= max_records {
            return Err(targeted_mft_unavailable(format!(
                "targeted traversal exceeded the {max_records} record candidate budget"
            )));
        }
        self.traversal_attempts = self.traversal_attempts.saturating_add(1);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TargetedDiskMapNode {
    reference: NtfsFileReference,
    path: PathBuf,
    depth: usize,
    directory_entry: Option<NtfsDirectoryEntry>,
}

fn targeted_mft_unavailable(reason: String) -> RebeccaError {
    RebeccaError::PlatformUnavailable(format!(
        "{EXPERIMENTAL_NTFS_MFT_BACKEND_LABEL} {TARGETED_MFT_SOURCE_LABEL} {reason}"
    ))
}

fn reference_sequence_mismatches(expected: NtfsFileReference, actual: NtfsFileReference) -> bool {
    if expected.record_id != actual.record_id {
        return true;
    }
    matches!(
        (expected.sequence_number, actual.sequence_number),
        (Some(expected), Some(actual)) if expected != 0 && actual != 0 && expected != actual
    )
}

fn is_dos_directory_entry(entry: &NtfsDirectoryEntry) -> bool {
    matches!(entry.namespace, rebecca_ntfs::FileNameNamespace::Dos)
}

fn add_file_allocated_bytes(
    current: Option<u64>,
    files_before: u64,
    file_allocated: Option<u64>,
) -> Option<u64> {
    match (current, file_allocated) {
        (None, Some(right)) if files_before == 0 => Some(right),
        (Some(left), Some(right)) => Some(left.saturating_add(right)),
        _ => None,
    }
}

const WINDOWS_TICK_SECONDS: u64 = 10_000_000;
const WINDOWS_TO_UNIX_EPOCH_SECONDS: u64 = 11_644_473_600;

fn ntfs_filetime_to_system_time(filetime: Option<u64>) -> Option<SystemTime> {
    let filetime = filetime?;
    if filetime == 0 {
        return None;
    }

    let seconds = filetime / WINDOWS_TICK_SECONDS;
    let ticks = filetime % WINDOWS_TICK_SECONDS;
    if seconds < WINDOWS_TO_UNIX_EPOCH_SECONDS {
        return None;
    }

    Some(
        UNIX_EPOCH
            + Duration::from_secs(seconds - WINDOWS_TO_UNIX_EPOCH_SECONDS)
            + Duration::from_nanos(ticks.saturating_mul(100)),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MftExtent {
    starting_vcn: u64,
    lcn: u64,
    cluster_count: u64,
}

fn parse_retrieval_pointer_extents(buffer: &[u8]) -> Result<Vec<MftExtent>> {
    let header_size = offset_of!(RETRIEVAL_POINTERS_BUFFER, Extents);
    if buffer.len() < header_size {
        return Err(RebeccaError::PlatformUnavailable(format!(
            "{EXPERIMENTAL_NTFS_MFT_BACKEND_LABEL} retrieval pointer buffer is truncated"
        )));
    }

    let extent_count = unsafe {
        ptr::read_unaligned(
            buffer
                .as_ptr()
                .add(offset_of!(RETRIEVAL_POINTERS_BUFFER, ExtentCount))
                .cast::<u32>(),
        )
    };
    let extent_count = usize::try_from(extent_count).unwrap_or(usize::MAX);
    if extent_count == 0 {
        return Err(RebeccaError::PlatformUnavailable(format!(
            "{EXPERIMENTAL_NTFS_MFT_BACKEND_LABEL} $MFT retrieval pointer list is empty"
        )));
    }

    let extents_size = extent_count
        .checked_mul(size_of::<RETRIEVAL_POINTERS_BUFFER_0>())
        .ok_or_else(|| {
            RebeccaError::PlatformUnavailable(format!(
                "{EXPERIMENTAL_NTFS_MFT_BACKEND_LABEL} retrieval pointer extent count overflowed"
            ))
        })?;
    let required_len = header_size.checked_add(extents_size).ok_or_else(|| {
        RebeccaError::PlatformUnavailable(format!(
            "{EXPERIMENTAL_NTFS_MFT_BACKEND_LABEL} retrieval pointer buffer length overflowed"
        ))
    })?;
    if buffer.len() < required_len {
        return Err(RebeccaError::PlatformUnavailable(format!(
            "{EXPERIMENTAL_NTFS_MFT_BACKEND_LABEL} retrieval pointer buffer ended before all extents"
        )));
    }

    let mut starting_vcn = unsafe {
        ptr::read_unaligned(
            buffer
                .as_ptr()
                .add(offset_of!(RETRIEVAL_POINTERS_BUFFER, StartingVcn))
                .cast::<i64>(),
        )
    };
    if starting_vcn < 0 {
        return Err(RebeccaError::PlatformUnavailable(format!(
            "{EXPERIMENTAL_NTFS_MFT_BACKEND_LABEL} $MFT retrieval pointer list has a negative starting VCN"
        )));
    }

    let mut extents = Vec::with_capacity(extent_count);
    for index in 0..extent_count {
        let offset = header_size + (index * size_of::<RETRIEVAL_POINTERS_BUFFER_0>());
        let raw = unsafe {
            ptr::read_unaligned(
                buffer
                    .as_ptr()
                    .add(offset)
                    .cast::<RETRIEVAL_POINTERS_BUFFER_0>(),
            )
        };
        if raw.NextVcn <= starting_vcn {
            return Err(RebeccaError::PlatformUnavailable(format!(
                "{EXPERIMENTAL_NTFS_MFT_BACKEND_LABEL} $MFT retrieval pointer extent {index} is not ordered"
            )));
        }
        if raw.Lcn < 0 {
            return Err(RebeccaError::PlatformUnavailable(format!(
                "{EXPERIMENTAL_NTFS_MFT_BACKEND_LABEL} $MFT retrieval pointer extent {index} is sparse"
            )));
        }

        extents.push(MftExtent {
            starting_vcn: starting_vcn as u64,
            lcn: raw.Lcn as u64,
            cluster_count: (raw.NextVcn - starting_vcn) as u64,
        });
        starting_vcn = raw.NextVcn;
    }

    Ok(extents)
}

fn next_mft_chunk_len(
    bytes_remaining_in_extent: u64,
    records_remaining: u64,
    record_size: usize,
) -> usize {
    if record_size == 0 || records_remaining == 0 {
        return 0;
    }

    let chunk_limit = SEQUENTIAL_MFT_CHUNK_BYTES.max(record_size) as u64;
    let record_bytes_remaining = records_remaining.saturating_mul(record_size as u64);
    let bytes_to_read = bytes_remaining_in_extent
        .min(record_bytes_remaining)
        .min(chunk_limit);
    usize::try_from(bytes_to_read - (bytes_to_read % record_size as u64)).unwrap_or(0)
}

fn sequential_mft_parse_window_chunks() -> usize {
    bounded_parallelism_budget().min(SEQUENTIAL_MFT_PARSE_WINDOW_CHUNKS)
}

fn run_scoped_mft_parse<R, F>(work: F) -> R
where
    F: FnOnce() -> R + Send,
    R: Send,
{
    run_scoped_parallel_work(&MFT_PARSE_THREAD_POOL, "ntfs-mft-parse", work)
}

#[derive(Debug)]
struct SequentialMftChunk {
    base_record_id: u64,
    bytes: Vec<u8>,
}

fn parse_sequential_mft_chunks(
    reader: &MftRecordReader,
    cancellation: &ScanCancellationToken,
    chunks: &[SequentialMftChunk],
) -> Result<Vec<MftRecordBatch>> {
    run_scoped_mft_parse(|| {
        chunks
            .par_iter()
            .map(|chunk| {
                check_not_cancelled(cancellation)?;
                Ok(reader.parse_records_from(chunk.base_record_id, &chunk.bytes))
            })
            .collect::<Result<Vec<_>>>()
    })
}

struct LiveNtfsVolume {
    handle: HANDLE,
    device_path: String,
    mft_data_path: String,
}

impl LiveNtfsVolume {
    fn open(capabilities: &NtfsVolumeCapabilities) -> Result<Self> {
        let device = wide_null(OsStr::new(&capabilities.device_path));
        let share_mode =
            FILE_SHARE_MODE(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0 | FILE_SHARE_DELETE.0);
        let flags = FILE_FLAGS_AND_ATTRIBUTES(FILE_FLAG_BACKUP_SEMANTICS.0);
        let handle = unsafe {
            CreateFileW(
                PCWSTR(device.as_ptr()),
                windows::Win32::Foundation::GENERIC_READ.0,
                share_mode,
                None,
                OPEN_EXISTING,
                flags,
                None,
            )
        }
        .map_err(|err| volume_open_error(&capabilities.device_path, &err))?;

        Ok(Self {
            handle,
            device_path: capabilities.device_path.clone(),
            mft_data_path: capabilities.mft_data_path.clone(),
        })
    }

    fn ntfs_volume_data(&self) -> Result<NTFS_VOLUME_DATA_BUFFER> {
        let mut volume_data = NTFS_VOLUME_DATA_BUFFER::default();
        let mut bytes_returned = 0_u32;
        unsafe {
            DeviceIoControl(
                self.handle,
                FSCTL_GET_NTFS_VOLUME_DATA,
                None,
                0,
                Some((&mut volume_data as *mut NTFS_VOLUME_DATA_BUFFER).cast()),
                size_of::<NTFS_VOLUME_DATA_BUFFER>() as u32,
                Some(&mut bytes_returned),
                None,
            )
        }
        .map_err(|err| {
            RebeccaError::PlatformUnavailable(format!(
                "{EXPERIMENTAL_NTFS_MFT_BACKEND_LABEL} could not read NTFS volume data from {}: {}",
                self.device_path,
                err.message()
            ))
        })?;

        Ok(volume_data)
    }

    fn read_mft_records(
        &self,
        volume_data: &NTFS_VOLUME_DATA_BUFFER,
        cancellation: &ScanCancellationToken,
        monitor: &NtfsMftBuildMonitor,
    ) -> Result<ParsedNtfsRecords> {
        let sequential_source = SequentialMftDataSource { volume: self };
        let fsctl_source = FsctlRecordMftSource { volume: self };
        read_mft_records_from_sources(
            &[&sequential_source, &fsctl_source],
            volume_data,
            cancellation,
            monitor,
        )
    }

    fn open_mft_data_stream(&self) -> Result<LiveNtfsMetadataFile> {
        let path = wide_null(OsStr::new(&self.mft_data_path));
        let share_mode =
            FILE_SHARE_MODE(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0 | FILE_SHARE_DELETE.0);
        let flags =
            FILE_FLAGS_AND_ATTRIBUTES(FILE_FLAG_OPEN_REPARSE_POINT.0 | FILE_FLAG_SEQUENTIAL_SCAN.0);
        let desired_access = FILE_READ_ATTRIBUTES.0 | SYNCHRONIZE.0;
        let handle = unsafe {
            CreateFileW(
                PCWSTR(path.as_ptr()),
                desired_access,
                share_mode,
                None,
                OPEN_EXISTING,
                flags,
                None,
            )
        }
        .map_err(|err| {
            RebeccaError::PlatformUnavailable(format!(
                "{EXPERIMENTAL_NTFS_MFT_BACKEND_LABEL} {SEQUENTIAL_MFT_SOURCE_LABEL} could not open {} read-only: {}",
                self.mft_data_path,
                err.message()
            ))
        })?;

        Ok(LiveNtfsMetadataFile { handle })
    }

    fn mft_extents(
        &self,
        mft_data: &LiveNtfsMetadataFile,
        cancellation: &ScanCancellationToken,
        monitor: &NtfsMftBuildMonitor,
    ) -> Result<Vec<MftExtent>> {
        let mut input = STARTING_VCN_INPUT_BUFFER { StartingVcn: 0 };
        let mut output = vec![
            0_u8;
            offset_of!(RETRIEVAL_POINTERS_BUFFER, Extents)
                + (32 * size_of::<RETRIEVAL_POINTERS_BUFFER_0>())
        ];

        loop {
            check_mft_build_progress(cancellation, monitor)?;
            let mut bytes_returned = 0_u32;
            let result = unsafe {
                DeviceIoControl(
                    mft_data.handle,
                    FSCTL_GET_RETRIEVAL_POINTERS,
                    Some((&mut input as *mut STARTING_VCN_INPUT_BUFFER).cast()),
                    size_of::<STARTING_VCN_INPUT_BUFFER>() as u32,
                    Some(output.as_mut_ptr().cast()),
                    output.len() as u32,
                    Some(&mut bytes_returned),
                    None,
                )
            };

            match result {
                Ok(()) => {
                    let returned = usize::try_from(bytes_returned).unwrap_or(0);
                    return parse_retrieval_pointer_extents(&output[..returned]);
                }
                Err(err)
                    if windows_error_matches(&err, ERROR_MORE_DATA)
                        && output.len() < MAX_RETRIEVAL_POINTER_BUFFER_BYTES =>
                {
                    let next_len = output
                        .len()
                        .saturating_mul(2)
                        .min(MAX_RETRIEVAL_POINTER_BUFFER_BYTES);
                    output.resize(next_len, 0);
                }
                Err(err) => {
                    return Err(RebeccaError::PlatformUnavailable(format!(
                        "{EXPERIMENTAL_NTFS_MFT_BACKEND_LABEL} {SEQUENTIAL_MFT_SOURCE_LABEL} could not read $MFT retrieval pointers from {}: {}",
                        self.mft_data_path,
                        err.message()
                    )));
                }
            }
        }
    }

    fn read_volume_bytes(&self, offset: u64, len: usize) -> Result<Vec<u8>> {
        let offset = i64::try_from(offset).map_err(|_| {
            RebeccaError::PlatformUnavailable(format!(
                "{EXPERIMENTAL_NTFS_MFT_BACKEND_LABEL} {SEQUENTIAL_MFT_SOURCE_LABEL} volume offset overflowed"
            ))
        })?;
        let mut buffer = vec![0_u8; len];
        let mut bytes_read = 0_u32;
        unsafe {
            SetFilePointerEx(self.handle, offset, None, FILE_BEGIN).map_err(|err| {
                RebeccaError::PlatformUnavailable(format!(
                    "{EXPERIMENTAL_NTFS_MFT_BACKEND_LABEL} {SEQUENTIAL_MFT_SOURCE_LABEL} could not seek {} to byte {offset}: {}",
                    self.device_path,
                    err.message()
                ))
            })?;
            ReadFile(
                self.handle,
                Some(&mut buffer),
                Some(&mut bytes_read),
                None,
            )
            .map_err(|err| {
                RebeccaError::PlatformUnavailable(format!(
                    "{EXPERIMENTAL_NTFS_MFT_BACKEND_LABEL} {SEQUENTIAL_MFT_SOURCE_LABEL} could not read {} at byte {offset}: {}",
                    self.device_path,
                    err.message()
                ))
            })?;
        }

        buffer.truncate(usize::try_from(bytes_read).unwrap_or(0));
        Ok(buffer)
    }
}

impl Drop for LiveNtfsVolume {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

struct LiveNtfsMetadataFile {
    handle: HANDLE,
}

impl Drop for LiveNtfsMetadataFile {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

struct LiveNtfsIndexStreamSource<'a> {
    volume: &'a LiveNtfsVolume,
    cancellation: &'a ScanCancellationToken,
    monitor: &'a NtfsMftBuildMonitor,
}

impl NtfsStreamSource for LiveNtfsIndexStreamSource<'_> {
    type Error = RebeccaError;

    fn read_bytes_at(
        &mut self,
        volume_offset: u64,
        len: usize,
    ) -> std::result::Result<Vec<u8>, Self::Error> {
        check_mft_build_progress(self.cancellation, self.monitor)?;
        self.volume.read_volume_bytes(volume_offset, len)
    }
}

struct SequentialMftDataSource<'a> {
    volume: &'a LiveNtfsVolume,
}

impl MftRecordSource for SequentialMftDataSource<'_> {
    fn label(&self) -> &'static str {
        SEQUENTIAL_MFT_SOURCE_LABEL
    }

    fn read_records(
        &self,
        volume_data: &NTFS_VOLUME_DATA_BUFFER,
        cancellation: &ScanCancellationToken,
        monitor: &NtfsMftBuildMonitor,
    ) -> Result<ParsedNtfsRecords> {
        let geometry = NtfsRecordGeometry::from_volume_data(&self.volume.device_path, volume_data)?;
        let mft_data = monitor.measure_checked(
            NtfsMftBuildStage::SequentialOpenMftData,
            cancellation,
            || self.volume.open_mft_data_stream(),
        )?;
        let extents = monitor.measure_checked(
            NtfsMftBuildStage::SequentialReadRetrievalPointers,
            cancellation,
            || self.volume.mft_extents(&mft_data, cancellation, monitor),
        )?;
        let reader = MftRecordReader::new(geometry.record_size, geometry.sector_size);
        let mut records = Vec::new();
        let mut caveats = Vec::new();
        let mut parse_errors = MftParseErrorCaveats::default();

        let mut context = SequentialMftReadContext {
            geometry,
            reader: &reader,
            cancellation,
            monitor,
            records: &mut records,
            parse_errors: &mut parse_errors,
            parse_chunks: Vec::with_capacity(sequential_mft_parse_window_chunks()),
            parse_window_chunks: sequential_mft_parse_window_chunks(),
        };
        for extent in extents {
            self.read_extent_records(extent, &mut context)?;
        }
        context.flush_parse_chunks()?;
        parse_errors.append_to(&mut caveats);

        Ok(ParsedNtfsRecords {
            source_label: self.label(),
            records,
            caveats,
        })
    }
}

struct SequentialMftReadContext<'a> {
    geometry: NtfsRecordGeometry,
    reader: &'a MftRecordReader,
    cancellation: &'a ScanCancellationToken,
    monitor: &'a NtfsMftBuildMonitor,
    records: &'a mut Vec<NtfsParsedRecord>,
    parse_errors: &'a mut MftParseErrorCaveats,
    parse_chunks: Vec<SequentialMftChunk>,
    parse_window_chunks: usize,
}

impl SequentialMftReadContext<'_> {
    fn push_parse_chunk(&mut self, chunk: SequentialMftChunk) -> Result<()> {
        self.parse_chunks.push(chunk);
        if self.parse_chunks.len() >= self.parse_window_chunks {
            self.flush_parse_chunks()?;
        }
        Ok(())
    }

    fn flush_parse_chunks(&mut self) -> Result<()> {
        if self.parse_chunks.is_empty() {
            return Ok(());
        }

        let chunks = std::mem::replace(
            &mut self.parse_chunks,
            Vec::with_capacity(self.parse_window_chunks),
        );
        let batches = self.monitor.measure_checked(
            NtfsMftBuildStage::SequentialParseRecords,
            self.cancellation,
            || parse_sequential_mft_chunks(self.reader, self.cancellation, &chunks),
        )?;
        for batch in batches {
            self.records.extend(batch.records);
            for err in batch.errors {
                self.parse_errors.record(err.record_id, err.error);
            }
        }

        Ok(())
    }
}

impl SequentialMftDataSource<'_> {
    fn read_extent_records(
        &self,
        extent: MftExtent,
        context: &mut SequentialMftReadContext<'_>,
    ) -> Result<()> {
        let geometry = context.geometry;
        let extent_stream_offset = extent
            .starting_vcn
            .checked_mul(geometry.bytes_per_cluster)
            .ok_or_else(|| {
                RebeccaError::PlatformUnavailable(format!(
                    "{EXPERIMENTAL_NTFS_MFT_BACKEND_LABEL} {SEQUENTIAL_MFT_SOURCE_LABEL} $MFT extent stream offset overflowed"
                ))
            })?;
        if !extent_stream_offset.is_multiple_of(geometry.record_size as u64) {
            return Err(RebeccaError::PlatformUnavailable(format!(
                "{EXPERIMENTAL_NTFS_MFT_BACKEND_LABEL} {SEQUENTIAL_MFT_SOURCE_LABEL} $MFT extent is not file-record aligned"
            )));
        }

        let mut next_record_id = extent_stream_offset / geometry.record_size as u64;
        let mut volume_offset = extent
            .lcn
            .checked_mul(geometry.bytes_per_cluster)
            .ok_or_else(|| {
                RebeccaError::PlatformUnavailable(format!(
                    "{EXPERIMENTAL_NTFS_MFT_BACKEND_LABEL} {SEQUENTIAL_MFT_SOURCE_LABEL} volume offset overflowed"
                ))
            })?;
        let mut bytes_remaining = extent
            .cluster_count
            .checked_mul(geometry.bytes_per_cluster)
            .ok_or_else(|| {
                RebeccaError::PlatformUnavailable(format!(
                    "{EXPERIMENTAL_NTFS_MFT_BACKEND_LABEL} {SEQUENTIAL_MFT_SOURCE_LABEL} extent length overflowed"
                ))
            })?;

        while bytes_remaining > 0 && next_record_id < geometry.max_record_count {
            check_mft_build_progress(context.cancellation, context.monitor)?;
            let records_remaining = geometry.max_record_count.saturating_sub(next_record_id);
            let read_len =
                next_mft_chunk_len(bytes_remaining, records_remaining, geometry.record_size);
            if read_len == 0 {
                break;
            }

            let bytes = context.monitor.measure_checked(
                NtfsMftBuildStage::SequentialReadMftBytes,
                context.cancellation,
                || self.volume.read_volume_bytes(volume_offset, read_len),
            )?;
            if bytes.len() != read_len {
                return Err(RebeccaError::PlatformUnavailable(format!(
                    "{EXPERIMENTAL_NTFS_MFT_BACKEND_LABEL} {SEQUENTIAL_MFT_SOURCE_LABEL} read only {} of {read_len} requested bytes from {}",
                    bytes.len(),
                    self.volume.device_path
                )));
            }

            let read_len = read_len as u64;
            let records_read = read_len / geometry.record_size as u64;
            context.push_parse_chunk(SequentialMftChunk {
                base_record_id: next_record_id,
                bytes,
            })?;
            next_record_id = next_record_id.saturating_add(records_read);
            volume_offset = volume_offset.saturating_add(read_len);
            bytes_remaining = bytes_remaining.saturating_sub(read_len);
        }

        Ok(())
    }
}

struct FsctlRecordMftSource<'a> {
    volume: &'a LiveNtfsVolume,
}

impl MftRecordSource for FsctlRecordMftSource<'_> {
    fn label(&self) -> &'static str {
        FSCTL_RECORD_SOURCE_LABEL
    }

    fn read_records(
        &self,
        volume_data: &NTFS_VOLUME_DATA_BUFFER,
        cancellation: &ScanCancellationToken,
        monitor: &NtfsMftBuildMonitor,
    ) -> Result<ParsedNtfsRecords> {
        let geometry = NtfsRecordGeometry::from_volume_data(&self.volume.device_path, volume_data)?;

        let mut records = Vec::new();
        let mut caveats = Vec::new();
        let mut parse_errors = MftParseErrorCaveats::default();
        let mut requested_record = 0_u64;

        monitor.measure_checked(
            NtfsMftBuildStage::FsctlReadParseRecords,
            cancellation,
            || {
                while requested_record < geometry.max_record_count {
                    if requested_record.is_multiple_of(256) {
                        check_mft_build_progress(cancellation, monitor)?;
                    }

                    match self
                        .volume
                        .read_file_record(requested_record, geometry.record_size)
                    {
                        Ok(Some((record_id, raw_record))) => {
                            let parsed_record_id = low_file_reference_number(record_id);
                            match NtfsParsedRecord::parse_fsctl_file_record(
                                parsed_record_id,
                                &raw_record,
                                geometry.sector_size,
                            ) {
                                Ok(record) => records.push(record),
                                Err(err) => parse_errors.record(parsed_record_id, err),
                            }
                            requested_record =
                                parsed_record_id.max(requested_record).saturating_add(1);
                        }
                        Ok(None) => break,
                        Err(err) if windows_error_matches(&err, ERROR_HANDLE_EOF) => break,
                        Err(err) if windows_error_matches(&err, ERROR_INVALID_PARAMETER) => {
                            requested_record = requested_record.saturating_add(1);
                        }
                        Err(err) => {
                            return Err(RebeccaError::PlatformUnavailable(format!(
                                "{EXPERIMENTAL_NTFS_MFT_BACKEND_LABEL} could not read MFT record {requested_record} from {}: {}",
                                self.volume.device_path,
                                err.message()
                            )));
                        }
                    }
                }
                Ok(())
            },
        )?;
        parse_errors.append_to(&mut caveats);

        Ok(ParsedNtfsRecords {
            source_label: self.label(),
            records,
            caveats,
        })
    }
}

impl LiveNtfsVolume {
    fn read_file_record(
        &self,
        record_number: u64,
        record_size: usize,
    ) -> std::result::Result<Option<(u64, Vec<u8>)>, WindowsError> {
        let mut input = NTFS_FILE_RECORD_INPUT_BUFFER {
            FileReferenceNumber: record_number as i64,
        };
        let output_size = NTFS_FILE_RECORD_OUTPUT_HEADER_BYTES + record_size;
        let mut output = vec![0_u8; output_size];
        let mut bytes_returned = 0_u32;
        unsafe {
            DeviceIoControl(
                self.handle,
                FSCTL_GET_NTFS_FILE_RECORD,
                Some((&mut input as *mut NTFS_FILE_RECORD_INPUT_BUFFER).cast()),
                size_of::<NTFS_FILE_RECORD_INPUT_BUFFER>() as u32,
                Some(output.as_mut_ptr().cast()),
                output.len() as u32,
                Some(&mut bytes_returned),
                None,
            )
        }?;

        if bytes_returned == 0 {
            return Ok(None);
        }

        let output_header = unsafe {
            ptr::read_unaligned(output.as_ptr().cast::<NTFS_FILE_RECORD_OUTPUT_BUFFER>())
        };
        let record_length = usize::try_from(output_header.FileRecordLength).unwrap_or(0);
        if record_length == 0 {
            return Ok(None);
        }

        let record_offset = NTFS_FILE_RECORD_OUTPUT_HEADER_BYTES;
        let available = output.len().saturating_sub(record_offset);
        let record_length = record_length.min(available);
        Ok(Some((
            output_header.FileReferenceNumber as u64,
            output[record_offset..record_offset + record_length].to_vec(),
        )))
    }
}

fn volume_open_error(device_path: &str, err: &WindowsError) -> RebeccaError {
    let reason = if windows_error_matches(err, ERROR_ACCESS_DENIED) {
        "permission denied while opening the volume; run an elevated shell or use a safe fallback backend"
    } else {
        "could not open the live volume read-only"
    };
    RebeccaError::PlatformUnavailable(format!(
        "{EXPERIMENTAL_NTFS_MFT_BACKEND_LABEL} {reason} for {device_path}: {}",
        err.message()
    ))
}

fn low_file_reference_number(file_reference_number: u64) -> u64 {
    file_reference_number & FILE_REFERENCE_LOW_MASK
}

fn file_reference_sequence_number(file_reference_number: u64) -> u16 {
    ((file_reference_number >> 48) & 0xFFFF) as u16
}

fn file_reference_from_number(file_reference_number: u64) -> NtfsFileReference {
    NtfsFileReference::known(
        low_file_reference_number(file_reference_number),
        file_reference_sequence_number(file_reference_number),
    )
}

fn file_reference_number(reference: NtfsFileReference) -> u64 {
    let record_id = reference.record_id & FILE_REFERENCE_LOW_MASK;
    reference.sequence_number.map_or(record_id, |sequence| {
        record_id | (u64::from(sequence) << 48)
    })
}

fn root_metadata(path: &Path) -> Result<std::fs::Metadata> {
    std::fs::symlink_metadata(path).map_err(|err| {
        RebeccaError::ScanFailed(ScanFailure::from_io(
            path,
            ScanFailurePhase::RootMetadata,
            &err,
        ))
    })
}

fn wide_null(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(std::iter::once(0)).collect()
}

fn wide_buffer_to_string(buffer: &[u16]) -> String {
    let len = buffer
        .iter()
        .position(|character| *character == 0)
        .unwrap_or(buffer.len());
    OsString::from_wide(&buffer[..len])
        .to_string_lossy()
        .into_owned()
}

fn windows_error_matches(err: &WindowsError, code: WIN32_ERROR) -> bool {
    err.code() == HRESULT::from_win32(code.0)
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::time::{Duration, UNIX_EPOCH};

    use rebecca_ntfs::{
        AttributeType, FileNameNamespace, MftIndex, MftRecordReader, NtfsAttributeStream,
        NtfsDataRun, NtfsDirectoryIndex, NtfsFileName, NtfsFileReference, NtfsIndexEntry,
        NtfsParsedRecord, NtfsStreamSource, ParseCaveat, PhysicalMetricsAccumulator,
    };

    use super::{
        MAX_MFT_ESTIMATE_CAVEAT_SAMPLES_PER_CODE, MAX_MFT_PARSE_ERROR_CAVEAT_SAMPLES,
        MFT_BUILD_TIMING_CAVEAT_CODE, MFT_CAVEAT_SUMMARY_CODE, MftExtent, MftParseErrorCaveats,
        MftRecordSource, NTFS_FILE_RECORD_OUTPUT_HEADER_BYTES, NTFS_VOLUME_DATA_BUFFER,
        NtfsMftBuildMonitor, NtfsMftBuildStage, NtfsRecordGeometry, ParsedNtfsRecords,
        RETRIEVAL_POINTERS_BUFFER, RETRIEVAL_POINTERS_BUFFER_0, SEQUENTIAL_MFT_CHUNK_BYTES,
        ScanCancellationToken, SequentialMftChunk, TargetedMftRecordResolver, TargetedMftTraversal,
        TargetedMftTraversalLimits, VolumePaths, build_mft_index_from_records,
        check_mft_build_progress, collect_mft_disk_map_entry, file_reference_from_number,
        file_reference_number, low_file_reference_number, next_mft_chunk_len,
        parse_retrieval_pointer_extents, parse_sequential_mft_chunks,
        read_mft_records_from_sources, with_bounded_mft_caveats,
    };
    use crate::disk_map::{
        DiskMapBackendOptions, DiskMapEntryKind, DiskMapGroup, DiskMapGroupKind, DiskMapSortField,
        DiskMapTopEntries,
    };
    use crate::error::{RebeccaError, Result};
    use crate::plan::EstimateProvenance;
    use crate::scan::ScanReport;
    use crate::scan::backend::{MeasuredScan, ScanBackendKind};

    #[test]
    fn volume_paths_support_drive_absolute_paths() {
        let paths = VolumePaths::from_path(std::path::Path::new("C:\\Temp\\Cache")).unwrap();

        assert_eq!(paths.root_path, std::path::PathBuf::from("C:\\"));
        assert_eq!(paths.device_path, "\\\\.\\C:");
        assert_eq!(paths.mft_data_path, "\\\\?\\C:\\$MFT::$DATA");
    }

    #[test]
    fn volume_paths_reject_relative_paths() {
        let err = VolumePaths::from_path(std::path::Path::new("Temp\\Cache")).unwrap_err();

        assert!(err.to_string().contains("absolute local path"));
    }

    #[test]
    fn low_file_reference_masks_sequence_bits() {
        assert_eq!(low_file_reference_number(0x0001_0000_0000_002A), 42);
    }

    #[test]
    fn file_reference_roundtrips_sequence_bits_for_targeted_fsctl() {
        let reference = file_reference_from_number(0x0003_0000_004B_DD21);

        assert_eq!(reference, NtfsFileReference::known(0x4B_DD21, 3));
        assert_eq!(file_reference_number(reference), 0x0003_0000_004B_DD21);
        assert_eq!(
            file_reference_number(NtfsFileReference::unknown_sequence(42)),
            42
        );
    }

    #[test]
    fn ntfs_file_record_output_buffer_uses_wire_header_offset() {
        assert_eq!(NTFS_FILE_RECORD_OUTPUT_HEADER_BYTES, 12);
    }

    #[test]
    fn ntfs_record_geometry_accepts_valid_volume_data() {
        let volume_data = ntfs_volume_data(1024, 512, 4096, 8192);

        let geometry = NtfsRecordGeometry::from_volume_data("\\\\.\\C:", &volume_data).unwrap();

        assert_eq!(geometry.record_size, 1024);
        assert_eq!(geometry.sector_size, 512);
        assert_eq!(geometry.bytes_per_cluster, 4096);
        assert_eq!(geometry.max_record_count, 8);
    }

    #[test]
    fn ntfs_record_geometry_rejects_unaligned_records() {
        let volume_data = ntfs_volume_data(1000, 512, 4096, 8192);

        let err = NtfsRecordGeometry::from_volume_data("\\\\.\\C:", &volume_data).unwrap_err();

        assert!(err.to_string().contains("unaligned"));
    }

    #[test]
    fn retrieval_pointer_parser_maps_ordered_extents() {
        let buffer = retrieval_pointer_buffer(0, &[(4, 10), (9, 20)]);

        let extents = parse_retrieval_pointer_extents(&buffer).unwrap();

        assert_eq!(
            extents,
            vec![
                MftExtent {
                    starting_vcn: 0,
                    lcn: 10,
                    cluster_count: 4,
                },
                MftExtent {
                    starting_vcn: 4,
                    lcn: 20,
                    cluster_count: 5,
                },
            ]
        );
    }

    #[test]
    fn retrieval_pointer_parser_rejects_sparse_extents() {
        let buffer = retrieval_pointer_buffer(0, &[(4, -1)]);

        let err = parse_retrieval_pointer_extents(&buffer).unwrap_err();

        assert!(err.to_string().contains("sparse"));
    }

    #[test]
    fn mft_chunk_len_is_bounded_and_record_aligned() {
        assert_eq!(next_mft_chunk_len(4097, 10, 1024), 4096);
        assert_eq!(
            next_mft_chunk_len(SEQUENTIAL_MFT_CHUNK_BYTES as u64 * 2, 100_000, 1024),
            SEQUENTIAL_MFT_CHUNK_BYTES
        );
        assert_eq!(next_mft_chunk_len(512, 10, 1024), 0);
    }

    #[test]
    fn mft_index_builder_expands_live_index_allocation_streams() {
        let monitor = test_build_monitor();
        let mut source = FakeIndexStreamSource::default().with_bytes(
            0x80_000,
            &index_allocation_record(
                0,
                vec![
                    index_allocation_entry(
                        file_reference(6, 6),
                        file_reference(5, 5),
                        "large.bin",
                        0,
                    ),
                    index_allocation_last_entry(),
                ],
            ),
        );
        let records = ParsedNtfsRecords {
            source_label: "sequential",
            records: vec![
                parsed_directory_with_index_allocation(5, "large-dir"),
                parsed_file(6, 99, "large.bin", 3),
            ],
            caveats: vec![ParseCaveat::new("source-caveat", "source")],
        };

        let (index, caveats) = build_mft_index_from_records(
            records,
            test_record_geometry(),
            &mut source,
            &ScanCancellationToken::new(),
            &monitor,
        )
        .unwrap();
        let summary = index.aggregate_subtree(5);

        assert_eq!(summary.bytes, 3);
        assert!(summary.caveats.iter().any(|caveat| {
            caveat.code == "directory-index-parent-map-fallback"
                && caveat.message.contains("large.bin")
        }));
        assert_eq!(caveats.len(), 1);
        assert_eq!(caveats[0].code, "source-caveat");
    }

    #[test]
    fn mft_index_builder_turns_stream_read_failure_into_bounded_caveat() {
        let monitor = test_build_monitor();
        let mut source = FakeIndexStreamSource::default();
        let records = ParsedNtfsRecords {
            source_label: "sequential",
            records: vec![parsed_directory_with_index_allocation(5, "large-dir")],
            caveats: Vec::new(),
        };

        let (index, caveats) = build_mft_index_from_records(
            records,
            test_record_geometry(),
            &mut source,
            &ScanCancellationToken::new(),
            &monitor,
        )
        .unwrap();
        let summary = index.aggregate_subtree(5);

        assert!(caveats.is_empty());
        assert!(summary.caveats.iter().any(|caveat| {
            caveat.code == "invalid-index-allocation"
                && caveat.message.contains("stream read failed")
        }));
    }

    #[test]
    fn mft_index_builder_preserves_cancellation_during_stream_expansion() {
        let monitor = test_build_monitor();
        let cancellation = ScanCancellationToken::new();
        let mut source = CancellingIndexStreamSource {
            cancellation: cancellation.clone(),
        };
        let records = ParsedNtfsRecords {
            source_label: "sequential",
            records: vec![parsed_directory_with_index_allocation(5, "large-dir")],
            caveats: Vec::new(),
        };

        let err = build_mft_index_from_records(
            records,
            test_record_geometry(),
            &mut source,
            &cancellation,
            &monitor,
        )
        .unwrap_err();

        assert!(matches!(err, RebeccaError::OperationCancelled(_)));
    }

    #[test]
    fn targeted_traversal_expands_index_allocation_without_full_index() {
        let monitor = test_build_monitor();
        let cancellation = ScanCancellationToken::new();
        let mut resolver = FakeTargetedRecordResolver::default()
            .with_record(parsed_directory_with_index_allocation(5, "large-dir"))
            .with_record(parsed_file(6, 99, "large.bin", 3));
        let mut source = FakeIndexStreamSource::default().with_bytes(
            0x80_000,
            &index_allocation_record(
                0,
                vec![
                    index_allocation_entry(
                        file_reference(6, 6),
                        file_reference(5, 5),
                        "large.bin",
                        0,
                    ),
                    index_allocation_last_entry(),
                ],
            ),
        );
        let summary = {
            let mut traversal = TargetedMftTraversal {
                resolver: &mut resolver,
                stream_source: &mut source,
                geometry: test_record_geometry(),
                cancellation: &cancellation,
                monitor: &monitor,
                limits: TargetedMftTraversalLimits::default(),
            };
            traversal
                .aggregate_subtree(NtfsFileReference::known(5, 5))
                .unwrap()
        };

        assert_eq!(summary.bytes, 3);
        assert_eq!(summary.files, 1);
        assert_eq!(summary.directories, 1);
        assert_eq!(resolver.reads, vec![5, 6]);
    }

    #[test]
    fn targeted_traversal_caveats_child_sequence_mismatch() {
        let monitor = test_build_monitor();
        let cancellation = ScanCancellationToken::new();
        let mut resolver = FakeTargetedRecordResolver::default()
            .with_record(parsed_directory_with_index_allocation(5, "large-dir"))
            .with_record(parsed_file(6, 99, "large.bin", 3));
        let mut source = FakeIndexStreamSource::default().with_bytes(
            0x80_000,
            &index_allocation_record(
                0,
                vec![
                    index_allocation_entry(
                        file_reference(6, 99),
                        file_reference(5, 5),
                        "large.bin",
                        0,
                    ),
                    index_allocation_last_entry(),
                ],
            ),
        );
        let mut traversal = TargetedMftTraversal {
            resolver: &mut resolver,
            stream_source: &mut source,
            geometry: test_record_geometry(),
            cancellation: &cancellation,
            monitor: &monitor,
            limits: TargetedMftTraversalLimits::default(),
        };

        let summary = traversal
            .aggregate_subtree(NtfsFileReference::known(5, 5))
            .unwrap();

        assert_eq!(summary.bytes, 0);
        assert_eq!(summary.files, 0);
        assert!(summary.caveats.iter().any(|caveat| {
            caveat.code == "directory-index-child-sequence-mismatch"
                && caveat.message.contains("record 6")
        }));
    }

    #[test]
    fn targeted_traversal_stops_at_record_budget() {
        let monitor = test_build_monitor();
        let cancellation = ScanCancellationToken::new();
        let mut resolver = FakeTargetedRecordResolver::default()
            .with_record(parsed_directory_with_index_allocation(5, "large-dir"))
            .with_record(parsed_file(6, 99, "large.bin", 3));
        let mut source = FakeIndexStreamSource::default().with_bytes(
            0x80_000,
            &index_allocation_record(
                0,
                vec![
                    index_allocation_entry(
                        file_reference(6, 6),
                        file_reference(5, 5),
                        "large.bin",
                        0,
                    ),
                    index_allocation_last_entry(),
                ],
            ),
        );
        let mut traversal = TargetedMftTraversal {
            resolver: &mut resolver,
            stream_source: &mut source,
            geometry: test_record_geometry(),
            cancellation: &cancellation,
            monitor: &monitor,
            limits: TargetedMftTraversalLimits {
                max_records: 1,
                max_depth: 512,
            },
        };

        let err = traversal
            .aggregate_subtree(NtfsFileReference::known(5, 5))
            .unwrap_err();

        assert!(matches!(err, RebeccaError::PlatformUnavailable(_)));
        assert!(err.to_string().contains("record candidate budget"));
    }

    #[test]
    fn targeted_traversal_stale_child_does_not_poison_later_valid_child() {
        let monitor = test_build_monitor();
        let cancellation = ScanCancellationToken::new();
        let mut resolver = FakeTargetedRecordResolver::default()
            .with_record(parsed_directory_with_index_allocation(5, "large-dir"))
            .with_record(parsed_file(6, 5, "large.bin", 3));
        let mut source = FakeIndexStreamSource::default().with_bytes(
            0x80_000,
            &index_allocation_record(
                0,
                vec![
                    index_allocation_entry(
                        file_reference(6, 6),
                        file_reference(5, 5),
                        "large.bin",
                        0,
                    ),
                    index_allocation_entry(
                        file_reference(6, 99),
                        file_reference(5, 5),
                        "large.bin",
                        0,
                    ),
                    index_allocation_last_entry(),
                ],
            ),
        );
        let mut traversal = TargetedMftTraversal {
            resolver: &mut resolver,
            stream_source: &mut source,
            geometry: test_record_geometry(),
            cancellation: &cancellation,
            monitor: &monitor,
            limits: TargetedMftTraversalLimits::default(),
        };

        let summary = traversal
            .aggregate_subtree(NtfsFileReference::known(5, 5))
            .unwrap();

        assert_eq!(summary.bytes, 3);
        assert_eq!(summary.files, 1);
        assert!(summary.caveats.iter().any(|caveat| {
            caveat.code == "directory-index-child-sequence-mismatch"
                && caveat.message.contains("record 6")
        }));
    }

    #[test]
    fn targeted_traversal_caveats_i30_parent_map_fallback() {
        let monitor = test_build_monitor();
        let cancellation = ScanCancellationToken::new();
        let mut resolver = FakeTargetedRecordResolver::default()
            .with_record(parsed_directory_with_index_allocation(5, "large-dir"))
            .with_record(parsed_file(6, 99, "large.bin", 3));
        let mut source = FakeIndexStreamSource::default().with_bytes(
            0x80_000,
            &index_allocation_record(
                0,
                vec![
                    index_allocation_entry(
                        file_reference(6, 6),
                        file_reference(5, 5),
                        "large.bin",
                        0,
                    ),
                    index_allocation_last_entry(),
                ],
            ),
        );
        let mut traversal = TargetedMftTraversal {
            resolver: &mut resolver,
            stream_source: &mut source,
            geometry: test_record_geometry(),
            cancellation: &cancellation,
            monitor: &monitor,
            limits: TargetedMftTraversalLimits::default(),
        };

        let summary = traversal
            .aggregate_subtree(NtfsFileReference::known(5, 5))
            .unwrap();

        assert_eq!(summary.bytes, 3);
        assert!(summary.caveats.iter().any(|caveat| {
            caveat.code == "directory-index-parent-map-fallback"
                && caveat.message.contains("large.bin")
        }));
    }

    #[test]
    fn targeted_traversal_skips_dos_i30_aliases() {
        let monitor = test_build_monitor();
        let cancellation = ScanCancellationToken::new();
        let mut resolver = FakeTargetedRecordResolver::default()
            .with_record(parsed_directory_with_index_allocation(5, "root"))
            .with_record(parsed_file(6, 5, "long-name.bin", 10));
        let mut source = FakeIndexStreamSource::default().with_bytes(
            0x80_000,
            &index_allocation_record(
                0,
                vec![
                    index_allocation_entry(
                        file_reference(6, 6),
                        file_reference(5, 5),
                        "long-name.bin",
                        0,
                    ),
                    index_allocation_entry_with_namespace(
                        file_reference(6, 6),
                        file_reference(5, 5),
                        "LONG-N~1.BIN",
                        0,
                        2,
                    ),
                    index_allocation_last_entry(),
                ],
            ),
        );
        let mut traversal = TargetedMftTraversal {
            resolver: &mut resolver,
            stream_source: &mut source,
            geometry: test_record_geometry(),
            cancellation: &cancellation,
            monitor: &monitor,
            limits: TargetedMftTraversalLimits::default(),
        };

        let summary = traversal
            .aggregate_subtree(NtfsFileReference::known(5, 5))
            .unwrap();

        assert_eq!(summary.bytes, 10);
        assert_eq!(summary.files, 1);
        assert!(!summary.caveats.iter().any(|caveat| {
            caveat.code == "directory-index-parent-map-fallback"
                && caveat.message.contains("LONG-N~1.BIN")
        }));
    }

    #[test]
    fn targeted_disk_map_collects_ranked_entries_without_full_index() {
        let monitor = test_build_monitor();
        let cancellation = ScanCancellationToken::new();
        let mut resolver = FakeTargetedRecordResolver::default()
            .with_record(parsed_directory_with_index_allocation(5, "root"))
            .with_record(parsed_file(6, 5, "large.bin", 10))
            .with_record(parsed_file(7, 5, "small.bin", 3));
        let mut source = FakeIndexStreamSource::default().with_bytes(
            0x80_000,
            &index_allocation_record(
                0,
                vec![
                    index_allocation_entry(
                        file_reference(7, 7),
                        file_reference(5, 5),
                        "small.bin",
                        0,
                    ),
                    index_allocation_entry(
                        file_reference(6, 6),
                        file_reference(5, 5),
                        "large.bin",
                        0,
                    ),
                    index_allocation_last_entry(),
                ],
            ),
        );
        let mut traversal = TargetedMftTraversal {
            resolver: &mut resolver,
            stream_source: &mut source,
            geometry: test_record_geometry(),
            cancellation: &cancellation,
            monitor: &monitor,
            limits: TargetedMftTraversalLimits::default(),
        };

        let map = traversal
            .collect_disk_map(
                NtfsFileReference::known(5, 5),
                std::path::Path::new("C:\\root"),
                &disk_map_options(2, None, Vec::new()),
                100,
                &EstimateProvenance::default(),
            )
            .unwrap();

        assert_eq!(map.metrics.logical_bytes, 13);
        assert_eq!(map.metrics.allocated_bytes, Some(13));
        assert_eq!(map.metrics.unique_logical_bytes, Some(13));
        assert_eq!(map.metrics.unique_allocated_bytes, Some(13));
        assert_eq!(map.metrics.files, 2);
        assert_eq!(map.metrics.directories, 0);
        assert_eq!(map.top_entries.len(), 2);
        assert_eq!(
            map.top_entries[0].path,
            std::path::PathBuf::from("C:\\root\\large.bin")
        );
        assert_eq!(map.top_entries[0].kind, DiskMapEntryKind::File);
        assert_eq!(map.top_entries[0].depth, 1);
        assert_eq!(
            map.top_entries[1].path,
            std::path::PathBuf::from("C:\\root\\small.bin")
        );
        assert!(map.caveats.is_empty());
    }

    #[test]
    fn full_index_disk_map_preserves_duplicate_paths_with_unique_metrics() {
        let mut dir_a = parsed_directory_with_index_allocation(6, "a");
        dir_a.names = vec![parsed_file_name(5, "a", FILE_ATTRIBUTE_DIRECTORY)];
        let mut dir_b = parsed_directory_with_index_allocation(7, "b");
        dir_b.names = vec![parsed_file_name(5, "b", FILE_ATTRIBUTE_DIRECTORY)];
        let mut file = parsed_file(8, 6, "left.bin", 10);
        file.names.push(parsed_file_name(7, "right.bin", 0));
        let index = MftIndex::from_parsed_records(vec![
            parsed_directory_with_index_allocation(5, "root"),
            dir_a,
            dir_b,
            file,
        ]);
        let options = disk_map_options(10, None, Vec::new());
        let mut top_entries = DiskMapTopEntries::new(
            options.top_limit,
            options.top_sort,
            options.entry_filter.clone(),
        );
        let mut groups = options.group_collector();
        let mut visited_directories = BTreeSet::new();
        let mut caveats = Vec::new();
        let mut aggregate = PhysicalMetricsAccumulator::default();
        let cancellation = ScanCancellationToken::new();
        let provenance = EstimateProvenance::default();

        for edge in index.child_edges(5).cloned().collect::<Vec<_>>() {
            let child = index.get(edge.child.record_id).unwrap().clone();
            let child_path = std::path::PathBuf::from("C:\\root").join(edge.name);
            let child_aggregate = collect_mft_disk_map_entry(
                &index,
                std::path::Path::new("C:\\root"),
                child_path,
                child,
                1,
                usize::MAX,
                &provenance,
                &mut visited_directories,
                &mut caveats,
                &mut top_entries,
                &mut groups,
                100,
                &cancellation,
            )
            .unwrap();
            aggregate.absorb_child(child_aggregate);
        }

        let metrics = aggregate.into_metrics();
        assert_eq!(metrics.logical_bytes, 20);
        assert_eq!(metrics.allocated_bytes, Some(20));
        assert_eq!(metrics.unique_logical_bytes, 10);
        assert_eq!(metrics.unique_allocated_bytes, Some(10));
        assert_eq!(metrics.files, 2);
        assert_eq!(metrics.directories, 2);
        let paths = top_entries
            .into_sorted_entries()
            .into_iter()
            .map(|entry| entry.path)
            .collect::<BTreeSet<_>>();
        assert!(paths.contains(&std::path::PathBuf::from("C:\\root\\a\\left.bin")));
        assert!(paths.contains(&std::path::PathBuf::from("C:\\root\\b\\right.bin")));
    }

    #[test]
    fn targeted_disk_map_collects_requested_groups_without_full_index() {
        let monitor = test_build_monitor();
        let cancellation = ScanCancellationToken::new();
        let mut resolver = FakeTargetedRecordResolver::default()
            .with_record(parsed_directory_with_index_allocation(5, "root"))
            .with_record(parsed_file(6, 5, "large.bin", 10))
            .with_record(parsed_file(7, 5, "small.txt", 3));
        let mut source = FakeIndexStreamSource::default().with_bytes(
            0x80_000,
            &index_allocation_record(
                0,
                vec![
                    index_allocation_entry(
                        file_reference(6, 6),
                        file_reference(5, 5),
                        "large.bin",
                        0,
                    ),
                    index_allocation_entry(
                        file_reference(7, 7),
                        file_reference(5, 5),
                        "small.txt",
                        0,
                    ),
                    index_allocation_last_entry(),
                ],
            ),
        );
        let mut traversal = TargetedMftTraversal {
            resolver: &mut resolver,
            stream_source: &mut source,
            geometry: test_record_geometry(),
            cancellation: &cancellation,
            monitor: &monitor,
            limits: TargetedMftTraversalLimits::default(),
        };

        let map = traversal
            .collect_disk_map(
                NtfsFileReference::known(5, 5),
                std::path::Path::new("C:\\root"),
                &disk_map_options(
                    0,
                    None,
                    vec![
                        DiskMapGroupKind::Extension,
                        DiskMapGroupKind::Depth,
                        DiskMapGroupKind::Age,
                    ],
                ),
                100,
                &EstimateProvenance::default(),
            )
            .unwrap();
        let groups = map.groups.finish();

        assert_group_metrics(&groups, DiskMapGroupKind::Extension, ".bin", 10, 1);
        assert_group_metrics(&groups, DiskMapGroupKind::Extension, ".txt", 3, 1);
        assert_group_metrics(&groups, DiskMapGroupKind::Depth, "depth-1", 13, 2);
        assert_group_metrics(&groups, DiskMapGroupKind::Age, "modified-unknown", 13, 2);
    }

    #[test]
    fn targeted_disk_map_max_depth_limits_entries_not_totals() {
        let monitor = test_build_monitor();
        let cancellation = ScanCancellationToken::new();
        let mut resolver = FakeTargetedRecordResolver::default()
            .with_record(parsed_directory_with_index_allocation(5, "root"))
            .with_record(parsed_file(6, 5, "large.bin", 10));
        let mut source = FakeIndexStreamSource::default().with_bytes(
            0x80_000,
            &index_allocation_record(
                0,
                vec![
                    index_allocation_entry(
                        file_reference(6, 6),
                        file_reference(5, 5),
                        "large.bin",
                        0,
                    ),
                    index_allocation_last_entry(),
                ],
            ),
        );
        let mut traversal = TargetedMftTraversal {
            resolver: &mut resolver,
            stream_source: &mut source,
            geometry: test_record_geometry(),
            cancellation: &cancellation,
            monitor: &monitor,
            limits: TargetedMftTraversalLimits::default(),
        };

        let map = traversal
            .collect_disk_map(
                NtfsFileReference::known(5, 5),
                std::path::Path::new("C:\\root"),
                &disk_map_options(10, Some(0), Vec::new()),
                100,
                &EstimateProvenance::default(),
            )
            .unwrap();

        assert_eq!(map.metrics.logical_bytes, 10);
        assert_eq!(map.metrics.files, 1);
        assert!(map.top_entries.is_empty());
    }

    #[test]
    fn targeted_disk_map_preserves_duplicate_paths_with_unique_metrics() {
        let monitor = test_build_monitor();
        let cancellation = ScanCancellationToken::new();
        let mut resolver = FakeTargetedRecordResolver::default()
            .with_record(parsed_directory_with_index_allocation(5, "root"))
            .with_record(parsed_file(6, 5, "large.bin", 10));
        let mut source = FakeIndexStreamSource::default().with_bytes(
            0x80_000,
            &index_allocation_record(
                0,
                vec![
                    index_allocation_entry(
                        file_reference(6, 6),
                        file_reference(5, 5),
                        "large.bin",
                        0,
                    ),
                    index_allocation_entry(
                        file_reference(6, 6),
                        file_reference(5, 5),
                        "alias.bin",
                        0,
                    ),
                    index_allocation_last_entry(),
                ],
            ),
        );
        let mut traversal = TargetedMftTraversal {
            resolver: &mut resolver,
            stream_source: &mut source,
            geometry: test_record_geometry(),
            cancellation: &cancellation,
            monitor: &monitor,
            limits: TargetedMftTraversalLimits::default(),
        };

        let map = traversal
            .collect_disk_map(
                NtfsFileReference::known(5, 5),
                std::path::Path::new("C:\\root"),
                &disk_map_options(10, None, Vec::new()),
                100,
                &EstimateProvenance::default(),
            )
            .unwrap();

        assert_eq!(map.metrics.logical_bytes, 20);
        assert_eq!(map.metrics.allocated_bytes, Some(20));
        assert_eq!(map.metrics.unique_logical_bytes, Some(10));
        assert_eq!(map.metrics.unique_allocated_bytes, Some(10));
        assert_eq!(map.metrics.files, 2);
        assert_eq!(map.top_entries.len(), 2);
        let paths = map
            .top_entries
            .iter()
            .map(|entry| entry.path.clone())
            .collect::<BTreeSet<_>>();
        assert!(paths.contains(&std::path::PathBuf::from("C:\\root\\large.bin")));
        assert!(paths.contains(&std::path::PathBuf::from("C:\\root\\alias.bin")));
        assert!(
            !map.caveats
                .iter()
                .any(|caveat| caveat.code == "mft-targeted-record-already-counted")
        );
    }

    #[test]
    fn targeted_disk_map_skips_dos_i30_aliases() {
        let monitor = test_build_monitor();
        let cancellation = ScanCancellationToken::new();
        let mut resolver = FakeTargetedRecordResolver::default()
            .with_record(parsed_directory_with_index_allocation(5, "root"))
            .with_record(parsed_file(6, 5, "long-name.bin", 10));
        let mut source = FakeIndexStreamSource::default().with_bytes(
            0x80_000,
            &index_allocation_record(
                0,
                vec![
                    index_allocation_entry(
                        file_reference(6, 6),
                        file_reference(5, 5),
                        "long-name.bin",
                        0,
                    ),
                    index_allocation_entry_with_namespace(
                        file_reference(6, 6),
                        file_reference(5, 5),
                        "LONG-N~1.BIN",
                        0,
                        2,
                    ),
                    index_allocation_last_entry(),
                ],
            ),
        );
        let mut traversal = TargetedMftTraversal {
            resolver: &mut resolver,
            stream_source: &mut source,
            geometry: test_record_geometry(),
            cancellation: &cancellation,
            monitor: &monitor,
            limits: TargetedMftTraversalLimits::default(),
        };

        let map = traversal
            .collect_disk_map(
                NtfsFileReference::known(5, 5),
                std::path::Path::new("C:\\root"),
                &disk_map_options(10, None, Vec::new()),
                100,
                &EstimateProvenance::default(),
            )
            .unwrap();

        assert_eq!(map.metrics.logical_bytes, 10);
        assert_eq!(map.metrics.unique_logical_bytes, Some(10));
        assert_eq!(map.metrics.files, 1);
        assert_eq!(map.top_entries.len(), 1);
        assert_eq!(
            map.top_entries[0].path,
            std::path::PathBuf::from("C:\\root\\long-name.bin")
        );
        assert!(!map.caveats.iter().any(|caveat| {
            caveat.code == "directory-index-parent-map-fallback"
                && caveat.message.contains("LONG-N~1.BIN")
        }));
    }

    #[test]
    fn targeted_disk_map_stale_child_does_not_poison_later_valid_child() {
        let monitor = test_build_monitor();
        let cancellation = ScanCancellationToken::new();
        let mut resolver = FakeTargetedRecordResolver::default()
            .with_record(parsed_directory_with_index_allocation(5, "root"))
            .with_record(parsed_file(6, 5, "large.bin", 10));
        let mut source = FakeIndexStreamSource::default().with_bytes(
            0x80_000,
            &index_allocation_record(
                0,
                vec![
                    index_allocation_entry(
                        file_reference(6, 6),
                        file_reference(5, 5),
                        "large.bin",
                        0,
                    ),
                    index_allocation_entry(
                        file_reference(6, 99),
                        file_reference(5, 5),
                        "large.bin",
                        0,
                    ),
                    index_allocation_last_entry(),
                ],
            ),
        );
        let mut traversal = TargetedMftTraversal {
            resolver: &mut resolver,
            stream_source: &mut source,
            geometry: test_record_geometry(),
            cancellation: &cancellation,
            monitor: &monitor,
            limits: TargetedMftTraversalLimits::default(),
        };

        let map = traversal
            .collect_disk_map(
                NtfsFileReference::known(5, 5),
                std::path::Path::new("C:\\root"),
                &disk_map_options(10, None, Vec::new()),
                100,
                &EstimateProvenance::default(),
            )
            .unwrap();

        assert_eq!(map.metrics.logical_bytes, 10);
        assert_eq!(map.metrics.files, 1);
        assert_eq!(map.top_entries.len(), 1);
        assert_eq!(
            map.top_entries[0].path,
            std::path::PathBuf::from("C:\\root\\large.bin")
        );
        assert!(map.caveats.iter().any(|caveat| {
            caveat.code == "directory-index-child-sequence-mismatch"
                && caveat.message.contains("record 6")
        }));
    }

    #[test]
    fn targeted_disk_map_caveats_i30_parent_map_fallback() {
        let monitor = test_build_monitor();
        let cancellation = ScanCancellationToken::new();
        let mut resolver = FakeTargetedRecordResolver::default()
            .with_record(parsed_directory_with_index_allocation(5, "root"))
            .with_record(parsed_file(6, 99, "large.bin", 10));
        let mut source = FakeIndexStreamSource::default().with_bytes(
            0x80_000,
            &index_allocation_record(
                0,
                vec![
                    index_allocation_entry(
                        file_reference(6, 6),
                        file_reference(5, 5),
                        "large.bin",
                        0,
                    ),
                    index_allocation_last_entry(),
                ],
            ),
        );
        let mut traversal = TargetedMftTraversal {
            resolver: &mut resolver,
            stream_source: &mut source,
            geometry: test_record_geometry(),
            cancellation: &cancellation,
            monitor: &monitor,
            limits: TargetedMftTraversalLimits::default(),
        };

        let map = traversal
            .collect_disk_map(
                NtfsFileReference::known(5, 5),
                std::path::Path::new("C:\\root"),
                &disk_map_options(10, None, Vec::new()),
                100,
                &EstimateProvenance::default(),
            )
            .unwrap();

        assert_eq!(map.metrics.logical_bytes, 10);
        assert_eq!(map.metrics.files, 1);
        assert!(map.caveats.iter().any(|caveat| {
            caveat.code == "directory-index-parent-map-fallback"
                && caveat.message.contains("large.bin")
        }));
    }

    #[test]
    fn mft_build_budget_timeout_is_fallback_capable_platform_error() {
        let monitor = NtfsMftBuildMonitor::expired_for_test(Duration::from_secs(1));

        let err = check_mft_build_progress(&ScanCancellationToken::new(), &monitor).unwrap_err();

        assert!(matches!(err, RebeccaError::PlatformUnavailable(_)));
        let message = err.to_string();
        assert!(message.contains("timed out after 1s"));
        assert!(message.contains("REBECCA_NTFS_MFT_INDEX_TIMEOUT_SECONDS"));
    }

    #[test]
    fn mft_build_timeout_reports_active_stage_and_completed_timings() {
        let monitor = NtfsMftBuildMonitor::expired_for_test(Duration::from_secs(1));
        monitor
            .measure(NtfsMftBuildStage::OpenVolume, || Ok(()))
            .unwrap();

        let err = monitor
            .measure(NtfsMftBuildStage::SequentialParseRecords, || {
                check_mft_build_progress(&ScanCancellationToken::new(), &monitor)
            })
            .unwrap_err();

        let message = err.to_string();
        assert!(message.contains("while sequential-parse-records"));
        assert!(message.contains("completed_timings=open-volume="));
    }

    #[test]
    fn mft_build_timing_caveat_is_opt_in() {
        let monitor = NtfsMftBuildMonitor::new(None, true);

        monitor
            .measure(NtfsMftBuildStage::SequentialReadMftBytes, || Ok(()))
            .unwrap();
        monitor
            .measure(NtfsMftBuildStage::SequentialParseRecords, || Ok(()))
            .unwrap();
        monitor
            .measure(NtfsMftBuildStage::BuildMftIndex, || Ok(()))
            .unwrap();
        let caveat = monitor.timing_caveat().unwrap();

        assert_eq!(caveat.code, MFT_BUILD_TIMING_CAVEAT_CODE);
        assert!(caveat.message.contains("sequential-read-mft-bytes="));
        assert!(caveat.message.contains("sequential-parse-records="));
        assert!(caveat.message.contains("build-mft-index="));
    }

    #[test]
    fn sequential_mft_parallel_parse_preserves_chunk_order_and_base_ids() {
        let reader = MftRecordReader::new(4, 1);
        let chunks = vec![
            SequentialMftChunk {
                base_record_id: 20,
                bytes: vec![0; 8],
            },
            SequentialMftChunk {
                base_record_id: 10,
                bytes: vec![0; 4],
            },
        ];

        let batches =
            parse_sequential_mft_chunks(&reader, &ScanCancellationToken::new(), &chunks).unwrap();
        let error_record_ids: Vec<_> = batches
            .into_iter()
            .flat_map(|batch| batch.errors)
            .map(|error| error.record_id)
            .collect();

        assert_eq!(error_record_ids, vec![20, 21, 10]);
    }

    #[test]
    fn sequential_mft_parallel_parse_preserves_cancellation() {
        let reader = MftRecordReader::new(4, 1);
        let cancellation = ScanCancellationToken::new();
        cancellation.cancel();

        let err = parse_sequential_mft_chunks(
            &reader,
            &cancellation,
            &[SequentialMftChunk {
                base_record_id: 0,
                bytes: vec![0; 4],
            }],
        )
        .unwrap_err();

        assert!(matches!(err, RebeccaError::OperationCancelled(_)));
    }

    #[test]
    fn record_source_strategy_returns_first_success() {
        let monitor = test_build_monitor();
        let source = FakeRecordSource {
            label: "primary",
            behavior: FakeRecordSourceBehavior::Success("primary-success"),
        };

        let records = read_mft_records_from_sources(
            &[&source],
            &NTFS_VOLUME_DATA_BUFFER::default(),
            &ScanCancellationToken::new(),
            &monitor,
        )
        .unwrap();

        assert_eq!(records.source_label, "primary");
        assert_eq!(records.caveats.len(), 1);
        assert_eq!(records.caveats[0].code, "primary-success");
    }

    #[test]
    fn record_source_strategy_tries_next_fallback_capable_source() {
        let monitor = test_build_monitor();
        let unavailable = FakeRecordSource {
            label: "sequential",
            behavior: FakeRecordSourceBehavior::PlatformUnavailable,
        };
        let fallback = FakeRecordSource {
            label: "fsctl-record",
            behavior: FakeRecordSourceBehavior::Success("fallback-success"),
        };

        let records = read_mft_records_from_sources(
            &[&unavailable, &fallback],
            &NTFS_VOLUME_DATA_BUFFER::default(),
            &ScanCancellationToken::new(),
            &monitor,
        )
        .unwrap();

        assert_eq!(records.source_label, "fsctl-record");
        assert!(
            records
                .caveats
                .iter()
                .any(|caveat| caveat.code == "fallback-success")
        );
        assert!(records.caveats.iter().any(|caveat| {
            caveat.code == "mft-record-source-fallback" && caveat.message.contains("sequential")
        }));
    }

    #[test]
    fn record_source_strategy_preserves_cancelled_error() {
        let monitor = test_build_monitor();
        let cancelled = FakeRecordSource {
            label: "sequential",
            behavior: FakeRecordSourceBehavior::Cancelled,
        };
        let fallback = FakeRecordSource {
            label: "fsctl-record",
            behavior: FakeRecordSourceBehavior::Success("fallback-success"),
        };

        let err = read_mft_records_from_sources(
            &[&cancelled, &fallback],
            &NTFS_VOLUME_DATA_BUFFER::default(),
            &ScanCancellationToken::new(),
            &monitor,
        )
        .unwrap_err();

        assert!(matches!(err, RebeccaError::OperationCancelled(_)));
    }

    #[test]
    fn record_source_strategy_stops_when_build_budget_expires() {
        let monitor = NtfsMftBuildMonitor::expired_for_test(Duration::from_secs(1));
        let source = FakeRecordSource {
            label: "primary",
            behavior: FakeRecordSourceBehavior::Success("should-not-run"),
        };

        let err = read_mft_records_from_sources(
            &[&source],
            &NTFS_VOLUME_DATA_BUFFER::default(),
            &ScanCancellationToken::new(),
            &monitor,
        )
        .unwrap_err();

        assert!(matches!(err, RebeccaError::PlatformUnavailable(_)));
        assert!(err.to_string().contains("timed out"));
    }

    #[test]
    fn parse_error_caveats_are_sampled_with_summary() {
        let mut parse_errors = MftParseErrorCaveats::default();
        for record_id in 0..MAX_MFT_PARSE_ERROR_CAVEAT_SAMPLES + 3 {
            parse_errors.record(record_id as u64, "invalid signature");
        }

        let mut caveats = Vec::new();
        parse_errors.append_to(&mut caveats);

        assert_eq!(caveats.len(), MAX_MFT_PARSE_ERROR_CAVEAT_SAMPLES + 1);
        assert_eq!(caveats[0].code, "mft-record-parse-error");
        assert!(caveats[0].message.contains("record 0"));
        let summary = caveats.last().unwrap();
        assert_eq!(summary.code, "mft-record-parse-error-summary");
        assert!(summary.message.contains("3 additional"));
    }

    #[test]
    fn estimate_caveats_are_bounded_per_code() {
        let measured = MeasuredScan::exact(
            ScanReport {
                bytes_scanned: 0,
                files_scanned: 0,
                directories_scanned: 0,
            },
            ScanBackendKind::WindowsNtfsMftExperimental,
        );
        let mut caveats: Vec<_> = (0..MAX_MFT_ESTIMATE_CAVEAT_SAMPLES_PER_CODE + 2)
            .map(|index| {
                ParseCaveat::new(
                    "multiple-file-names",
                    format!("record {index} has multiple names"),
                )
            })
            .collect();
        caveats.push(ParseCaveat::new(
            "attribute-list-present",
            "record uses an attribute list",
        ));

        let measured = with_bounded_mft_caveats(measured, caveats);

        assert_eq!(
            measured
                .caveats
                .iter()
                .filter(|caveat| caveat.code == "multiple-file-names")
                .count(),
            MAX_MFT_ESTIMATE_CAVEAT_SAMPLES_PER_CODE
        );
        assert!(measured.caveats.iter().any(|caveat| {
            caveat.code == MFT_CAVEAT_SUMMARY_CODE
                && caveat
                    .message
                    .contains("2 additional 'multiple-file-names'")
        }));
        assert!(measured.caveats.iter().any(|caveat| {
            caveat.code == "attribute-list-present"
                && caveat.message == "record uses an attribute list"
        }));
    }

    struct FakeRecordSource {
        label: &'static str,
        behavior: FakeRecordSourceBehavior,
    }

    enum FakeRecordSourceBehavior {
        Success(&'static str),
        PlatformUnavailable,
        Cancelled,
    }

    impl MftRecordSource for FakeRecordSource {
        fn label(&self) -> &'static str {
            self.label
        }

        fn read_records(
            &self,
            _volume_data: &NTFS_VOLUME_DATA_BUFFER,
            _cancellation: &ScanCancellationToken,
            _monitor: &NtfsMftBuildMonitor,
        ) -> Result<ParsedNtfsRecords> {
            match self.behavior {
                FakeRecordSourceBehavior::Success(code) => Ok(ParsedNtfsRecords {
                    source_label: self.label,
                    records: Vec::new(),
                    caveats: vec![ParseCaveat::new(code, self.label)],
                }),
                FakeRecordSourceBehavior::PlatformUnavailable => Err(
                    RebeccaError::PlatformUnavailable("not available".to_string()),
                ),
                FakeRecordSourceBehavior::Cancelled => {
                    Err(RebeccaError::OperationCancelled("cancelled".to_string()))
                }
            }
        }
    }

    #[derive(Default)]
    struct FakeIndexStreamSource {
        bytes: BTreeMap<u64, u8>,
    }

    impl FakeIndexStreamSource {
        fn with_bytes(mut self, offset: u64, bytes: &[u8]) -> Self {
            for (index, byte) in bytes.iter().copied().enumerate() {
                self.bytes.insert(offset + index as u64, byte);
            }
            self
        }
    }

    impl NtfsStreamSource for FakeIndexStreamSource {
        type Error = &'static str;

        fn read_bytes_at(
            &mut self,
            volume_offset: u64,
            len: usize,
        ) -> std::result::Result<Vec<u8>, Self::Error> {
            let mut bytes = Vec::new();
            for index in 0..len {
                let Some(byte) = self.bytes.get(&(volume_offset + index as u64)) else {
                    break;
                };
                bytes.push(*byte);
            }
            Ok(bytes)
        }
    }

    #[derive(Default)]
    struct FakeTargetedRecordResolver {
        records: BTreeMap<u64, NtfsParsedRecord>,
        reads: Vec<u64>,
    }

    impl FakeTargetedRecordResolver {
        fn with_record(mut self, record: NtfsParsedRecord) -> Self {
            self.records.insert(record.reference.record_id, record);
            self
        }
    }

    impl TargetedMftRecordResolver for FakeTargetedRecordResolver {
        fn resolve_record(
            &mut self,
            reference: NtfsFileReference,
        ) -> Result<Option<NtfsParsedRecord>> {
            self.reads.push(reference.record_id);
            Ok(self.records.get(&reference.record_id).cloned())
        }
    }

    struct CancellingIndexStreamSource {
        cancellation: ScanCancellationToken,
    }

    impl NtfsStreamSource for CancellingIndexStreamSource {
        type Error = &'static str;

        fn read_bytes_at(
            &mut self,
            _volume_offset: u64,
            _len: usize,
        ) -> std::result::Result<Vec<u8>, Self::Error> {
            self.cancellation.cancel();
            Err("cancelled")
        }
    }

    fn test_record_geometry() -> NtfsRecordGeometry {
        NtfsRecordGeometry {
            record_size: 1024,
            sector_size: 512,
            bytes_per_cluster: 4096,
            max_record_count: 16,
        }
    }

    fn test_build_monitor() -> NtfsMftBuildMonitor {
        NtfsMftBuildMonitor::new(None, false)
    }

    fn disk_map_options(
        top_limit: usize,
        max_depth: Option<usize>,
        group_kinds: Vec<DiskMapGroupKind>,
    ) -> DiskMapBackendOptions {
        DiskMapBackendOptions {
            top_limit,
            top_sort: DiskMapSortField::Logical,
            entry_filter: Default::default(),
            max_depth,
            group_kinds,
            group_limit: 20,
            group_now: UNIX_EPOCH,
            group_sort: DiskMapSortField::Logical,
        }
    }

    fn assert_group_metrics(
        groups: &[DiskMapGroup],
        kind: DiskMapGroupKind,
        key: &str,
        logical_bytes: u64,
        files: u64,
    ) {
        let group = groups
            .iter()
            .find(|group| group.kind == kind && group.key == key)
            .unwrap_or_else(|| panic!("missing group {}:{key}", kind.label()));
        assert_eq!(group.metrics.logical_bytes, logical_bytes);
        assert_eq!(group.metrics.files, files);
    }

    fn parsed_directory_with_index_allocation(record_id: u64, name: &str) -> NtfsParsedRecord {
        NtfsParsedRecord {
            reference: NtfsFileReference::known(record_id, record_id as u16),
            base_reference: None,
            in_use: true,
            is_directory: true,
            is_reparse_point: false,
            attributes: Vec::new(),
            attribute_list_entries: Vec::new(),
            names: vec![parsed_file_name(record_id, name, FILE_ATTRIBUTE_DIRECTORY)],
            attribute_streams: vec![NtfsAttributeStream {
                attribute_type: AttributeType::IndexAllocation,
                attribute_id: 0,
                name: Some("$I30".to_string()),
                non_resident: true,
                flags: 0,
                lowest_vcn: Some(0),
                highest_vcn: Some(0),
                logical_size: RECORD_SIZE as u64,
                allocated_size: Some(RECORD_SIZE as u64),
                initialized_size: Some(RECORD_SIZE as u64),
                data_runs: vec![NtfsDataRun {
                    starting_vcn: 0,
                    cluster_count: 1,
                    lcn: Some(0x80),
                }],
            }],
            directory_indexes: vec![NtfsDirectoryIndex {
                name: "$I30".to_string(),
                attribute_id: 0,
                indexed_attribute: AttributeType::FileName,
                index_record_size: RECORD_SIZE as u32,
                root_entries: vec![NtfsIndexEntry {
                    directory_entry: None,
                    child_vcn: Some(0),
                    is_last: true,
                }],
            }],
            directory_entries: Vec::new(),
            caveats: Vec::new(),
        }
    }

    fn parsed_file(record_id: u64, parent_id: u64, name: &str, bytes: u64) -> NtfsParsedRecord {
        NtfsParsedRecord {
            reference: NtfsFileReference::known(record_id, record_id as u16),
            base_reference: None,
            in_use: true,
            is_directory: false,
            is_reparse_point: false,
            attributes: Vec::new(),
            attribute_list_entries: Vec::new(),
            names: vec![parsed_file_name(parent_id, name, 0)],
            attribute_streams: vec![NtfsAttributeStream {
                attribute_type: AttributeType::Data,
                attribute_id: 0,
                name: None,
                non_resident: false,
                flags: 0,
                lowest_vcn: None,
                highest_vcn: None,
                logical_size: bytes,
                allocated_size: Some(bytes),
                initialized_size: Some(bytes),
                data_runs: Vec::new(),
            }],
            directory_indexes: Vec::new(),
            directory_entries: Vec::new(),
            caveats: Vec::new(),
        }
    }

    fn parsed_file_name(parent_id: u64, name: &str, file_attributes: u32) -> NtfsFileName {
        NtfsFileName {
            parent: NtfsFileReference::known(parent_id, parent_id as u16),
            namespace: FileNameNamespace::Win32,
            name: name.to_string(),
            attribute_id: Some(0),
            attribute_name: None,
            lowest_vcn: None,
            modified_windows_filetime: 0,
            allocated_size: 0,
            real_size: 0,
            file_attributes,
        }
    }

    fn ntfs_volume_data(
        record_size: u32,
        sector_size: u32,
        bytes_per_cluster: u32,
        mft_valid_data_length: i64,
    ) -> NTFS_VOLUME_DATA_BUFFER {
        NTFS_VOLUME_DATA_BUFFER {
            BytesPerFileRecordSegment: record_size,
            BytesPerSector: sector_size,
            BytesPerCluster: bytes_per_cluster,
            MftValidDataLength: mft_valid_data_length,
            ..Default::default()
        }
    }

    const RECORD_SIZE: usize = 1024;
    const SECTOR_SIZE: usize = 512;
    const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x0000_0010;

    fn index_allocation_record(vcn: u64, entries: Vec<Vec<u8>>) -> Vec<u8> {
        let mut raw_entries = Vec::new();
        for entry in entries {
            raw_entries.extend_from_slice(&entry);
        }

        let mut record = vec![0_u8; RECORD_SIZE];
        let usa_offset = 0x28;
        let index_header_offset = 0x18;
        let entries_offset = 0x20;
        let entries_start = index_header_offset + entries_offset;
        let index_size = entries_offset + raw_entries.len();

        record[0..4].copy_from_slice(b"INDX");
        put_u16(&mut record, 4, usa_offset as u16);
        put_u16(&mut record, 6, 3);
        put_u64(&mut record, 16, vcn);
        put_u32(&mut record, index_header_offset, entries_offset as u32);
        put_u32(&mut record, index_header_offset + 4, index_size as u32);
        put_u32(&mut record, index_header_offset + 8, index_size as u32);
        record[entries_start..entries_start + raw_entries.len()].copy_from_slice(&raw_entries);
        apply_test_fixup_at(&mut record, usa_offset);
        record
    }

    fn index_allocation_entry(
        child_reference: u64,
        parent_reference: u64,
        name: &str,
        file_attributes: u32,
    ) -> Vec<u8> {
        index_allocation_entry_with_namespace(
            child_reference,
            parent_reference,
            name,
            file_attributes,
            1,
        )
    }

    fn index_allocation_entry_with_namespace(
        child_reference: u64,
        parent_reference: u64,
        name: &str,
        file_attributes: u32,
        namespace: u8,
    ) -> Vec<u8> {
        let file_name =
            file_name_value_with_namespace(parent_reference, name, file_attributes, namespace);
        let entry_len = align8(16 + file_name.len());
        let mut entry = vec![0_u8; entry_len];
        put_u64(&mut entry, 0, child_reference);
        put_u16(&mut entry, 8, entry_len as u16);
        put_u16(&mut entry, 10, file_name.len() as u16);
        entry[16..16 + file_name.len()].copy_from_slice(&file_name);
        entry
    }

    fn index_allocation_last_entry() -> Vec<u8> {
        let mut entry = vec![0_u8; 16];
        put_u16(&mut entry, 8, 16);
        put_u16(&mut entry, 12, 0x0002);
        entry
    }

    fn file_name_value_with_namespace(
        parent_reference: u64,
        name: &str,
        file_attributes: u32,
        namespace: u8,
    ) -> Vec<u8> {
        let name_utf16 = name.encode_utf16().collect::<Vec<_>>();
        let mut value = vec![0_u8; 66 + (name_utf16.len() * 2)];
        put_u64(&mut value, 0, parent_reference);
        put_u32(&mut value, 56, file_attributes);
        value[64] = name_utf16.len() as u8;
        value[65] = namespace;
        for (index, character) in name_utf16.iter().enumerate() {
            put_u16(&mut value, 66 + (index * 2), *character);
        }
        value
    }

    fn file_reference(record_id: u64, sequence_number: u16) -> u64 {
        ((sequence_number as u64) << 48) | (record_id & 0x0000_FFFF_FFFF_FFFF)
    }

    fn apply_test_fixup_at(record: &mut [u8], usa_offset: usize) {
        let update_sequence = 0xBBAA_u16;
        let sector_count = record.len() / SECTOR_SIZE;
        put_u16(record, usa_offset, update_sequence);
        for sector_index in 0..sector_count {
            let tail = ((sector_index + 1) * SECTOR_SIZE) - 2;
            let original = u16::from_le_bytes([record[tail], record[tail + 1]]);
            put_u16(record, usa_offset + ((sector_index + 1) * 2), original);
            put_u16(record, tail, update_sequence);
        }
    }

    fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
        bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn put_u64(bytes: &mut [u8], offset: usize, value: u64) {
        bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn align8(value: usize) -> usize {
        (value + 7) & !7
    }

    fn retrieval_pointer_buffer(starting_vcn: i64, extents: &[(i64, i64)]) -> Vec<u8> {
        let header_size = std::mem::offset_of!(RETRIEVAL_POINTERS_BUFFER, Extents);
        let extent_size = std::mem::size_of::<RETRIEVAL_POINTERS_BUFFER_0>();
        let mut buffer = vec![0_u8; header_size + (extent_size * extents.len())];
        unsafe {
            std::ptr::write_unaligned(
                buffer.as_mut_ptr().cast::<RETRIEVAL_POINTERS_BUFFER>(),
                RETRIEVAL_POINTERS_BUFFER {
                    ExtentCount: extents.len() as u32,
                    StartingVcn: starting_vcn,
                    Extents: [RETRIEVAL_POINTERS_BUFFER_0::default()],
                },
            );
            for (index, (next_vcn, lcn)) in extents.iter().copied().enumerate() {
                std::ptr::write_unaligned(
                    buffer
                        .as_mut_ptr()
                        .add(header_size + (index * extent_size))
                        .cast::<RETRIEVAL_POINTERS_BUFFER_0>(),
                    RETRIEVAL_POINTERS_BUFFER_0 {
                        NextVcn: next_vcn,
                        Lcn: lcn,
                    },
                );
            }
        }
        buffer
    }
}

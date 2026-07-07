mod backend;
mod portable;
mod progress;
#[cfg(windows)]
pub(crate) mod windows_native;
#[cfg(all(windows, feature = "ntfs"))]
mod windows_ntfs_mft;

use std::path::{Path, PathBuf};
#[cfg(all(windows, feature = "ntfs"))]
use std::sync::Arc;
use std::sync::OnceLock;

use rayon::ThreadPool;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

pub use backend::{
    MeasuredScan, ScanBackend, ScanBackendEvidence, ScanBackendKind, ScanCacheEvidenceEvent,
    ScanEstimateCaveat, ScanEstimateConfidence, ScanMetricSemantics, ScanRequest,
};
pub use portable::PortableRecursiveScanBackend;
pub use progress::{ScanCancellationToken, ScanProgressEvent};
#[cfg(windows)]
pub use windows_native::WindowsNativeDirectoryScanBackend;

#[cfg(all(windows, feature = "ntfs"))]
use crate::disk_map::{DiskMapBackendOptions, DiskMapBackendRoot};
use crate::error::Result;
use crate::model::DeleteMode;
use crate::parallelism::{bounded_parallelism_budget, run_scoped_parallel_work};
use crate::plan::{CleanupTarget, CleanupTargetIssueReason};
#[cfg(all(windows, feature = "ntfs"))]
use crate::progress::{InspectProgressEvent, InspectProgressResult};
use crate::safety::{PATH_DOES_NOT_EXIST_REASON, PathDisposition, assess_existing_path};

static SCAN_THREAD_POOL: OnceLock<ThreadPool> = OnceLock::new();
#[cfg(all(debug_assertions, windows, feature = "ntfs"))]
const TEST_DISABLE_LIVE_NTFS_MFT_ENV: &str = "REBECCA_TEST_DISABLE_LIVE_NTFS_MFT";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanReport {
    pub bytes_scanned: u64,
    pub files_scanned: u64,
    pub directories_scanned: u64,
}

impl ScanReport {
    pub(crate) fn record_file(&mut self, bytes: u64) {
        self.files_scanned = self.files_scanned.saturating_add(1);
        self.bytes_scanned = self.bytes_scanned.saturating_add(bytes);
    }

    pub(crate) fn record_directory(&mut self) {
        self.directories_scanned = self.directories_scanned.saturating_add(1);
    }
}

#[derive(Debug, Clone)]
pub struct ScanEngine {
    #[cfg(all(windows, feature = "ntfs"))]
    context: Arc<ScanEngineContext>,
}

#[cfg(all(windows, feature = "ntfs"))]
#[derive(Debug, Default)]
struct ScanEngineContext {
    ntfs_mft_cache: windows_ntfs_mft::WindowsNtfsMftIndexCache,
}

impl Default for ScanEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl ScanEngine {
    pub fn new() -> Self {
        Self {
            #[cfg(all(windows, feature = "ntfs"))]
            context: Arc::new(ScanEngineContext::default()),
        }
    }

    #[cfg(all(windows, feature = "ntfs"))]
    pub fn with_ntfs_mft_manifest_cache_root(cache_root: impl Into<PathBuf>) -> Self {
        let manifest_store = windows_ntfs_mft::NtfsVolumeIndexManifestStore::new(cache_root.into());
        Self {
            context: Arc::new(ScanEngineContext {
                ntfs_mft_cache: windows_ntfs_mft::WindowsNtfsMftIndexCache::with_manifest_store(
                    manifest_store,
                ),
            }),
        }
    }

    pub fn measure_path(&self, path: &Path) -> Result<ScanReport> {
        self.measure_scan(path).map(|measured| measured.report)
    }

    pub fn measure_scan(&self, path: &Path) -> Result<MeasuredScan> {
        self.measure_scan_with_progress(path, &ScanCancellationToken::new(), |_| {})
    }

    #[cfg(all(windows, feature = "ntfs"))]
    pub(crate) fn inspect_windows_ntfs_mft_disk_map_with_progress<F>(
        &self,
        path: &Path,
        options: DiskMapBackendOptions,
        cancellation: &ScanCancellationToken,
        progress: &mut F,
    ) -> Result<DiskMapBackendRoot>
    where
        F: for<'event> FnMut(InspectProgressEvent<'event>) -> InspectProgressResult,
    {
        inspect_windows_ntfs_mft_disk_map_with_progress(self, path, options, cancellation, progress)
    }

    pub fn measure_path_with_progress<F>(
        &self,
        path: &Path,
        cancellation: &ScanCancellationToken,
        progress: F,
    ) -> Result<ScanReport>
    where
        F: for<'a> FnMut(ScanProgressEvent<'a>),
    {
        self.measure_scan_with_progress(path, cancellation, progress)
            .map(|measured| measured.report)
    }

    pub fn measure_scan_with_progress<F>(
        &self,
        path: &Path,
        cancellation: &ScanCancellationToken,
        progress: F,
    ) -> Result<MeasuredScan>
    where
        F: for<'a> FnMut(ScanProgressEvent<'a>),
    {
        self.measure_scan_with_backend(
            path,
            cancellation,
            ScanBackendKind::PortableRecursive,
            progress,
        )
    }

    pub fn measure_scan_with_backend<F>(
        &self,
        path: &Path,
        cancellation: &ScanCancellationToken,
        backend: ScanBackendKind,
        progress: F,
    ) -> Result<MeasuredScan>
    where
        F: for<'a> FnMut(ScanProgressEvent<'a>),
    {
        let request = ScanRequest::new(path, cancellation);
        match backend {
            ScanBackendKind::PortableRecursive => {
                PortableRecursiveScanBackend.measure_path_with_progress(request, progress)
            }
            ScanBackendKind::WindowsNative => {
                self.measure_windows_native_with_portable_fallback(request, progress)
            }
            ScanBackendKind::WindowsNtfsMftExperimental => {
                self.measure_windows_ntfs_mft_with_fallback(request, progress)
            }
        }
    }

    fn measure_windows_native_with_portable_fallback<F>(
        &self,
        request: ScanRequest<'_>,
        progress: F,
    ) -> Result<MeasuredScan>
    where
        F: for<'a> FnMut(ScanProgressEvent<'a>),
    {
        let mut progress = progress;

        match measure_windows_native(request, &mut progress) {
            Ok(measured) => Ok(measured),
            Err(err) if scan_error_can_fallback(&err) => PortableRecursiveScanBackend
                .measure_path_with_progress(request, progress)
                .map(|measured| measured.with_fallback_reason(format!("windows-native: {err}"))),
            Err(err) => Err(err),
        }
    }

    fn measure_windows_ntfs_mft_with_fallback<F>(
        &self,
        request: ScanRequest<'_>,
        progress: F,
    ) -> Result<MeasuredScan>
    where
        F: for<'a> FnMut(ScanProgressEvent<'a>),
    {
        let mut progress = progress;

        match measure_windows_ntfs_mft(self, request, &mut progress) {
            Ok(measured) => Ok(measured),
            Err(err) if scan_error_can_fallback(&err) => {
                self.measure_windows_native_with_portable_fallback(request, progress)
                    .map(|measured| {
                        measured
                            .with_fallback_reason(format!(
                                "windows-ntfs-mft-experimental: {err}"
                            ))
                            .with_caveat(
                                "experimental-ntfs-mft-fallback",
                                "experimental NTFS/MFT indexing was unavailable; Rebecca used a safe directory scanner instead",
                            )
                    })
            }
            Err(err) => Err(err),
        }
    }

    pub fn measure_target(&self, target: ScanTargetRequest) -> CleanupTarget {
        match assess_existing_path(&target.path) {
            PathDisposition::Allowed => match self.measure_path(&target.path) {
                Ok(report) => CleanupTarget::allowed(
                    target.rule_id,
                    target.path,
                    report.bytes_scanned,
                    target.mode,
                ),
                Err(err) => CleanupTarget::failed_with_reason_code(
                    target.rule_id,
                    target.path,
                    target.mode,
                    0,
                    CleanupTargetIssueReason::ScanFailed,
                    err.to_string(),
                ),
            },
            PathDisposition::Missing => CleanupTarget::skipped_with_reason_code(
                target.rule_id,
                target.path,
                target.mode,
                CleanupTargetIssueReason::SafetyPolicySkipped,
                PATH_DOES_NOT_EXIST_REASON,
            ),
            PathDisposition::Skipped(reason) => CleanupTarget::skipped_with_reason_code(
                target.rule_id,
                target.path,
                target.mode,
                CleanupTargetIssueReason::SafetyPolicySkipped,
                reason,
            ),
            PathDisposition::Blocked(reason) => CleanupTarget::blocked_with_reason_code(
                target.rule_id,
                target.path,
                target.mode,
                CleanupTargetIssueReason::SafetyPolicyBlocked,
                reason,
            ),
        }
    }

    pub fn measure_targets(&self, targets: Vec<ScanTargetRequest>) -> Vec<CleanupTarget> {
        let scanner = self;
        let mut scanned: Vec<_> = run_scoped_scan(|| {
            targets
                .into_par_iter()
                .map(|target| scanner.measure_target(target))
                .collect()
        });

        scanned.sort_by(|left, right| {
            left.rule_id
                .cmp(&right.rule_id)
                .then_with(|| left.path.cmp(&right.path))
        });

        scanned
    }
}

#[derive(Debug, Clone)]
pub struct ScanTargetRequest {
    rule_id: String,
    path: PathBuf,
    mode: DeleteMode,
}

impl ScanTargetRequest {
    pub fn new(rule_id: impl Into<String>, path: PathBuf, mode: DeleteMode) -> Self {
        Self {
            rule_id: rule_id.into(),
            path,
            mode,
        }
    }
}

fn scan_error_can_fallback(err: &crate::error::RebeccaError) -> bool {
    matches!(
        err,
        crate::error::RebeccaError::PlatformUnavailable(_)
            | crate::error::RebeccaError::ScanFailed(_)
    )
}

#[cfg(windows)]
fn measure_windows_native<F>(request: ScanRequest<'_>, progress: F) -> Result<MeasuredScan>
where
    F: for<'a> FnMut(ScanProgressEvent<'a>),
{
    WindowsNativeDirectoryScanBackend.measure_path_with_progress(request, progress)
}

#[cfg(not(windows))]
fn measure_windows_native<F>(_request: ScanRequest<'_>, _progress: F) -> Result<MeasuredScan>
where
    F: for<'a> FnMut(ScanProgressEvent<'a>),
{
    Err(crate::error::RebeccaError::PlatformUnavailable(format!(
        "{} scan backend is only available on Windows",
        ScanBackendKind::WindowsNative.label()
    )))
}

#[cfg(all(windows, feature = "ntfs"))]
fn measure_windows_ntfs_mft<F>(
    engine: &ScanEngine,
    request: ScanRequest<'_>,
    progress: F,
) -> Result<MeasuredScan>
where
    F: for<'a> FnMut(ScanProgressEvent<'a>),
{
    #[cfg(debug_assertions)]
    if std::env::var_os(TEST_DISABLE_LIVE_NTFS_MFT_ENV)
        .is_some_and(|value| value != std::ffi::OsStr::new("0"))
    {
        return Err(crate::error::RebeccaError::PlatformUnavailable(format!(
            "windows-ntfs-mft-experimental live volume indexing was disabled by {TEST_DISABLE_LIVE_NTFS_MFT_ENV}"
        )));
    }

    windows_ntfs_mft::WindowsNtfsMftScanBackend::new(&engine.context.ntfs_mft_cache)
        .measure_path_with_progress(request, progress)
}

#[cfg(all(windows, feature = "ntfs"))]
fn inspect_windows_ntfs_mft_disk_map_with_progress<F>(
    engine: &ScanEngine,
    path: &Path,
    options: DiskMapBackendOptions,
    cancellation: &ScanCancellationToken,
    progress: &mut F,
) -> Result<DiskMapBackendRoot>
where
    F: for<'event> FnMut(InspectProgressEvent<'event>) -> InspectProgressResult,
{
    #[cfg(debug_assertions)]
    if std::env::var_os(TEST_DISABLE_LIVE_NTFS_MFT_ENV)
        .is_some_and(|value| value != std::ffi::OsStr::new("0"))
    {
        return Err(crate::error::RebeccaError::PlatformUnavailable(format!(
            "windows-ntfs-mft-experimental live volume indexing was disabled by {TEST_DISABLE_LIVE_NTFS_MFT_ENV}"
        )));
    }

    windows_ntfs_mft::inspect_disk_map_with_progress(
        &engine.context.ntfs_mft_cache,
        path,
        options,
        cancellation,
        progress,
    )
}

#[cfg(not(all(windows, feature = "ntfs")))]
fn measure_windows_ntfs_mft<F>(
    _engine: &ScanEngine,
    _request: ScanRequest<'_>,
    _progress: F,
) -> Result<MeasuredScan>
where
    F: for<'a> FnMut(ScanProgressEvent<'a>),
{
    Err(windows_ntfs_mft_unavailable_error("scan backend"))
}

#[cfg(not(all(windows, feature = "ntfs")))]
pub(crate) fn windows_ntfs_mft_unavailable_error(capability: &str) -> crate::error::RebeccaError {
    let reason = if cfg!(all(windows, not(feature = "ntfs"))) {
        "requires the rebecca-core ntfs feature; the ntfs feature is disabled in this build"
    } else if cfg!(windows) {
        "requires a live NTFS volume index provider; live volume indexing is not enabled in this build"
    } else if cfg!(feature = "ntfs") {
        "requires Windows; live volume indexing is not available on this platform"
    } else {
        "requires Windows and the rebecca-core ntfs feature; the ntfs feature is disabled in this build"
    };
    crate::error::RebeccaError::PlatformUnavailable(format!(
        "windows-ntfs-mft-experimental {capability} {reason}"
    ))
}

pub fn scan_parallelism_budget() -> usize {
    bounded_parallelism_budget()
}

pub(crate) fn run_scoped_scan<R, F>(work: F) -> R
where
    F: FnOnce() -> R + Send,
    R: Send,
{
    run_scoped_parallel_work(&SCAN_THREAD_POOL, "scan", work)
}

#[cfg(test)]
mod tests {
    #[cfg(all(windows, feature = "ntfs"))]
    use super::{ScanBackendKind, ScanCancellationToken, ScanEngine};
    use super::{run_scoped_scan, scan_parallelism_budget};
    use crate::executor::cleanup_parallelism_budget;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn scan_parallelism_budget_stays_bounded() {
        let budget = scan_parallelism_budget();

        assert!((2..=8).contains(&budget));
    }

    #[test]
    fn run_scoped_scan_executes_work() {
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_ref = Arc::clone(&counter);

        run_scoped_scan(move || {
            counter_ref.fetch_add(1, Ordering::SeqCst);
        });

        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn scan_and_cleanup_parallelism_budgets_match() {
        assert_eq!(scan_parallelism_budget(), cleanup_parallelism_budget());
    }

    #[cfg(all(windows, feature = "ntfs"))]
    #[test]
    fn scan_engine_ntfs_manifest_constructor_keeps_portable_scans_available() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("cache");
        let file = temp.path().join("file.bin");
        std::fs::write(&file, b"abc").unwrap();
        let engine = ScanEngine::with_ntfs_mft_manifest_cache_root(root);

        let measured = engine
            .measure_scan_with_backend(
                &file,
                &ScanCancellationToken::new(),
                ScanBackendKind::PortableRecursive,
                |_| {},
            )
            .unwrap();

        assert_eq!(measured.report.bytes_scanned, 3);
        assert_eq!(measured.backend, ScanBackendKind::PortableRecursive);
    }
}

use std::path::Path;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use ignore::WalkBuilder;
use rayon::prelude::*;

use crate::TargetStatus;
use crate::error::{RebeccaError, Result, ScanFailure, ScanFailurePhase};
use crate::plan::CleanupTarget;
use crate::safety::{PathDisposition, assess_existing_path};

#[derive(Debug, Clone, Default)]
pub struct ScanCancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl ScanCancellationToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ScanProgressEvent<'a> {
    FileMeasured {
        path: &'a Path,
        file_size: u64,
        files_scanned: u64,
        bytes_scanned: u64,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScanReport {
    pub bytes_scanned: u64,
    pub files_scanned: u64,
    pub directories_scanned: u64,
}

impl ScanReport {
    fn record_file(&mut self, bytes: u64) {
        self.files_scanned = self.files_scanned.saturating_add(1);
        self.bytes_scanned = self.bytes_scanned.saturating_add(bytes);
    }

    fn record_directory(&mut self) {
        self.directories_scanned = self.directories_scanned.saturating_add(1);
    }
}

pub fn measure_path_size(path: &Path) -> Result<u64> {
    measure_path(path).map(|report| report.bytes_scanned)
}

pub fn measure_path_size_with_progress<F>(
    path: &Path,
    cancellation: &ScanCancellationToken,
    progress: F,
) -> Result<u64>
where
    F: for<'a> FnMut(ScanProgressEvent<'a>),
{
    measure_path_with_progress(path, cancellation, progress).map(|report| report.bytes_scanned)
}

pub fn measure_path(path: &Path) -> Result<ScanReport> {
    measure_path_with_progress(path, &ScanCancellationToken::new(), |_| {})
}

pub fn measure_path_with_progress<F>(
    path: &Path,
    cancellation: &ScanCancellationToken,
    progress: F,
) -> Result<ScanReport>
where
    F: for<'a> FnMut(ScanProgressEvent<'a>),
{
    check_not_cancelled(cancellation)?;
    let metadata = std::fs::symlink_metadata(path).map_err(|err| {
        RebeccaError::ScanFailed(ScanFailure::from_io(
            path,
            ScanFailurePhase::RootMetadata,
            &err,
        ))
    })?;

    if metadata.file_type().is_symlink() {
        return Err(RebeccaError::SafetyBlocked(
            "symlink traversal is disabled".to_string(),
        ));
    }

    if metadata.is_file() {
        let file_size = metadata.len();
        let mut progress = progress;
        let mut report = ScanReport::default();
        report.record_file(file_size);
        progress(ScanProgressEvent::FileMeasured {
            path,
            file_size,
            files_scanned: report.files_scanned,
            bytes_scanned: report.bytes_scanned,
        });
        return Ok(report);
    }

    if metadata.is_dir() {
        return measure_directory_with_progress(path, cancellation, progress);
    }

    Ok(ScanReport::default())
}

pub fn measure_directory_size(path: &Path) -> Result<u64> {
    measure_directory(path).map(|report| report.bytes_scanned)
}

pub fn measure_directory_size_with_progress<F>(
    path: &Path,
    cancellation: &ScanCancellationToken,
    progress: F,
) -> Result<u64>
where
    F: for<'a> FnMut(ScanProgressEvent<'a>),
{
    measure_directory_with_progress(path, cancellation, progress).map(|report| report.bytes_scanned)
}

pub fn measure_directory(path: &Path) -> Result<ScanReport> {
    measure_directory_with_progress(path, &ScanCancellationToken::new(), |_| {})
}

pub fn measure_directory_with_progress<F>(
    path: &Path,
    cancellation: &ScanCancellationToken,
    mut progress: F,
) -> Result<ScanReport>
where
    F: for<'a> FnMut(ScanProgressEvent<'a>),
{
    let mut report = ScanReport::default();
    let walker = WalkBuilder::new(path)
        .hidden(false)
        .follow_links(false)
        .build();

    for entry in walker {
        check_not_cancelled(cancellation)?;
        let entry = entry
            .map_err(|err| RebeccaError::ScanFailed(ScanFailure::directory_walk(path, &err)))?;
        let metadata = entry.metadata().map_err(|err| {
            RebeccaError::ScanFailed(ScanFailure::from_ignore(
                entry.path(),
                ScanFailurePhase::EntryMetadata,
                &err,
            ))
        })?;

        if metadata.file_type().is_symlink() {
            continue;
        }

        if metadata.is_dir() {
            report.record_directory();
        }

        if metadata.is_file() {
            let file_size = metadata.len();
            report.record_file(file_size);
            progress(ScanProgressEvent::FileMeasured {
                path: entry.path(),
                file_size,
                files_scanned: report.files_scanned,
                bytes_scanned: report.bytes_scanned,
            });
        }
    }

    Ok(report)
}

fn check_not_cancelled(cancellation: &ScanCancellationToken) -> Result<()> {
    if cancellation.is_cancelled() {
        return Err(RebeccaError::OperationCancelled(
            "scan was cancelled".to_string(),
        ));
    }

    Ok(())
}

pub fn scan_target(
    rule_id: impl Into<String>,
    path: std::path::PathBuf,
    mode: crate::DeleteMode,
) -> CleanupTarget {
    let rule_id = rule_id.into();

    match assess_existing_path(&path) {
        PathDisposition::Allowed => match measure_path_size(&path) {
            Ok(size) => CleanupTarget::allowed(rule_id, path, size, mode),
            Err(err) => CleanupTarget::failed(rule_id, path, mode, 0, err.to_string()),
        },
        PathDisposition::Skipped(reason) => CleanupTarget::skipped(rule_id, path, mode, reason),
        PathDisposition::Blocked(reason) => CleanupTarget::blocked(rule_id, path, mode, reason),
    }
}

pub fn scan_targets(
    targets: Vec<(String, std::path::PathBuf, crate::DeleteMode)>,
) -> Vec<CleanupTarget> {
    let mut scanned: Vec<_> = targets
        .into_par_iter()
        .map(|(rule_id, path, mode)| scan_target(rule_id, path, mode))
        .collect();

    scanned.sort_by(|left, right| {
        left.rule_id
            .cmp(&right.rule_id)
            .then_with(|| left.path.cmp(&right.path))
    });

    scanned
}

pub fn allowed_target_count(targets: &[CleanupTarget]) -> usize {
    targets
        .iter()
        .filter(|target| matches!(target.status, TargetStatus::Allowed))
        .count()
}

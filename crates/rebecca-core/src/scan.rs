use std::path::{Path, PathBuf};
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicBool, Ordering},
};

use ignore::{DirEntry, WalkBuilder};
use rayon::ThreadPool;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::error::{RebeccaError, Result, ScanFailure, ScanFailurePhase};
use crate::model::DeleteMode;
use crate::parallelism::{bounded_parallelism_budget, run_scoped_parallel_work};
use crate::plan::{CleanupTarget, CleanupTargetIssueReason};
use crate::safety::{
    PATH_DOES_NOT_EXIST_REASON, PathDisposition, assess_existing_path, is_reparse_like,
};

#[derive(Debug, Clone, Default)]
pub struct ScanCancellationToken {
    cancelled: Arc<AtomicBool>,
}

static SCAN_THREAD_POOL: OnceLock<ThreadPool> = OnceLock::new();

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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Default)]
pub struct ScanEngine;

impl ScanEngine {
    pub fn new() -> Self {
        Self
    }

    pub fn measure_path(&self, path: &Path) -> Result<ScanReport> {
        self.measure_path_with_progress(path, &ScanCancellationToken::new(), |_| {})
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
        check_not_cancelled(cancellation)?;
        let metadata = root_metadata(path)?;

        if is_reparse_like(&metadata) {
            return Err(RebeccaError::SafetyBlocked(
                "symlink or reparse point traversal is disabled".to_string(),
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
            return IgnoreWalkerAdapter.measure_directory(path, cancellation, progress);
        }

        Ok(ScanReport::default())
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

#[derive(Debug, Clone, Copy)]
struct IgnoreWalkerAdapter;

impl IgnoreWalkerAdapter {
    fn measure_directory<F>(
        self,
        path: &Path,
        cancellation: &ScanCancellationToken,
        mut progress: F,
    ) -> Result<ScanReport>
    where
        F: for<'a> FnMut(ScanProgressEvent<'a>),
    {
        let mut report = ScanReport::default();
        let walker = cleanup_walk_builder(path).build();

        for entry in walker {
            check_not_cancelled(cancellation)?;
            let entry = entry
                .map_err(|err| RebeccaError::ScanFailed(ScanFailure::directory_walk(path, &err)))?;
            let classification = classify_entry(&entry)?;

            if classification.reparse_like {
                continue;
            }

            if classification.file_type.is_dir() {
                report.record_directory();
            }

            if classification.file_type.is_file() {
                let file_size = classification.size_bytes.unwrap_or(0);
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
}

#[derive(Debug, Clone, Copy)]
struct ScanEntryClassification {
    file_type: std::fs::FileType,
    reparse_like: bool,
    size_bytes: Option<u64>,
}

fn classify_entry(entry: &DirEntry) -> Result<ScanEntryClassification> {
    if let Some(file_type) = entry.file_type() {
        if file_type.is_file() {
            return entry_metadata(entry.path()).map(|metadata| ScanEntryClassification {
                file_type,
                reparse_like: is_reparse_like(&metadata),
                size_bytes: Some(metadata.len()),
            });
        }

        if file_type.is_dir() {
            return entry_metadata(entry.path()).map(|metadata| ScanEntryClassification {
                file_type,
                reparse_like: is_reparse_like(&metadata),
                size_bytes: None,
            });
        }
    }

    let metadata = entry_metadata(entry.path())?;
    Ok(ScanEntryClassification {
        file_type: metadata.file_type(),
        reparse_like: is_reparse_like(&metadata),
        size_bytes: metadata.is_file().then_some(metadata.len()),
    })
}

fn cleanup_walk_builder(path: &Path) -> WalkBuilder {
    let mut builder = WalkBuilder::new(path);
    builder.standard_filters(false).follow_links(false);
    builder
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

fn entry_metadata(path: &Path) -> Result<std::fs::Metadata> {
    std::fs::symlink_metadata(path).map_err(|err| {
        RebeccaError::ScanFailed(ScanFailure::from_io(
            path,
            ScanFailurePhase::EntryMetadata,
            &err,
        ))
    })
}

fn check_not_cancelled(cancellation: &ScanCancellationToken) -> Result<()> {
    if cancellation.is_cancelled() {
        return Err(RebeccaError::OperationCancelled(
            "scan was cancelled".to_string(),
        ));
    }

    Ok(())
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
}

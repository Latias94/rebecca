use std::path::Path;

use ignore::{DirEntry, WalkBuilder};

use crate::error::{RebeccaError, Result, ScanFailure, ScanFailurePhase};
use crate::safety::is_reparse_like;

use super::backend::{MeasuredScan, ScanBackend, ScanBackendKind, ScanRequest};
use super::progress::{ScanProgressEvent, check_not_cancelled};
use super::{ScanCancellationToken, ScanReport};

#[derive(Debug, Clone, Copy, Default)]
pub struct PortableRecursiveScanBackend;

impl ScanBackend for PortableRecursiveScanBackend {
    fn kind(&self) -> ScanBackendKind {
        ScanBackendKind::PortableRecursive
    }

    fn measure_path_with_progress<F>(
        &self,
        request: ScanRequest<'_>,
        progress: F,
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

        let report = if metadata.is_file() {
            measure_file(request.path, &metadata, progress)
        } else if metadata.is_dir() {
            self.measure_directory(request.path, request.cancellation, progress)?
        } else {
            ScanReport::default()
        };

        Ok(MeasuredScan::exact(report, self.kind()))
    }
}

impl PortableRecursiveScanBackend {
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

fn measure_file<F>(path: &Path, metadata: &std::fs::Metadata, progress: F) -> ScanReport
where
    F: for<'a> FnMut(ScanProgressEvent<'a>),
{
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
    report
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

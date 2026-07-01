use std::ffi::{OsStr, OsString};
use std::io;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Component, Path, PathBuf, Prefix};

use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_NO_MORE_FILES, HANDLE, WIN32_ERROR};
use windows::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FindClose, FindFirstFileW,
    FindNextFileW, WIN32_FIND_DATAW,
};
use windows::core::{Error as WindowsError, HRESULT, PCWSTR};

use crate::error::{RebeccaError, Result, ScanFailure, ScanFailurePhase};
use crate::safety::is_reparse_like;

use super::backend::{MeasuredScan, ScanBackend, ScanBackendKind, ScanRequest};
use super::progress::{ScanProgressEvent, check_not_cancelled};
use super::{ScanCancellationToken, ScanReport};

#[derive(Debug, Clone, Copy, Default)]
pub struct WindowsNativeDirectoryScanBackend;

impl ScanBackend for WindowsNativeDirectoryScanBackend {
    fn kind(&self) -> ScanBackendKind {
        ScanBackendKind::WindowsNative
    }

    fn measure_path_with_progress<F>(
        &self,
        request: ScanRequest<'_>,
        progress: F,
    ) -> Result<MeasuredScan>
    where
        F: for<'a> FnMut(ScanProgressEvent<'a>),
    {
        if let Some(reason) = unsupported_path_reason(request.path) {
            return Err(RebeccaError::PlatformUnavailable(reason));
        }

        check_not_cancelled(request.cancellation)?;
        let metadata = root_metadata(request.path)?;

        if is_reparse_like(&metadata) {
            return Err(RebeccaError::SafetyBlocked(
                "symlink or reparse point traversal is disabled".to_string(),
            ));
        }

        let report = if metadata.is_file() {
            measure_file(request.path, metadata.len(), progress)
        } else if metadata.is_dir() {
            measure_directory(request.path, request.cancellation, progress)?
        } else {
            ScanReport::default()
        };

        Ok(MeasuredScan::exact(report, self.kind()))
    }
}

fn measure_directory<F>(
    root: &Path,
    cancellation: &ScanCancellationToken,
    mut progress: F,
) -> Result<ScanReport>
where
    F: for<'a> FnMut(ScanProgressEvent<'a>),
{
    let mut report = ScanReport::default();
    report.record_directory();

    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        check_not_cancelled(cancellation)?;
        enumerate_directory(
            &directory,
            cancellation,
            &mut report,
            &mut stack,
            &mut progress,
        )?;
    }

    Ok(report)
}

fn enumerate_directory<F>(
    directory: &Path,
    cancellation: &ScanCancellationToken,
    report: &mut ScanReport,
    stack: &mut Vec<PathBuf>,
    progress: &mut F,
) -> Result<()>
where
    F: for<'a> FnMut(ScanProgressEvent<'a>),
{
    let mut data = WIN32_FIND_DATAW::default();
    let Some(handle) = find_first_entry(directory, &mut data)? else {
        return Ok(());
    };

    loop {
        check_not_cancelled(cancellation)?;
        record_find_data(directory, &data, report, stack, progress);

        data = WIN32_FIND_DATAW::default();
        match unsafe { FindNextFileW(handle.raw(), &mut data) } {
            Ok(()) => {}
            Err(err) if windows_error_matches(&err, ERROR_NO_MORE_FILES) => return Ok(()),
            Err(err) => {
                return Err(windows_scan_error(
                    directory,
                    ScanFailurePhase::DirectoryWalk,
                    &err,
                ));
            }
        }
    }
}

fn find_first_entry(directory: &Path, data: &mut WIN32_FIND_DATAW) -> Result<Option<FindHandle>> {
    let search_path = directory.join("*");
    let wide_path = wide_null(search_path.as_os_str());

    match unsafe { FindFirstFileW(PCWSTR(wide_path.as_ptr()), data) } {
        Ok(handle) => Ok(Some(FindHandle(handle))),
        Err(err) if windows_error_matches(&err, ERROR_FILE_NOT_FOUND) => Ok(None),
        Err(err) => Err(windows_scan_error(
            directory,
            ScanFailurePhase::DirectoryWalk,
            &err,
        )),
    }
}

fn record_find_data<F>(
    directory: &Path,
    data: &WIN32_FIND_DATAW,
    report: &mut ScanReport,
    stack: &mut Vec<PathBuf>,
    progress: &mut F,
) where
    F: for<'a> FnMut(ScanProgressEvent<'a>),
{
    let Some(file_name) = find_data_file_name(data) else {
        return;
    };

    if file_name == OsStr::new(".") || file_name == OsStr::new("..") || is_reparse_entry(data) {
        return;
    }

    let path = directory.join(&file_name);
    if is_directory_entry(data) {
        report.record_directory();
        stack.push(path);
        return;
    }

    let file_size = find_data_file_size(data);
    report.record_file(file_size);
    progress(ScanProgressEvent::FileMeasured {
        path: &path,
        file_size,
        files_scanned: report.files_scanned,
        bytes_scanned: report.bytes_scanned,
    });
}

fn measure_file<F>(path: &Path, file_size: u64, progress: F) -> ScanReport
where
    F: for<'a> FnMut(ScanProgressEvent<'a>),
{
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

fn root_metadata(path: &Path) -> Result<std::fs::Metadata> {
    std::fs::symlink_metadata(path).map_err(|err| {
        RebeccaError::ScanFailed(ScanFailure::from_io(
            path,
            ScanFailurePhase::RootMetadata,
            &err,
        ))
    })
}

fn unsupported_path_reason(path: &Path) -> Option<String> {
    if !path.is_absolute() {
        return Some("windows-native scan backend requires an absolute local path".to_string());
    }

    if is_unc_path(path) {
        return Some("windows-native scan backend does not scan UNC roots yet".to_string());
    }

    None
}

fn is_unc_path(path: &Path) -> bool {
    matches!(
        path.components().next(),
        Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::UNC(..))
    )
}

fn is_directory_entry(data: &WIN32_FIND_DATAW) -> bool {
    (data.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY.0) != 0
}

fn is_reparse_entry(data: &WIN32_FIND_DATAW) -> bool {
    (data.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT.0) != 0
}

fn find_data_file_size(data: &WIN32_FIND_DATAW) -> u64 {
    (u64::from(data.nFileSizeHigh) << 32) | u64::from(data.nFileSizeLow)
}

fn find_data_file_name(data: &WIN32_FIND_DATAW) -> Option<OsString> {
    let len = data
        .cFileName
        .iter()
        .position(|character| *character == 0)
        .unwrap_or(data.cFileName.len());

    (len > 0).then(|| OsString::from_wide(&data.cFileName[..len]))
}

fn wide_null(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(std::iter::once(0)).collect()
}

fn windows_scan_error(path: &Path, phase: ScanFailurePhase, err: &WindowsError) -> RebeccaError {
    let io_error = windows_error_to_io(err);
    RebeccaError::ScanFailed(ScanFailure::from_io(path, phase, &io_error))
}

fn windows_error_to_io(err: &WindowsError) -> io::Error {
    hresult_win32_code(err.code())
        .map(|code| io::Error::from_raw_os_error(code as i32))
        .unwrap_or_else(|| io::Error::other(err.message().to_string()))
}

fn windows_error_matches(err: &WindowsError, code: WIN32_ERROR) -> bool {
    err.code() == HRESULT::from_win32(code.0)
}

fn hresult_win32_code(hresult: HRESULT) -> Option<u32> {
    let value = hresult.0 as u32;
    if (value & 0xFFFF_0000) == 0x8007_0000 {
        return Some(value & 0x0000_FFFF);
    }

    (value <= 0x0000_FFFF).then_some(value)
}

struct FindHandle(HANDLE);

impl FindHandle {
    fn raw(&self) -> HANDLE {
        self.0
    }
}

impl Drop for FindHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = FindClose(self.0);
        }
    }
}

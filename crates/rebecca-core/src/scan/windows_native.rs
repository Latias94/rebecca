use std::ffi::{OsStr, OsString};
use std::io;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Component, Path, PathBuf, Prefix};

use windows::Win32::Foundation::{
    ERROR_FILE_NOT_FOUND, ERROR_NO_MORE_FILES, ERROR_SUCCESS, GetLastError, HANDLE, SetLastError,
    WIN32_ERROR,
};
use windows::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FindClose, FindFirstFileW,
    FindNextFileW, GetCompressedFileSizeW, INVALID_FILE_SIZE, WIN32_FIND_DATAW,
};
use windows::core::{Error as WindowsError, HRESULT, PCWSTR};

use crate::error::{RebeccaError, Result, ScanFailure, ScanFailurePhase};
use crate::safety::is_reparse_like;

use super::backend::{MeasuredScan, ScanBackend, ScanBackendKind, ScanRequest};
use super::progress::{ScanProgressEvent, check_not_cancelled};
use super::{ScanCancellationToken, ScanReport};

#[derive(Debug, Clone, Copy, Default)]
pub struct WindowsNativeDirectoryScanBackend;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WindowsNativeEntryKind {
    File,
    Directory,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WindowsNativeDirectoryEntry {
    path: PathBuf,
    kind: WindowsNativeEntryKind,
    file_size: u64,
    allocated_size: Option<u64>,
    reparse_like: bool,
}

impl WindowsNativeDirectoryEntry {
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn kind(&self) -> WindowsNativeEntryKind {
        self.kind
    }

    pub(crate) fn file_size(&self) -> u64 {
        self.file_size
    }

    pub(crate) fn allocated_size(&self) -> Option<u64> {
        self.allocated_size
    }

    pub(crate) fn is_reparse_like(&self) -> bool {
        self.reparse_like
    }
}

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
    for_each_directory_entry(directory, cancellation, |entry| {
        record_native_entry(entry, report, stack, progress);
        Ok(())
    })
}

pub(crate) fn read_directory_entries(
    directory: &Path,
    cancellation: &ScanCancellationToken,
) -> Result<Vec<WindowsNativeDirectoryEntry>> {
    let mut entries = Vec::new();
    for_each_directory_entry(directory, cancellation, |entry| {
        entries.push(entry);
        Ok(())
    })?;
    Ok(entries)
}

fn for_each_directory_entry<F>(
    directory: &Path,
    cancellation: &ScanCancellationToken,
    mut visitor: F,
) -> Result<()>
where
    F: FnMut(WindowsNativeDirectoryEntry) -> Result<()>,
{
    let mut data = WIN32_FIND_DATAW::default();
    let Some(handle) = find_first_entry(directory, &mut data)? else {
        return Ok(());
    };

    loop {
        check_not_cancelled(cancellation)?;
        if let Some(entry) = directory_entry_from_find_data(directory, &data) {
            visitor(entry)?;
        }

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

fn record_native_entry<F>(
    entry: WindowsNativeDirectoryEntry,
    report: &mut ScanReport,
    stack: &mut Vec<PathBuf>,
    progress: &mut F,
) where
    F: for<'a> FnMut(ScanProgressEvent<'a>),
{
    if entry.is_reparse_like() {
        return;
    }

    if entry.kind() == WindowsNativeEntryKind::Directory {
        report.record_directory();
        stack.push(entry.path);
        return;
    }

    if entry.kind() != WindowsNativeEntryKind::File {
        return;
    }

    let file_size = entry.file_size();
    let path = entry.path;
    report.record_file(file_size);
    progress(ScanProgressEvent::FileMeasured {
        path: &path,
        file_size,
        files_scanned: report.files_scanned,
        bytes_scanned: report.bytes_scanned,
    });
}

fn directory_entry_from_find_data(
    directory: &Path,
    data: &WIN32_FIND_DATAW,
) -> Option<WindowsNativeDirectoryEntry> {
    let file_name = find_data_file_name(data)?;
    if file_name == OsStr::new(".") || file_name == OsStr::new("..") {
        return None;
    }

    let path = directory.join(file_name);
    let kind = find_data_entry_kind(data);
    let reparse_like = is_reparse_entry(data);
    Some(WindowsNativeDirectoryEntry {
        allocated_size: find_data_allocated_size(&path, kind, reparse_like),
        path,
        kind,
        file_size: find_data_file_size(data),
        reparse_like,
    })
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

pub(crate) fn unsupported_path_reason(path: &Path) -> Option<String> {
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

fn find_data_entry_kind(data: &WIN32_FIND_DATAW) -> WindowsNativeEntryKind {
    if is_directory_entry(data) {
        WindowsNativeEntryKind::Directory
    } else {
        WindowsNativeEntryKind::File
    }
}

fn is_reparse_entry(data: &WIN32_FIND_DATAW) -> bool {
    (data.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT.0) != 0
}

fn find_data_file_size(data: &WIN32_FIND_DATAW) -> u64 {
    (u64::from(data.nFileSizeHigh) << 32) | u64::from(data.nFileSizeLow)
}

fn find_data_allocated_size(
    path: &Path,
    kind: WindowsNativeEntryKind,
    reparse_like: bool,
) -> Option<u64> {
    if kind != WindowsNativeEntryKind::File || reparse_like {
        return None;
    }

    file_allocated_size(path)
}

pub(crate) fn file_allocated_size(path: &Path) -> Option<u64> {
    let wide_path = wide_null(path.as_os_str());
    let mut high = 0_u32;

    unsafe {
        SetLastError(ERROR_SUCCESS);
        let low = GetCompressedFileSizeW(PCWSTR(wide_path.as_ptr()), Some(&mut high));
        let last_error = GetLastError();
        if low == INVALID_FILE_SIZE && last_error != ERROR_SUCCESS {
            return None;
        }

        Some((u64::from(high) << 32) | u64::from(low))
    }
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

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::mem::{offset_of, size_of};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::path::{Component, Path, PathBuf, Prefix};
use std::ptr;
use std::sync::{Arc, Mutex};

use rebecca_ntfs::{MftRecord, MftTree, ParseCaveat};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_ACCESS_DENIED, ERROR_HANDLE_EOF, ERROR_INVALID_PARAMETER, HANDLE,
    WIN32_ERROR,
};
use windows::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAGS_AND_ATTRIBUTES,
    FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_MODE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    GetDriveTypeW, GetFileInformationByHandle, GetVolumeInformationW, OPEN_EXISTING,
};
use windows::Win32::System::IO::DeviceIoControl;
use windows::Win32::System::Ioctl::{
    FSCTL_GET_NTFS_FILE_RECORD, FSCTL_GET_NTFS_VOLUME_DATA, NTFS_FILE_RECORD_INPUT_BUFFER,
    NTFS_FILE_RECORD_OUTPUT_BUFFER, NTFS_VOLUME_DATA_BUFFER,
};
use windows::core::{Error as WindowsError, HRESULT, PCWSTR};

use crate::error::{RebeccaError, Result, ScanFailure, ScanFailurePhase};
use crate::safety::is_reparse_like;

use super::backend::{MeasuredScan, ScanBackend, ScanBackendKind, ScanRequest};
use super::progress::{ScanProgressEvent, check_not_cancelled};
use super::{ScanCancellationToken, ScanReport};

const EXPERIMENTAL_NTFS_MFT_BACKEND_LABEL: &str = "windows-ntfs-mft-experimental";
const NTFS_FILE_SYSTEM_NAME: &str = "NTFS";
const DRIVE_FIXED: u32 = 3;
const FILE_REFERENCE_LOW_MASK: u64 = 0x0000_FFFF_FFFF_FFFF;

#[derive(Debug, Default)]
pub(super) struct WindowsNtfsMftIndexCache {
    volumes: Mutex<BTreeMap<String, Arc<CachedNtfsVolumeIndex>>>,
}

impl WindowsNtfsMftIndexCache {
    fn load_or_build(
        &self,
        capabilities: &NtfsVolumeCapabilities,
        cancellation: &ScanCancellationToken,
    ) -> Result<Arc<CachedNtfsVolumeIndex>> {
        let cache_key = capabilities.cache_key();
        let mut volumes = self.volumes.lock().map_err(|_| {
            RebeccaError::PlatformUnavailable(format!(
                "{EXPERIMENTAL_NTFS_MFT_BACKEND_LABEL} volume index cache is unavailable"
            ))
        })?;
        if let Some(index) = volumes.get(&cache_key) {
            return Ok(Arc::clone(index));
        }

        let index = Arc::new(CachedNtfsVolumeIndex::build(capabilities, cancellation)?);
        volumes.insert(cache_key, Arc::clone(&index));
        Ok(index)
    }
}

#[derive(Debug)]
struct CachedNtfsVolumeIndex {
    tree: MftTree,
    caveats: Vec<ParseCaveat>,
}

impl CachedNtfsVolumeIndex {
    fn build(
        capabilities: &NtfsVolumeCapabilities,
        cancellation: &ScanCancellationToken,
    ) -> Result<Self> {
        let volume = LiveNtfsVolume::open(capabilities)?;
        let volume_data = volume.ntfs_volume_data()?;
        let records = volume.read_mft_records(&volume_data, cancellation)?;
        Ok(Self {
            tree: MftTree::from_records(records.records),
            caveats: records.caveats,
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

        let index = self
            .cache
            .load_or_build(&capabilities, request.cancellation)?;
        let Some(_) = index.tree.get(target_identity.file_reference_number) else {
            return Err(RebeccaError::PlatformUnavailable(format!(
                "{EXPERIMENTAL_NTFS_MFT_BACKEND_LABEL} could not map {} to MFT record {}",
                request.path.display(),
                target_identity.file_reference_number
            )));
        };

        let summary = index
            .tree
            .aggregate_subtree(target_identity.file_reference_number);
        let report = ScanReport {
            bytes_scanned: summary.bytes,
            files_scanned: summary.files,
            directories_scanned: summary.directories,
        };
        let mut measured = MeasuredScan::exact(report, self.kind());
        for caveat in index.caveats.iter().cloned().chain(summary.caveats) {
            measured = measured.with_caveat(caveat.code, caveat.message);
        }

        Ok(measured)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NtfsVolumeCapabilities {
    device_path: String,
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
            device_path: volume_paths.device_path,
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
        })
    }
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
    file_reference_number: u64,
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
            file_reference_number: low_file_reference_number(
                (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
            ),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedMftRecords {
    records: Vec<MftRecord>,
    caveats: Vec<ParseCaveat>,
}

trait MftRecordSource {
    fn label(&self) -> &'static str;

    fn read_records(
        &self,
        volume_data: &NTFS_VOLUME_DATA_BUFFER,
        cancellation: &ScanCancellationToken,
    ) -> Result<ParsedMftRecords>;
}

fn read_mft_records_from_sources(
    sources: &[&dyn MftRecordSource],
    volume_data: &NTFS_VOLUME_DATA_BUFFER,
    cancellation: &ScanCancellationToken,
) -> Result<ParsedMftRecords> {
    let mut fallback_errors = Vec::new();

    for source in sources {
        check_not_cancelled(cancellation)?;
        match source.read_records(volume_data, cancellation) {
            Ok(mut records) => {
                records.caveats.extend(
                    fallback_errors
                        .drain(..)
                        .map(|reason| ParseCaveat::new("mft-record-source-fallback", reason)),
                );
                return Ok(records);
            }
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

struct LiveNtfsVolume {
    handle: HANDLE,
    device_path: String,
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
    ) -> Result<ParsedMftRecords> {
        let fsctl_source = FsctlRecordMftSource { volume: self };
        read_mft_records_from_sources(&[&fsctl_source], volume_data, cancellation)
    }
}

impl Drop for LiveNtfsVolume {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

struct FsctlRecordMftSource<'a> {
    volume: &'a LiveNtfsVolume,
}

impl MftRecordSource for FsctlRecordMftSource<'_> {
    fn label(&self) -> &'static str {
        "fsctl-record"
    }

    fn read_records(
        &self,
        volume_data: &NTFS_VOLUME_DATA_BUFFER,
        cancellation: &ScanCancellationToken,
    ) -> Result<ParsedMftRecords> {
        let record_size = usize::try_from(volume_data.BytesPerFileRecordSegment).unwrap_or(0);
        let sector_size = usize::try_from(volume_data.BytesPerSector).unwrap_or(0);
        if record_size == 0 || sector_size == 0 {
            return Err(RebeccaError::PlatformUnavailable(format!(
                "{EXPERIMENTAL_NTFS_MFT_BACKEND_LABEL} received invalid NTFS record geometry from {}",
                self.volume.device_path
            )));
        }
        if volume_data.MftValidDataLength <= 0 {
            return Err(RebeccaError::PlatformUnavailable(format!(
                "{EXPERIMENTAL_NTFS_MFT_BACKEND_LABEL} received empty NTFS MFT metadata from {}",
                self.volume.device_path
            )));
        }

        let max_records = (volume_data.MftValidDataLength as u64)
            .saturating_div(volume_data.BytesPerFileRecordSegment as u64);
        let mut records = Vec::new();
        let mut caveats = Vec::new();
        let mut requested_record = 0_u64;

        while requested_record < max_records {
            if requested_record.is_multiple_of(256) {
                check_not_cancelled(cancellation)?;
            }

            match self.volume.read_file_record(requested_record, record_size) {
                Ok(Some((record_id, raw_record))) => {
                    let parsed_record_id = low_file_reference_number(record_id);
                    match MftRecord::parse(parsed_record_id, &raw_record, sector_size) {
                        Ok(record) => records.push(record),
                        Err(err) => caveats.push(ParseCaveat::new(
                            "mft-record-parse-error",
                            format!("record {parsed_record_id} could not be parsed: {err}"),
                        )),
                    }
                    requested_record = parsed_record_id.max(requested_record).saturating_add(1);
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

        Ok(ParsedMftRecords { records, caveats })
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
        let output_size =
            offset_of!(NTFS_FILE_RECORD_OUTPUT_BUFFER, FileRecordBuffer) + record_size;
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

        let record_offset = offset_of!(NTFS_FILE_RECORD_OUTPUT_BUFFER, FileRecordBuffer);
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
    use rebecca_ntfs::ParseCaveat;

    use super::{
        MftRecordSource, NTFS_VOLUME_DATA_BUFFER, ParsedMftRecords, ScanCancellationToken,
        VolumePaths, low_file_reference_number, read_mft_records_from_sources,
    };
    use crate::error::{RebeccaError, Result};

    #[test]
    fn volume_paths_support_drive_absolute_paths() {
        let paths = VolumePaths::from_path(std::path::Path::new("C:\\Temp\\Cache")).unwrap();

        assert_eq!(paths.root_path, std::path::PathBuf::from("C:\\"));
        assert_eq!(paths.device_path, "\\\\.\\C:");
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
    fn record_source_strategy_returns_first_success() {
        let source = FakeRecordSource {
            label: "primary",
            behavior: FakeRecordSourceBehavior::Success("primary-success"),
        };

        let records = read_mft_records_from_sources(
            &[&source],
            &NTFS_VOLUME_DATA_BUFFER::default(),
            &ScanCancellationToken::new(),
        )
        .unwrap();

        assert_eq!(records.caveats.len(), 1);
        assert_eq!(records.caveats[0].code, "primary-success");
    }

    #[test]
    fn record_source_strategy_tries_next_fallback_capable_source() {
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
        )
        .unwrap();

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
        )
        .unwrap_err();

        assert!(matches!(err, RebeccaError::OperationCancelled(_)));
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
        ) -> Result<ParsedMftRecords> {
            match self.behavior {
                FakeRecordSourceBehavior::Success(code) => Ok(ParsedMftRecords {
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
}

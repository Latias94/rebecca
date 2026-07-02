use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::mem::{offset_of, size_of};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::path::{Component, Path, PathBuf, Prefix};
use std::ptr;
use std::sync::{Arc, Mutex};

use rebecca_ntfs::{MftIndex, MftRecordReader, NtfsParsedRecord, ParseCaveat};
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

use crate::error::{RebeccaError, Result, ScanFailure, ScanFailurePhase};
use crate::safety::is_reparse_like;

use super::backend::{MeasuredScan, ScanBackend, ScanBackendKind, ScanRequest};
use super::progress::{ScanProgressEvent, check_not_cancelled};
use super::{ScanCancellationToken, ScanReport};

const EXPERIMENTAL_NTFS_MFT_BACKEND_LABEL: &str = "windows-ntfs-mft-experimental";
const NTFS_FILE_SYSTEM_NAME: &str = "NTFS";
const DRIVE_FIXED: u32 = 3;
const FILE_REFERENCE_LOW_MASK: u64 = 0x0000_FFFF_FFFF_FFFF;
const SEQUENTIAL_MFT_SOURCE_LABEL: &str = "sequential";
const FSCTL_RECORD_SOURCE_LABEL: &str = "fsctl-record";
const SEQUENTIAL_MFT_CHUNK_BYTES: usize = 4 * 1024 * 1024;
const MAX_RETRIEVAL_POINTER_BUFFER_BYTES: usize = 16 * 1024 * 1024;
const MAX_MFT_PARSE_ERROR_CAVEAT_SAMPLES: usize = 8;
const MAX_MFT_ESTIMATE_CAVEAT_SAMPLES_PER_CODE: usize = 8;
const MFT_CAVEAT_SUMMARY_CODE: &str = "mft-caveat-summary";

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
    mft_index: MftIndex,
    source_label: &'static str,
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
            mft_index: MftIndex::from_parsed_records(records.records),
            source_label: records.source_label,
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
        let Some(_) = index.mft_index.get(target_identity.file_reference_number) else {
            return Err(RebeccaError::PlatformUnavailable(format!(
                "{EXPERIMENTAL_NTFS_MFT_BACKEND_LABEL} could not map {} to MFT record {}",
                request.path.display(),
                target_identity.file_reference_number
            )));
        };

        let summary = index
            .mft_index
            .aggregate_subtree(target_identity.file_reference_number);
        let report = ScanReport {
            bytes_scanned: summary.bytes,
            files_scanned: summary.files,
            directories_scanned: summary.directories,
        };
        let measured = MeasuredScan::exact(report, self.kind())
            .with_backend_source(mft_backend_source_label(index.source_label));

        Ok(with_bounded_mft_caveats(
            measured,
            index.caveats.iter().cloned().chain(summary.caveats),
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NtfsVolumeCapabilities {
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
    ) -> Result<ParsedNtfsRecords>;
}

fn read_mft_records_from_sources(
    sources: &[&dyn MftRecordSource],
    volume_data: &NTFS_VOLUME_DATA_BUFFER,
    cancellation: &ScanCancellationToken,
) -> Result<ParsedNtfsRecords> {
    let mut fallback_errors = Vec::new();

    for source in sources {
        check_not_cancelled(cancellation)?;
        match source.read_records(volume_data, cancellation) {
            Ok(mut records) => {
                records.source_label = source.label();
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
    ) -> Result<ParsedNtfsRecords> {
        let sequential_source = SequentialMftDataSource { volume: self };
        let fsctl_source = FsctlRecordMftSource { volume: self };
        read_mft_records_from_sources(
            &[&sequential_source, &fsctl_source],
            volume_data,
            cancellation,
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

    fn mft_extents(&self, mft_data: &LiveNtfsMetadataFile) -> Result<Vec<MftExtent>> {
        let mut input = STARTING_VCN_INPUT_BUFFER { StartingVcn: 0 };
        let mut output = vec![
            0_u8;
            offset_of!(RETRIEVAL_POINTERS_BUFFER, Extents)
                + (32 * size_of::<RETRIEVAL_POINTERS_BUFFER_0>())
        ];

        loop {
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
    ) -> Result<ParsedNtfsRecords> {
        let geometry = NtfsRecordGeometry::from_volume_data(&self.volume.device_path, volume_data)?;
        let mft_data = self.volume.open_mft_data_stream()?;
        let extents = self.volume.mft_extents(&mft_data)?;
        let reader = MftRecordReader::new(geometry.record_size, geometry.sector_size);
        let mut records = Vec::new();
        let mut caveats = Vec::new();
        let mut parse_errors = MftParseErrorCaveats::default();

        for extent in extents {
            self.read_extent_records(
                extent,
                geometry,
                &reader,
                cancellation,
                &mut records,
                &mut parse_errors,
            )?;
        }
        parse_errors.append_to(&mut caveats);

        Ok(ParsedNtfsRecords {
            source_label: self.label(),
            records,
            caveats,
        })
    }
}

impl SequentialMftDataSource<'_> {
    fn read_extent_records(
        &self,
        extent: MftExtent,
        geometry: NtfsRecordGeometry,
        reader: &MftRecordReader,
        cancellation: &ScanCancellationToken,
        records: &mut Vec<NtfsParsedRecord>,
        parse_errors: &mut MftParseErrorCaveats,
    ) -> Result<()> {
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
            check_not_cancelled(cancellation)?;
            let records_remaining = geometry.max_record_count.saturating_sub(next_record_id);
            let read_len =
                next_mft_chunk_len(bytes_remaining, records_remaining, geometry.record_size);
            if read_len == 0 {
                break;
            }

            let bytes = self.volume.read_volume_bytes(volume_offset, read_len)?;
            if bytes.len() != read_len {
                return Err(RebeccaError::PlatformUnavailable(format!(
                    "{EXPERIMENTAL_NTFS_MFT_BACKEND_LABEL} {SEQUENTIAL_MFT_SOURCE_LABEL} read only {} of {read_len} requested bytes from {}",
                    bytes.len(),
                    self.volume.device_path
                )));
            }

            let batch = reader.parse_records_from(next_record_id, &bytes);
            records.extend(batch.records);
            for err in batch.errors {
                parse_errors.record(err.record_id, err.error);
            }

            let read_len = read_len as u64;
            let records_read = read_len / geometry.record_size as u64;
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
    ) -> Result<ParsedNtfsRecords> {
        let geometry = NtfsRecordGeometry::from_volume_data(&self.volume.device_path, volume_data)?;

        let mut records = Vec::new();
        let mut caveats = Vec::new();
        let mut parse_errors = MftParseErrorCaveats::default();
        let mut requested_record = 0_u64;

        while requested_record < geometry.max_record_count {
            if requested_record.is_multiple_of(256) {
                check_not_cancelled(cancellation)?;
            }

            match self
                .volume
                .read_file_record(requested_record, geometry.record_size)
            {
                Ok(Some((record_id, raw_record))) => {
                    let parsed_record_id = low_file_reference_number(record_id);
                    match NtfsParsedRecord::parse(
                        parsed_record_id,
                        &raw_record,
                        geometry.sector_size,
                    ) {
                        Ok(record) => records.push(record),
                        Err(err) => parse_errors.record(parsed_record_id, err),
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
        MAX_MFT_ESTIMATE_CAVEAT_SAMPLES_PER_CODE, MAX_MFT_PARSE_ERROR_CAVEAT_SAMPLES,
        MFT_CAVEAT_SUMMARY_CODE, MftExtent, MftParseErrorCaveats, MftRecordSource,
        NTFS_VOLUME_DATA_BUFFER, NtfsRecordGeometry, ParsedNtfsRecords, RETRIEVAL_POINTERS_BUFFER,
        RETRIEVAL_POINTERS_BUFFER_0, SEQUENTIAL_MFT_CHUNK_BYTES, ScanCancellationToken,
        VolumePaths, low_file_reference_number, next_mft_chunk_len,
        parse_retrieval_pointer_extents, read_mft_records_from_sources, with_bounded_mft_caveats,
    };
    use crate::error::{RebeccaError, Result};
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

        assert_eq!(records.source_label, "primary");
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

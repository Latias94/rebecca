use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::error::{RebeccaError, Result};
use crate::scan::{MeasuredScan, ScanBackendKind, ScanEstimateConfidence, ScanReport};

pub const SCAN_CACHE_VERSION: u32 = 2;
pub const DEFAULT_DIRECTORY_SCAN_CACHE_MAX_AGE_SECONDS: u64 = 5 * 60;
const SCAN_CACHE_PRUNE_BATCH_LIMIT: usize = 64;
const SCAN_CACHE_DIR: &str = "scan";

mod store;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScanCachePolicy {
    directory_record_max_age_seconds: u64,
    write_durability: ScanCacheWriteDurability,
}

impl ScanCachePolicy {
    pub const fn new(directory_record_max_age_seconds: u64) -> Self {
        Self {
            directory_record_max_age_seconds,
            write_durability: ScanCacheWriteDurability::Fast,
        }
    }

    pub const fn directory_record_max_age_seconds(self) -> u64 {
        self.directory_record_max_age_seconds
    }

    pub const fn write_durability(self) -> ScanCacheWriteDurability {
        self.write_durability
    }

    pub const fn with_write_durability(
        mut self,
        write_durability: ScanCacheWriteDurability,
    ) -> Self {
        self.write_durability = write_durability;
        self
    }
}

impl Default for ScanCachePolicy {
    fn default() -> Self {
        Self::new(DEFAULT_DIRECTORY_SCAN_CACHE_MAX_AGE_SECONDS)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ScanCacheWriteDurability {
    #[default]
    Fast,
    Strict,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanCacheRecord {
    pub version: u32,
    pub root: PathBuf,
    #[serde(default = "default_scan_cache_backend")]
    pub backend: ScanBackendKind,
    #[serde(default = "default_scan_cache_confidence")]
    pub confidence: ScanEstimateConfidence,
    #[serde(default)]
    pub identity: ScanCacheIdentity,
    pub fingerprint: ScanCacheFingerprint,
    pub report: ScanReport,
    pub written_at_unix_seconds: u64,
}

impl ScanCacheRecord {
    pub fn new(
        root: PathBuf,
        snapshot: ScanCachePathSnapshot,
        measured_scan: MeasuredScan,
    ) -> Self {
        Self {
            version: SCAN_CACHE_VERSION,
            root,
            backend: measured_scan.backend,
            confidence: measured_scan.confidence,
            identity: snapshot.identity,
            fingerprint: snapshot.fingerprint,
            report: measured_scan.report,
            written_at_unix_seconds: unix_now(),
        }
    }

    fn miss_reason(
        &self,
        root: &Path,
        snapshot: &ScanCachePathSnapshot,
        policy: ScanCachePolicy,
        now_unix_seconds: u64,
    ) -> Option<ScanCacheMiss> {
        if self.version != SCAN_CACHE_VERSION
            || self.root != root
            || self.backend != ScanBackendKind::PortableRecursive
            || self.confidence != ScanEstimateConfidence::Exact
            || !self.fingerprint.matches_current(&snapshot.fingerprint)
            || !self.identity.matches_current(&snapshot.identity)
        {
            return Some(ScanCacheMiss::Stale);
        }

        if self.is_expired_directory_record(policy, now_unix_seconds) {
            return Some(ScanCacheMiss::Expired);
        }

        None
    }

    fn is_expired_directory_record(&self, policy: ScanCachePolicy, now_unix_seconds: u64) -> bool {
        self.fingerprint.file_type == ScanCacheFileType::Directory
            && now_unix_seconds.saturating_sub(self.written_at_unix_seconds)
                > policy.directory_record_max_age_seconds()
    }
}

fn default_scan_cache_backend() -> ScanBackendKind {
    ScanBackendKind::PortableRecursive
}

fn default_scan_cache_confidence() -> ScanEstimateConfidence {
    ScanEstimateConfidence::Exact
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanCachePathSnapshot {
    pub fingerprint: ScanCacheFingerprint,
    pub identity: ScanCacheIdentity,
}

impl ScanCachePathSnapshot {
    pub(crate) fn from_path(path: &Path) -> Result<Self> {
        Self::read_from_path(path).map_err(|err| {
            RebeccaError::ScanCacheUnavailable(format!(
                "scan cache metadata unavailable for {}: {}",
                path.display(),
                err
            ))
        })
    }

    pub(crate) fn read_from_path(path: &Path) -> std::io::Result<Self> {
        let metadata = std::fs::symlink_metadata(path)?;
        Ok(Self {
            fingerprint: ScanCacheFingerprint::from_metadata(&metadata),
            identity: ScanCacheIdentity::from_path_and_metadata(path, &metadata),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanCacheFingerprint {
    pub file_type: ScanCacheFileType,
    pub len: u64,
    pub modified_unix_seconds: Option<u64>,
}

impl ScanCacheFingerprint {
    pub fn from_path(path: &Path) -> Result<Self> {
        ScanCachePathSnapshot::from_path(path).map(|snapshot| snapshot.fingerprint)
    }

    fn from_metadata(metadata: &std::fs::Metadata) -> Self {
        let file_type = metadata.file_type();
        let file_type = if file_type.is_symlink() {
            ScanCacheFileType::Symlink
        } else if metadata.is_file() {
            ScanCacheFileType::File
        } else if metadata.is_dir() {
            ScanCacheFileType::Directory
        } else {
            ScanCacheFileType::Other
        };

        let modified_unix_seconds = metadata
            .modified()
            .ok()
            .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs());

        Self {
            file_type,
            len: metadata.len(),
            modified_unix_seconds,
        }
    }

    fn matches_current(&self, current: &Self) -> bool {
        if self.file_type != current.file_type {
            return false;
        }

        if self.file_type == ScanCacheFileType::Directory {
            return true;
        }

        self.len == current.len && self.modified_unix_seconds == current.modified_unix_seconds
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanCacheIdentity {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume_serial: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usn_checkpoint: Option<ScanCacheUsnCheckpoint>,
}

impl ScanCacheIdentity {
    fn from_path_and_metadata(path: &Path, metadata: &std::fs::Metadata) -> Self {
        platform_identity(path, metadata)
    }

    fn matches_current(&self, current: &Self) -> bool {
        optional_identity_matches(self.volume_serial, current.volume_serial)
            && optional_identity_matches(self.file_id, current.file_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanCacheUsnCheckpoint {
    pub journal_id: u64,
    pub next_usn: u64,
}

fn optional_identity_matches(stored: Option<u64>, current: Option<u64>) -> bool {
    stored.is_none_or(|stored| current == Some(stored))
}

#[cfg(windows)]
fn platform_identity(path: &Path, metadata: &std::fs::Metadata) -> ScanCacheIdentity {
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, FILE_FLAG_BACKUP_SEMANTICS, FILE_READ_ATTRIBUTES,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, GetFileInformationByHandle,
    };

    let mut options = std::fs::OpenOptions::new();
    options
        .access_mode(FILE_READ_ATTRIBUTES.0)
        .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0 | FILE_SHARE_DELETE.0);

    if metadata.is_dir() {
        options.custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0);
    }

    let Ok(file) = options.open(path) else {
        return ScanCacheIdentity::default();
    };

    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    if unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }.is_err() {
        return ScanCacheIdentity::default();
    }

    let file_id = (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow);
    ScanCacheIdentity {
        volume_serial: Some(u64::from(info.dwVolumeSerialNumber)),
        file_id: Some(file_id),
        usn_checkpoint: None,
    }
}

#[cfg(unix)]
fn platform_identity(_path: &Path, metadata: &std::fs::Metadata) -> ScanCacheIdentity {
    use std::os::unix::fs::MetadataExt;

    ScanCacheIdentity {
        volume_serial: Some(metadata.dev()),
        file_id: Some(metadata.ino()),
        usn_checkpoint: None,
    }
}

#[cfg(not(any(windows, unix)))]
fn platform_identity(_path: &Path, _metadata: &std::fs::Metadata) -> ScanCacheIdentity {
    ScanCacheIdentity::default()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScanCacheFileType {
    File,
    Directory,
    Symlink,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanCacheLookup {
    Hit(ScanReport),
    Miss(ScanCacheMissOutcome),
}

impl ScanCacheLookup {
    pub fn miss(reason: ScanCacheMiss) -> Self {
        Self::Miss(ScanCacheMissOutcome {
            reason,
            pruned: false,
        })
    }

    pub fn pruned_miss(reason: ScanCacheMiss) -> Self {
        Self::Miss(ScanCacheMissOutcome {
            reason,
            pruned: true,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScanCacheMissOutcome {
    pub reason: ScanCacheMiss,
    pub pruned: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScanCachePruneReport {
    pub inspected: usize,
    pub pruned: usize,
    pub retained: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanCacheMiss {
    Missing,
    Stale,
    Expired,
    Corrupted,
    MetadataUnavailable,
}

impl ScanCacheMiss {
    pub fn label(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Stale => "stale",
            Self::Expired => "expired",
            Self::Corrupted => "corrupted",
            Self::MetadataUnavailable => "metadata-unavailable",
        }
    }

    fn should_prune_cache_file(self) -> bool {
        matches!(self, Self::Stale | Self::Expired | Self::Corrupted)
    }
}

#[derive(Debug, Clone)]
pub struct ScanCacheStore {
    root_dir: PathBuf,
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::error::{RebeccaError, Result};
use crate::scan::ScanReport;

pub const SCAN_CACHE_VERSION: u32 = 1;
pub const DEFAULT_DIRECTORY_SCAN_CACHE_MAX_AGE_SECONDS: u64 = 5 * 60;
const SCAN_CACHE_PRUNE_BATCH_LIMIT: usize = 64;
const SCAN_CACHE_DIR: &str = "scan";

mod store;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScanCachePolicy {
    directory_record_max_age_seconds: u64,
}

impl ScanCachePolicy {
    pub const fn new(directory_record_max_age_seconds: u64) -> Self {
        Self {
            directory_record_max_age_seconds,
        }
    }

    pub const fn directory_record_max_age_seconds(self) -> u64 {
        self.directory_record_max_age_seconds
    }
}

impl Default for ScanCachePolicy {
    fn default() -> Self {
        Self::new(DEFAULT_DIRECTORY_SCAN_CACHE_MAX_AGE_SECONDS)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanCacheRecord {
    pub version: u32,
    pub root: PathBuf,
    pub fingerprint: ScanCacheFingerprint,
    pub report: ScanReport,
    pub written_at_unix_seconds: u64,
}

impl ScanCacheRecord {
    pub fn new(root: PathBuf, fingerprint: ScanCacheFingerprint, report: ScanReport) -> Self {
        Self {
            version: SCAN_CACHE_VERSION,
            root,
            fingerprint,
            report,
            written_at_unix_seconds: unix_now(),
        }
    }

    fn miss_reason(
        &self,
        root: &Path,
        fingerprint: &ScanCacheFingerprint,
        policy: ScanCachePolicy,
        now_unix_seconds: u64,
    ) -> Option<ScanCacheMiss> {
        if self.version != SCAN_CACHE_VERSION
            || self.root != root
            || &self.fingerprint != fingerprint
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanCacheFingerprint {
    pub file_type: ScanCacheFileType,
    pub len: u64,
    pub modified_unix_seconds: Option<u64>,
}

impl ScanCacheFingerprint {
    pub fn from_path(path: &Path) -> Result<Self> {
        let metadata = std::fs::symlink_metadata(path).map_err(|err| {
            RebeccaError::ScanCacheUnavailable(format!(
                "scan cache metadata unavailable for {}: {}",
                path.display(),
                err
            ))
        })?;

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

        Ok(Self {
            file_type,
            len: metadata.len(),
            modified_unix_seconds,
        })
    }
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
    Miss(ScanCacheMiss),
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

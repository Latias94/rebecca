use std::path::Path;

use crate::protection::{ProtectionAssessment, ProtectionPolicy};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathDisposition {
    Allowed,
    Skipped(String),
    Blocked(String),
}

impl PathDisposition {
    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::Allowed)
    }
}

pub fn assess_path(path: &Path) -> PathDisposition {
    assess_path_with_policy(path, ProtectionPolicy::new())
}

pub fn assess_path_with_policy(path: &Path, policy: ProtectionPolicy<'_>) -> PathDisposition {
    match policy.assess_path(path) {
        ProtectionAssessment::Allowed => PathDisposition::Allowed,
        ProtectionAssessment::Blocked(block) => PathDisposition::Blocked(block.message),
    }
}

pub fn assess_existing_path(path: &Path) -> PathDisposition {
    assess_existing_path_with_policy(path, ProtectionPolicy::new())
}

pub fn assess_existing_path_with_policy(
    path: &Path,
    policy: ProtectionPolicy<'_>,
) -> PathDisposition {
    let path_disposition = assess_path_with_policy(path, policy);
    if !path_disposition.is_allowed() {
        return path_disposition;
    }

    match std::fs::symlink_metadata(path) {
        Ok(metadata) => {
            if is_reparse_like(&metadata) {
                return PathDisposition::Blocked("reparse-point traversal is disabled".to_string());
            }

            PathDisposition::Allowed
        }
        Err(_) => PathDisposition::Skipped("path does not exist".to_string()),
    }
}

pub fn is_reparse_like(metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;

        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
        (metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT) != 0
            || metadata.file_type().is_symlink()
    }

    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

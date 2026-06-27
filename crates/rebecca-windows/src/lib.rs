use rebecca_core::error::Result;
use rebecca_core::executor::{CleanupBackend, ExecutionOutcome};
use rebecca_core::plan::CleanupTarget;

pub mod apps;
pub mod steam;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivilegeLevel {
    StandardUser,
    Elevated,
    Unknown,
}

pub fn current_privilege_level() -> PrivilegeLevel {
    platform::current_privilege_level()
}

#[derive(Debug, Default, Clone, Copy)]
pub struct WindowsRecycleBinBackend;

impl WindowsRecycleBinBackend {
    pub fn new() -> Self {
        Self
    }
}

impl CleanupBackend for WindowsRecycleBinBackend {
    fn delete(&self, target: &CleanupTarget) -> Result<ExecutionOutcome> {
        platform::delete_to_recycle_bin(&target.path, target.estimated_bytes, target.deletion_style)
    }
}

#[cfg(windows)]
mod platform {
    use std::fs;
    use std::path::Path;

    use rebecca_core::error::{RebeccaError, Result};
    use rebecca_core::executor::ExecutionOutcome;
    use windows::Win32::UI::Shell::IsUserAnAdmin;

    pub fn current_privilege_level() -> super::PrivilegeLevel {
        unsafe {
            if IsUserAnAdmin().as_bool() {
                super::PrivilegeLevel::Elevated
            } else {
                super::PrivilegeLevel::StandardUser
            }
        }
    }

    pub fn delete_to_recycle_bin(
        path: &Path,
        estimated_bytes: u64,
        deletion_style: rebecca_core::CleanupTargetDeletionStyle,
    ) -> Result<ExecutionOutcome> {
        match deletion_style {
            rebecca_core::CleanupTargetDeletionStyle::DeleteWholePath => {
                trash::delete(path)
                    .map_err(|err| RebeccaError::ExecutionFailed(err.to_string()))?;
            }
            rebecca_core::CleanupTargetDeletionStyle::PreserveRootContents => {
                if path.is_dir() {
                    for entry in fs::read_dir(path)? {
                        let entry = entry?;
                        trash::delete(entry.path())
                            .map_err(|err| RebeccaError::ExecutionFailed(err.to_string()))?;
                    }
                } else {
                    trash::delete(path)
                        .map_err(|err| RebeccaError::ExecutionFailed(err.to_string()))?;
                }
            }
        }

        Ok(ExecutionOutcome {
            freed_bytes: 0,
            pending_reclaim_bytes: estimated_bytes,
            note: Some("moved to Recycle Bin".to_string()),
        })
    }
}

#[cfg(not(windows))]
mod platform {
    use std::path::Path;

    use rebecca_core::error::{RebeccaError, Result};
    use rebecca_core::executor::ExecutionOutcome;

    pub fn current_privilege_level() -> super::PrivilegeLevel {
        super::PrivilegeLevel::Unknown
    }

    pub fn delete_to_recycle_bin(
        _path: &Path,
        _estimated_bytes: u64,
        _deletion_style: rebecca_core::CleanupTargetDeletionStyle,
    ) -> Result<ExecutionOutcome> {
        Err(RebeccaError::PlatformUnavailable(
            "Windows recycle bin deletion is not available on this platform".to_string(),
        ))
    }
}

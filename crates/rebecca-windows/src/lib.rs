use std::path::Path;

use rebecca_core::cache::{CachePurgeBackend, CachePurgeEntryKind, CachePurgeOutcome};
use rebecca_core::error::{RebeccaError, Result};
use rebecca_core::executor::{CleanupBackend, ExecutionOutcome};
use rebecca_core::plan::{CleanupTarget, CleanupTargetDeletionStyle};

pub mod apps;
pub mod process;
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

    fn supports_batch_delete(&self) -> bool {
        cfg!(windows)
    }

    fn delete_batch(&self, targets: &[&CleanupTarget]) -> Vec<Result<ExecutionOutcome>> {
        platform::delete_batch_to_recycle_bin(targets)
    }
}

impl CachePurgeBackend for WindowsRecycleBinBackend {
    fn purge(
        &self,
        path: &Path,
        kind: CachePurgeEntryKind,
        estimated_bytes: u64,
    ) -> Result<CachePurgeOutcome> {
        match kind {
            CachePurgeEntryKind::File | CachePurgeEntryKind::Directory => {
                platform::delete_to_recycle_bin(
                    path,
                    estimated_bytes,
                    CleanupTargetDeletionStyle::DeleteWholePath,
                )
                .map(|outcome| CachePurgeOutcome {
                    reclaimed_bytes: outcome.freed_bytes,
                    pending_reclaim_bytes: outcome.pending_reclaim_bytes,
                    note: outcome.note,
                })
            }
            CachePurgeEntryKind::Symlink | CachePurgeEntryKind::Other => {
                Err(RebeccaError::ExecutionFailed(format!(
                    "cache purge backend does not support {} entries",
                    kind.label()
                )))
            }
        }
    }
}

#[cfg(windows)]
mod platform {
    use std::fs;
    use std::path::{Path, PathBuf};

    use rebecca_core::error::{RebeccaError, Result};
    use rebecca_core::executor::ExecutionOutcome;
    use rebecca_core::plan::{CleanupTarget, CleanupTargetDeletionStyle};
    use windows::Win32::UI::Shell::IsUserAnAdmin;

    struct BatchRecycleTarget {
        target_index: usize,
        delete_paths: Vec<PathBuf>,
    }

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
        deletion_style: CleanupTargetDeletionStyle,
    ) -> Result<ExecutionOutcome> {
        match deletion_style {
            CleanupTargetDeletionStyle::DeleteWholePath => {
                trash::delete(path)
                    .map_err(|err| RebeccaError::ExecutionFailed(err.to_string()))?;
            }
            CleanupTargetDeletionStyle::PreserveRootContents => {
                if path.is_dir() {
                    let entries = fs::read_dir(path)?
                        .map(|entry| entry.map(|entry| entry.path()))
                        .collect::<std::io::Result<Vec<_>>>()?;
                    if !entries.is_empty() {
                        trash::delete_all(entries)
                            .map_err(|err| RebeccaError::ExecutionFailed(err.to_string()))?;
                    }
                } else {
                    trash::delete(path)
                        .map_err(|err| RebeccaError::ExecutionFailed(err.to_string()))?;
                }
            }
        }

        Ok(recycle_bin_outcome(estimated_bytes))
    }

    pub fn delete_batch_to_recycle_bin(
        targets: &[&CleanupTarget],
    ) -> Vec<Result<ExecutionOutcome>> {
        let mut outcomes = (0..targets.len()).map(|_| None).collect::<Vec<_>>();
        let mut batch_targets = Vec::new();
        let mut batch_paths = Vec::new();

        for (target_index, target) in targets.iter().enumerate() {
            match recycle_paths_for_target(target) {
                Ok(delete_paths) if delete_paths.is_empty() => {
                    outcomes[target_index] = Some(Ok(recycle_bin_outcome(target.estimated_bytes)));
                }
                Ok(delete_paths) => {
                    batch_paths.extend(delete_paths.iter().cloned());
                    batch_targets.push(BatchRecycleTarget {
                        target_index,
                        delete_paths,
                    });
                }
                Err(err) => {
                    outcomes[target_index] = Some(Err(err));
                }
            }
        }

        if !batch_paths.is_empty() {
            match trash::delete_all(batch_paths.iter()) {
                Ok(()) => {
                    for batch_target in batch_targets {
                        let target = targets[batch_target.target_index];
                        outcomes[batch_target.target_index] =
                            Some(Ok(recycle_bin_outcome(target.estimated_bytes)));
                    }
                }
                Err(_) => {
                    for batch_target in batch_targets {
                        let target = targets[batch_target.target_index];
                        outcomes[batch_target.target_index] =
                            Some(reconstruct_or_fallback_after_batch_failure(
                                target,
                                &batch_target.delete_paths,
                            ));
                    }
                }
            }
        }

        outcomes
            .into_iter()
            .map(|outcome| {
                outcome.unwrap_or_else(|| {
                    Err(RebeccaError::ExecutionFailed(
                        "batch recycle bin backend did not produce a target outcome".to_string(),
                    ))
                })
            })
            .collect()
    }

    fn recycle_paths_for_target(target: &CleanupTarget) -> Result<Vec<PathBuf>> {
        let metadata = fs::symlink_metadata(&target.path)?;
        match target.deletion_style {
            CleanupTargetDeletionStyle::DeleteWholePath => Ok(vec![target.path.clone()]),
            CleanupTargetDeletionStyle::PreserveRootContents => {
                if metadata.is_dir() {
                    fs::read_dir(&target.path)?
                        .map(|entry| entry.map(|entry| entry.path()))
                        .collect::<std::io::Result<Vec<_>>>()
                        .map_err(Into::into)
                } else {
                    Ok(vec![target.path.clone()])
                }
            }
        }
    }

    fn reconstruct_or_fallback_after_batch_failure(
        target: &CleanupTarget,
        delete_paths: &[PathBuf],
    ) -> Result<ExecutionOutcome> {
        if delete_paths
            .iter()
            .all(|path| matches!(path.try_exists(), Ok(false)))
        {
            return Ok(recycle_bin_outcome(target.estimated_bytes));
        }

        delete_to_recycle_bin(&target.path, target.estimated_bytes, target.deletion_style)
    }

    fn recycle_bin_outcome(estimated_bytes: u64) -> ExecutionOutcome {
        ExecutionOutcome {
            freed_bytes: 0,
            pending_reclaim_bytes: estimated_bytes,
            note: Some("moved to Recycle Bin".to_string()),
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use std::path::Path;

    use rebecca_core::error::{RebeccaError, Result};
    use rebecca_core::executor::ExecutionOutcome;
    use rebecca_core::plan::CleanupTarget;

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

    pub fn delete_batch_to_recycle_bin(
        targets: &[&CleanupTarget],
    ) -> Vec<Result<ExecutionOutcome>> {
        targets
            .iter()
            .map(|target| {
                delete_to_recycle_bin(&target.path, target.estimated_bytes, target.deletion_style)
            })
            .collect()
    }
}

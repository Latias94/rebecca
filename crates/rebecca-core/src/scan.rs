use std::path::Path;

use ignore::WalkBuilder;
use rayon::prelude::*;

use crate::TargetStatus;
use crate::error::{RebeccaError, Result};
use crate::plan::CleanupTarget;
use crate::safety::{PathDisposition, assess_existing_path};

pub fn measure_path_size(path: &Path) -> Result<u64> {
    let metadata = std::fs::symlink_metadata(path)?;

    if metadata.file_type().is_symlink() {
        return Err(RebeccaError::SafetyBlocked(
            "symlink traversal is disabled".to_string(),
        ));
    }

    if metadata.is_file() {
        return Ok(metadata.len());
    }

    if metadata.is_dir() {
        return measure_directory_size(path);
    }

    Ok(0)
}

pub fn measure_directory_size(path: &Path) -> Result<u64> {
    let mut total = 0u64;
    let walker = WalkBuilder::new(path)
        .hidden(false)
        .follow_links(false)
        .build();

    for entry in walker {
        let entry = entry.map_err(|err| RebeccaError::ScanFailed(err.to_string()))?;
        let metadata = entry
            .metadata()
            .map_err(|err| RebeccaError::ScanFailed(err.to_string()))?;

        if metadata.file_type().is_symlink() {
            continue;
        }

        if metadata.is_file() {
            total = total.saturating_add(metadata.len());
        }
    }

    Ok(total)
}

pub fn scan_target(
    rule_id: impl Into<String>,
    path: std::path::PathBuf,
    mode: crate::DeleteMode,
) -> CleanupTarget {
    let rule_id = rule_id.into();

    match assess_existing_path(&path) {
        PathDisposition::Allowed => match measure_path_size(&path) {
            Ok(size) => CleanupTarget::allowed(rule_id, path, size, mode),
            Err(err) => CleanupTarget::failed(rule_id, path, mode, 0, err.to_string()),
        },
        PathDisposition::Skipped(reason) => CleanupTarget::skipped(rule_id, path, mode, reason),
        PathDisposition::Blocked(reason) => CleanupTarget::blocked(rule_id, path, mode, reason),
    }
}

pub fn scan_targets(
    targets: Vec<(String, std::path::PathBuf, crate::DeleteMode)>,
) -> Vec<CleanupTarget> {
    let mut scanned: Vec<_> = targets
        .into_par_iter()
        .map(|(rule_id, path, mode)| scan_target(rule_id, path, mode))
        .collect();

    scanned.sort_by(|left, right| {
        left.rule_id
            .cmp(&right.rule_id)
            .then_with(|| left.path.cmp(&right.path))
    });

    scanned
}

pub fn allowed_target_count(targets: &[CleanupTarget]) -> usize {
    targets
        .iter()
        .filter(|target| matches!(target.status, TargetStatus::Allowed))
        .count()
}

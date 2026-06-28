use std::collections::{BTreeSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::error::{RebeccaError, Result};
use crate::scan::ScanCancellationToken;

use super::catalog::{cachedir_tag_definition, definition_for_directory, should_prune_scan_dir};
use super::{ProjectArtifactCandidate, ProjectArtifactDefinition, ProjectArtifactScanOptions};

const CACHEDIR_TAG_FILE_NAME: &str = "CACHEDIR.TAG";
const CACHEDIR_TAG_SIGNATURE: &str = "Signature: 8a477f597d28d172789f06886806bc55";

pub fn discover_project_artifacts(
    options: &ProjectArtifactScanOptions,
    cancellation: &ScanCancellationToken,
) -> Result<Vec<ProjectArtifactCandidate>> {
    let mut candidates = Vec::new();
    let mut seen_paths = BTreeSet::new();

    for root in &options.roots {
        scan_root(
            root,
            options.max_depth,
            cancellation,
            &mut seen_paths,
            &mut candidates,
        )?;
    }

    candidates.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then_with(|| left.definition.rule_id.cmp(right.definition.rule_id))
    });
    Ok(candidates)
}

pub fn recently_modified_reason(path: &Path, min_age_days: u64) -> Option<String> {
    if min_age_days == 0 || !is_recently_modified(path, min_age_days, SystemTime::now()) {
        return None;
    }

    Some(format!(
        "project artifact was modified within the last {}",
        format_days(min_age_days)
    ))
}

fn is_recently_modified(path: &Path, min_age_days: u64, now: SystemTime) -> bool {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return false;
    };
    let Ok(modified) = metadata.modified() else {
        return false;
    };

    let age = now.duration_since(modified).unwrap_or(Duration::ZERO);
    age.as_secs() < min_age_days.saturating_mul(24 * 60 * 60)
}

fn format_days(days: u64) -> String {
    if days == 1 {
        "1 day".to_string()
    } else {
        format!("{days} days")
    }
}

fn scan_root(
    root: &Path,
    max_depth: usize,
    cancellation: &ScanCancellationToken,
    seen_paths: &mut BTreeSet<String>,
    candidates: &mut Vec<ProjectArtifactCandidate>,
) -> Result<()> {
    check_cancelled(cancellation)?;

    let metadata = match fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err.into()),
    };

    if !metadata.is_dir() || crate::safety::is_reparse_like(&metadata) {
        return Ok(());
    }

    let mut pending = VecDeque::from([(root.to_path_buf(), 0_usize)]);
    while let Some((dir, depth)) = pending.pop_front() {
        check_cancelled(cancellation)?;

        if let Some(name) = dir.file_name().and_then(|name| name.to_str()) {
            if let Some(definition) = definition_for_directory(&dir, name) {
                push_candidate(definition, dir, seen_paths, candidates);
                continue;
            }

            if should_prune_scan_dir(name) {
                continue;
            }
        }

        if depth > 0 && has_valid_cachedir_tag(&dir) {
            push_candidate(cachedir_tag_definition(), dir, seen_paths, candidates);
            continue;
        }

        if depth >= max_depth {
            continue;
        }

        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(err) => {
                tracing::debug!(
                    path = %dir.display(),
                    error = %err,
                    "project artifact directory read skipped"
                );
                continue;
            }
        };
        let mut child_dirs = Vec::new();

        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(err) => {
                    tracing::debug!(
                        path = %dir.display(),
                        error = %err,
                        "project artifact directory entry skipped"
                    );
                    continue;
                }
            };
            let path = entry.path();
            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(err) => {
                    tracing::debug!(
                        path = %path.display(),
                        error = %err,
                        "project artifact metadata read skipped"
                    );
                    continue;
                }
            };

            if metadata.is_dir() && !crate::safety::is_reparse_like(&metadata) {
                child_dirs.push(path);
            }
        }

        child_dirs.sort();
        for child in child_dirs {
            pending.push_back((child, depth.saturating_add(1)));
        }
    }

    Ok(())
}

fn has_valid_cachedir_tag(dir: &Path) -> bool {
    let tag = dir.join(CACHEDIR_TAG_FILE_NAME);
    let Ok(metadata) = fs::symlink_metadata(&tag) else {
        return false;
    };

    if !metadata.is_file() || crate::safety::is_reparse_like(&metadata) {
        return false;
    }

    let Ok(contents) = fs::read_to_string(&tag) else {
        return false;
    };

    contents.starts_with(CACHEDIR_TAG_SIGNATURE)
}

fn push_candidate(
    definition: ProjectArtifactDefinition,
    path: PathBuf,
    seen_paths: &mut BTreeSet<String>,
    candidates: &mut Vec<ProjectArtifactCandidate>,
) {
    let key = path.as_os_str().to_string_lossy().replace('\\', "/");
    if seen_paths.insert(key.to_ascii_lowercase()) {
        candidates.push(ProjectArtifactCandidate {
            definition,
            path: path.clone(),
            modified_at_unix_seconds: modified_at_unix_seconds(&path),
        });
    }
}

fn modified_at_unix_seconds(path: &Path) -> Option<u64> {
    let metadata = fs::symlink_metadata(path).ok()?;
    metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
}

fn check_cancelled(cancellation: &ScanCancellationToken) -> Result<()> {
    if cancellation.is_cancelled() {
        return Err(RebeccaError::OperationCancelled(
            "project artifact scan was cancelled".to_string(),
        ));
    }

    Ok(())
}

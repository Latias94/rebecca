use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{RebeccaError, Result};
use crate::inventory::{
    Inventory, InventoryDiagnostic, InventoryEntryRole, InventoryFile, InventoryRequest,
    build_inventory,
};
use crate::scan::ScanCancellationToken;

pub const DEFAULT_LARGE_FILE_THRESHOLD_BYTES: u64 = 100 * 1024 * 1024;
pub const DEFAULT_LINT_TOP_LIMIT: usize = 20;
const PREHASH_BYTES: usize = 4096;

#[derive(Debug, Clone)]
pub struct LintReportRequest {
    pub inventory: InventoryRequest,
    pub large_file_threshold_bytes: u64,
    pub top_limit: usize,
}

impl LintReportRequest {
    pub fn new(roots: Vec<PathBuf>) -> Self {
        Self {
            inventory: InventoryRequest::new(roots),
            large_file_threshold_bytes: DEFAULT_LARGE_FILE_THRESHOLD_BYTES,
            top_limit: DEFAULT_LINT_TOP_LIMIT,
        }
    }

    pub fn with_reference_roots(mut self, reference_roots: Vec<PathBuf>) -> Self {
        self.inventory = self.inventory.with_reference_roots(reference_roots);
        self
    }

    pub fn with_protected_roots(mut self, protected_roots: Vec<PathBuf>) -> Self {
        self.inventory = self.inventory.with_protected_roots(protected_roots);
        self
    }

    pub fn with_exclude_paths(mut self, exclude_paths: Vec<PathBuf>) -> Self {
        self.inventory = self.inventory.with_exclude_paths(exclude_paths);
        self
    }

    pub fn with_large_file_threshold_bytes(mut self, threshold: u64) -> Self {
        self.large_file_threshold_bytes = threshold;
        self
    }

    pub fn with_top_limit(mut self, top_limit: usize) -> Self {
        self.top_limit = top_limit;
        self
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LintReport {
    pub roots: Vec<PathBuf>,
    pub reference_roots: Vec<PathBuf>,
    pub summary: LintReportSummary,
    pub duplicate_groups: Vec<DuplicateFileGroup>,
    pub large_files: Vec<LintFileEntry>,
    pub empty_files: Vec<LintFileEntry>,
    pub empty_directories: Vec<LintDirectoryEntry>,
    pub diagnostics: Vec<InventoryDiagnostic>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LintReportSummary {
    pub files_scanned: u64,
    pub directories_scanned: u64,
    pub duplicate_groups: u64,
    pub duplicate_files: u64,
    pub large_files: u64,
    pub empty_files: u64,
    pub empty_directories: u64,
    pub conservative_reclaim_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DuplicateFileGroup {
    pub size_bytes: u64,
    pub total_files: u64,
    pub keep_candidates: u64,
    pub conservative_reclaim_bytes: u64,
    pub hash: String,
    pub files: Vec<LintFileEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LintFileEntry {
    pub path: PathBuf,
    pub root: PathBuf,
    pub size_bytes: u64,
    pub modified_at_unix_seconds: Option<u64>,
    pub role: InventoryEntryRole,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LintDirectoryEntry {
    pub path: PathBuf,
    pub root: PathBuf,
    pub depth: usize,
    pub role: InventoryEntryRole,
}

pub fn inspect_lint(
    request: &LintReportRequest,
    cancellation: &ScanCancellationToken,
) -> Result<LintReport> {
    let inventory = build_inventory(&request.inventory, cancellation)?;
    lint_inventory(request, inventory, cancellation)
}

pub fn lint_inventory(
    request: &LintReportRequest,
    inventory: Inventory,
    cancellation: &ScanCancellationToken,
) -> Result<LintReport> {
    let mut duplicate_groups = duplicate_file_groups(&inventory.files, cancellation)?;
    duplicate_groups.sort_by(|left, right| {
        right
            .conservative_reclaim_bytes
            .cmp(&left.conservative_reclaim_bytes)
            .then_with(|| right.size_bytes.cmp(&left.size_bytes))
            .then_with(|| left.files[0].path.cmp(&right.files[0].path))
    });

    let mut large_files = inventory
        .files
        .iter()
        .filter(|file| file.size_bytes >= request.large_file_threshold_bytes)
        .map(LintFileEntry::from)
        .collect::<Vec<_>>();
    large_files.sort_by(|left, right| {
        right
            .size_bytes
            .cmp(&left.size_bytes)
            .then_with(|| left.path.cmp(&right.path))
    });

    let mut empty_files = inventory
        .files
        .iter()
        .filter(|file| file.size_bytes == 0)
        .map(LintFileEntry::from)
        .collect::<Vec<_>>();
    empty_files.sort_by(|left, right| left.path.cmp(&right.path));

    let mut empty_directories = inventory
        .directories
        .iter()
        .filter(|directory| directory.is_empty)
        .map(LintDirectoryEntry::from)
        .collect::<Vec<_>>();
    empty_directories.sort_by(|left, right| {
        right
            .depth
            .cmp(&left.depth)
            .then_with(|| left.path.cmp(&right.path))
    });

    let summary = LintReportSummary {
        files_scanned: inventory.files.len() as u64,
        directories_scanned: inventory.directories.len() as u64,
        duplicate_groups: duplicate_groups.len() as u64,
        duplicate_files: duplicate_groups
            .iter()
            .map(|group| group.total_files)
            .sum::<u64>(),
        large_files: large_files.len() as u64,
        empty_files: empty_files.len() as u64,
        empty_directories: empty_directories.len() as u64,
        conservative_reclaim_bytes: duplicate_groups
            .iter()
            .map(|group| group.conservative_reclaim_bytes)
            .sum::<u64>(),
    };

    duplicate_groups.truncate(request.top_limit);
    large_files.truncate(request.top_limit);
    empty_files.truncate(request.top_limit);
    empty_directories.truncate(request.top_limit);

    Ok(LintReport {
        roots: request.inventory.roots.clone(),
        reference_roots: request.inventory.reference_roots.clone(),
        summary,
        duplicate_groups,
        large_files,
        empty_files,
        empty_directories,
        diagnostics: inventory.diagnostics,
    })
}

fn duplicate_file_groups(
    files: &[InventoryFile],
    cancellation: &ScanCancellationToken,
) -> Result<Vec<DuplicateFileGroup>> {
    let mut by_size = BTreeMap::<u64, Vec<&InventoryFile>>::new();
    for file in files.iter().filter(|file| file.size_bytes > 0) {
        by_size.entry(file.size_bytes).or_default().push(file);
    }

    let mut groups = Vec::new();
    for (size, size_bucket) in by_size {
        check_cancelled(cancellation)?;
        if size_bucket.len() < 2 {
            continue;
        }

        let mut by_prehash = BTreeMap::<u64, Vec<&InventoryFile>>::new();
        for file in size_bucket {
            check_cancelled(cancellation)?;
            let prehash = content_hash_limited(&file.path, PREHASH_BYTES)?;
            by_prehash.entry(prehash).or_default().push(file);
        }

        for prehash_bucket in by_prehash.into_values() {
            check_cancelled(cancellation)?;
            if prehash_bucket.len() < 2 {
                continue;
            }

            let mut by_full_hash = BTreeMap::<u64, Vec<&InventoryFile>>::new();
            for file in prehash_bucket {
                check_cancelled(cancellation)?;
                let full_hash = content_hash_full(&file.path)?;
                by_full_hash.entry(full_hash).or_default().push(file);
            }

            for (hash, mut hash_bucket) in by_full_hash {
                if hash_bucket.len() < 2 {
                    continue;
                }
                hash_bucket.sort_by(|left, right| {
                    entry_keep_rank(left)
                        .cmp(&entry_keep_rank(right))
                        .then_with(|| left.path.cmp(&right.path))
                });
                groups.push(DuplicateFileGroup::new(size, hash, hash_bucket));
            }
        }
    }

    Ok(groups)
}

impl DuplicateFileGroup {
    fn new(size_bytes: u64, hash: u64, files: Vec<&InventoryFile>) -> Self {
        let keep_candidates = files
            .iter()
            .filter(|file| file.role.is_keep_candidate())
            .count() as u64;
        let reclaimable_files = if keep_candidates > 0 {
            files
                .iter()
                .filter(|file| !file.role.is_keep_candidate())
                .count() as u64
        } else {
            files.len().saturating_sub(1) as u64
        };

        Self {
            size_bytes,
            total_files: files.len() as u64,
            keep_candidates,
            conservative_reclaim_bytes: size_bytes.saturating_mul(reclaimable_files),
            hash: format!("{hash:016x}"),
            files: files.into_iter().map(LintFileEntry::from).collect(),
        }
    }
}

fn entry_keep_rank(file: &InventoryFile) -> u8 {
    match file.role {
        InventoryEntryRole::Protected => 0,
        InventoryEntryRole::Reference => 1,
        InventoryEntryRole::Scanned => 2,
    }
}

impl From<&InventoryFile> for LintFileEntry {
    fn from(file: &InventoryFile) -> Self {
        Self {
            path: file.path.clone(),
            root: file.root.clone(),
            size_bytes: file.size_bytes,
            modified_at_unix_seconds: file.modified_at_unix_seconds,
            role: file.role,
        }
    }
}

impl From<&crate::inventory::InventoryDirectory> for LintDirectoryEntry {
    fn from(directory: &crate::inventory::InventoryDirectory) -> Self {
        Self {
            path: directory.path.clone(),
            root: directory.root.clone(),
            depth: directory.depth,
            role: directory.role,
        }
    }
}

fn content_hash_limited(path: &Path, limit: usize) -> Result<u64> {
    let file = std::fs::File::open(path).map_err(RebeccaError::Io)?;
    hash_reader(file.take(limit as u64))
}

fn content_hash_full(path: &Path) -> Result<u64> {
    let file = std::fs::File::open(path).map_err(RebeccaError::Io)?;
    hash_reader(file)
}

fn hash_reader(mut reader: impl Read) -> Result<u64> {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let read = reader.read(&mut buffer).map_err(RebeccaError::Io)?;
        if read == 0 {
            break;
        }
        for byte in &buffer[..read] {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    Ok(hash)
}

fn check_cancelled(cancellation: &ScanCancellationToken) -> Result<()> {
    if cancellation.is_cancelled() {
        return Err(RebeccaError::OperationCancelled(
            "lint inspection was cancelled".to_string(),
        ));
    }

    Ok(())
}

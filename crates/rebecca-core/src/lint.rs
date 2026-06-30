use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap};
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
    let duplicate_selection =
        duplicate_file_groups(&inventory.files, request.top_limit, cancellation)?;

    let mut large_files = BoundedLintFiles::new(request.top_limit);
    let mut empty_files = BoundedLintFiles::new(request.top_limit);
    let mut large_file_count = 0_u64;
    let mut empty_file_count = 0_u64;
    for file in &inventory.files {
        if file.size_bytes >= request.large_file_threshold_bytes {
            large_file_count = large_file_count.saturating_add(1);
            large_files.push(
                LintFileEntry::from(file),
                LintEntryRank {
                    primary: file.size_bytes,
                    reverse_path: Reverse(file.path.clone()),
                },
            );
        }

        if file.size_bytes == 0 {
            empty_file_count = empty_file_count.saturating_add(1);
            empty_files.push(
                LintFileEntry::from(file),
                LintEntryRank {
                    primary: 0,
                    reverse_path: Reverse(file.path.clone()),
                },
            );
        }
    }

    let mut empty_directories = BoundedLintDirectories::new(request.top_limit);
    let mut empty_directory_count = 0_u64;
    for directory in inventory
        .directories
        .iter()
        .filter(|directory| directory.is_empty)
    {
        empty_directory_count = empty_directory_count.saturating_add(1);
        empty_directories.push(
            LintDirectoryEntry::from(directory),
            LintEntryRank {
                primary: directory.depth as u64,
                reverse_path: Reverse(directory.path.clone()),
            },
        );
    }

    let summary = LintReportSummary {
        files_scanned: inventory.files.len() as u64,
        directories_scanned: inventory.directories.len() as u64,
        duplicate_groups: duplicate_selection.total_groups,
        duplicate_files: duplicate_selection.total_files,
        large_files: large_file_count,
        empty_files: empty_file_count,
        empty_directories: empty_directory_count,
        conservative_reclaim_bytes: duplicate_selection.conservative_reclaim_bytes,
    };

    Ok(LintReport {
        roots: request.inventory.roots.clone(),
        reference_roots: request.inventory.reference_roots.clone(),
        summary,
        duplicate_groups: duplicate_selection.groups,
        large_files: large_files.into_sorted_entries(),
        empty_files: empty_files.into_sorted_entries(),
        empty_directories: empty_directories.into_sorted_entries(),
        diagnostics: inventory.diagnostics,
    })
}

fn duplicate_file_groups(
    files: &[InventoryFile],
    top_limit: usize,
    cancellation: &ScanCancellationToken,
) -> Result<DuplicateGroupSelection> {
    let mut by_size = BTreeMap::<u64, Vec<&InventoryFile>>::new();
    for file in files.iter().filter(|file| file.size_bytes > 0) {
        by_size.entry(file.size_bytes).or_default().push(file);
    }

    let mut groups = DuplicateGroupAccumulator::new(top_limit);
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

    Ok(groups.into_selection())
}

#[derive(Debug, Default)]
struct DuplicateGroupSelection {
    total_groups: u64,
    total_files: u64,
    conservative_reclaim_bytes: u64,
    groups: Vec<DuplicateFileGroup>,
}

#[derive(Debug)]
struct DuplicateGroupAccumulator {
    total_groups: u64,
    total_files: u64,
    conservative_reclaim_bytes: u64,
    top: BoundedDuplicateGroups,
}

impl DuplicateGroupAccumulator {
    fn new(limit: usize) -> Self {
        Self {
            total_groups: 0,
            total_files: 0,
            conservative_reclaim_bytes: 0,
            top: BoundedDuplicateGroups::new(limit),
        }
    }

    fn push(&mut self, group: DuplicateFileGroup) {
        self.total_groups = self.total_groups.saturating_add(1);
        self.total_files = self.total_files.saturating_add(group.total_files);
        self.conservative_reclaim_bytes = self
            .conservative_reclaim_bytes
            .saturating_add(group.conservative_reclaim_bytes);
        self.top.push(group);
    }

    fn into_selection(self) -> DuplicateGroupSelection {
        DuplicateGroupSelection {
            total_groups: self.total_groups,
            total_files: self.total_files,
            conservative_reclaim_bytes: self.conservative_reclaim_bytes,
            groups: self.top.into_sorted_groups(),
        }
    }
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

#[derive(Debug)]
struct BoundedDuplicateGroups {
    top: BoundedHeap<DuplicateFileGroup, DuplicateGroupRank>,
}

impl BoundedDuplicateGroups {
    fn new(limit: usize) -> Self {
        Self {
            top: BoundedHeap::new(limit),
        }
    }

    fn push(&mut self, group: DuplicateFileGroup) {
        let rank = DuplicateGroupRank::from_group(&group);
        self.top.push(group, rank);
    }

    fn into_sorted_groups(self) -> Vec<DuplicateFileGroup> {
        self.top.into_sorted_entries()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct DuplicateGroupRank {
    conservative_reclaim_bytes: u64,
    size_bytes: u64,
    reverse_path: Reverse<PathBuf>,
}

impl DuplicateGroupRank {
    fn from_group(group: &DuplicateFileGroup) -> Self {
        Self {
            conservative_reclaim_bytes: group.conservative_reclaim_bytes,
            size_bytes: group.size_bytes,
            reverse_path: Reverse(group.files[0].path.clone()),
        }
    }
}

#[derive(Debug)]
struct BoundedLintFiles {
    top: BoundedHeap<LintFileEntry, LintEntryRank>,
}

impl BoundedLintFiles {
    fn new(limit: usize) -> Self {
        Self {
            top: BoundedHeap::new(limit),
        }
    }

    fn push(&mut self, entry: LintFileEntry, rank: LintEntryRank) {
        self.top.push(entry, rank);
    }

    fn into_sorted_entries(self) -> Vec<LintFileEntry> {
        self.top.into_sorted_entries()
    }
}

#[derive(Debug)]
struct BoundedLintDirectories {
    top: BoundedHeap<LintDirectoryEntry, LintEntryRank>,
}

impl BoundedLintDirectories {
    fn new(limit: usize) -> Self {
        Self {
            top: BoundedHeap::new(limit),
        }
    }

    fn push(&mut self, entry: LintDirectoryEntry, rank: LintEntryRank) {
        self.top.push(entry, rank);
    }

    fn into_sorted_entries(self) -> Vec<LintDirectoryEntry> {
        self.top.into_sorted_entries()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct LintEntryRank {
    primary: u64,
    reverse_path: Reverse<PathBuf>,
}

#[derive(Debug)]
struct BoundedHeap<Entry, Rank> {
    limit: usize,
    heap: BinaryHeap<Reverse<BoundedHeapItem<Entry, Rank>>>,
    sequence: u64,
}

impl<Entry, Rank> BoundedHeap<Entry, Rank>
where
    Rank: Ord,
{
    fn new(limit: usize) -> Self {
        Self {
            limit,
            heap: BinaryHeap::with_capacity(limit),
            sequence: 0,
        }
    }

    fn push(&mut self, entry: Entry, rank: Rank) {
        if self.limit == 0 {
            return;
        }

        let item = BoundedHeapItem {
            rank,
            sequence: self.sequence,
            entry,
        };
        self.sequence = self.sequence.saturating_add(1);

        if self.heap.len() < self.limit {
            self.heap.push(Reverse(item));
            return;
        }

        if self.heap.peek().is_some_and(|current| item > current.0) {
            self.heap.pop();
            self.heap.push(Reverse(item));
        }
    }

    fn into_sorted_entries(self) -> Vec<Entry> {
        let mut items = self
            .heap
            .into_iter()
            .map(|Reverse(item)| item)
            .collect::<Vec<_>>();
        items.sort_by(|left, right| right.cmp(left));
        items.into_iter().map(|item| item.entry).collect()
    }
}

#[derive(Debug)]
struct BoundedHeapItem<Entry, Rank> {
    rank: Rank,
    sequence: u64,
    entry: Entry,
}

impl<Entry, Rank> Ord for BoundedHeapItem<Entry, Rank>
where
    Rank: Ord,
{
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.rank
            .cmp(&other.rank)
            .then_with(|| other.sequence.cmp(&self.sequence))
    }
}

impl<Entry, Rank> PartialOrd for BoundedHeapItem<Entry, Rank>
where
    Rank: Ord,
{
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<Entry, Rank> PartialEq for BoundedHeapItem<Entry, Rank>
where
    Rank: PartialEq,
{
    fn eq(&self, other: &Self) -> bool {
        self.rank == other.rank && self.sequence == other.sequence
    }
}

impl<Entry, Rank> Eq for BoundedHeapItem<Entry, Rank> where Rank: Eq {}

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

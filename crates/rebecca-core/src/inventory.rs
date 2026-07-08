use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};

use crate::error::{RebeccaError, Result};
use crate::path_overlap::{path_is_same_or_child, paths_overlap};
use crate::safety::is_reparse_like;
use crate::scan::ScanCancellationToken;

#[derive(Debug, Clone)]
pub struct InventoryRequest {
    pub roots: Vec<PathBuf>,
    pub reference_roots: Vec<PathBuf>,
    pub protected_roots: Vec<PathBuf>,
    pub exclude_paths: Vec<PathBuf>,
}

impl InventoryRequest {
    pub fn new(roots: Vec<PathBuf>) -> Self {
        Self {
            roots,
            reference_roots: Vec::new(),
            protected_roots: Vec::new(),
            exclude_paths: Vec::new(),
        }
    }

    pub fn with_reference_roots(mut self, reference_roots: Vec<PathBuf>) -> Self {
        self.reference_roots = reference_roots;
        self
    }

    pub fn with_protected_roots(mut self, protected_roots: Vec<PathBuf>) -> Self {
        self.protected_roots = protected_roots;
        self
    }

    pub fn with_exclude_paths(mut self, exclude_paths: Vec<PathBuf>) -> Self {
        self.exclude_paths = exclude_paths;
        self
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inventory {
    pub roots: Vec<InventoryRoot>,
    pub files: Vec<InventoryFile>,
    pub directories: Vec<InventoryDirectory>,
    pub diagnostics: Vec<InventoryDiagnostic>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InventoryMetrics {
    pub logical_bytes: u64,
    pub allocated_bytes: Option<u64>,
    pub unique_logical_bytes: Option<u64>,
    pub unique_allocated_bytes: Option<u64>,
    pub files: u64,
    pub directories: u64,
}

impl InventoryMetrics {
    pub(crate) fn add(&mut self, other: Self) {
        self.logical_bytes = self.logical_bytes.saturating_add(other.logical_bytes);
        self.allocated_bytes = add_optional_bytes(
            self.allocated_bytes,
            self.files,
            other.allocated_bytes,
            other.files,
        );
        self.unique_logical_bytes = add_optional_bytes(
            self.unique_logical_bytes,
            self.files,
            other.unique_logical_bytes,
            other.files,
        );
        self.unique_allocated_bytes = add_optional_bytes(
            self.unique_allocated_bytes,
            self.files,
            other.unique_allocated_bytes,
            other.files,
        );
        self.files = self.files.saturating_add(other.files);
        self.directories = self.directories.saturating_add(other.directories);
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InventorySortField {
    #[default]
    Logical,
    Allocated,
    Files,
    Unique,
}

impl InventorySortField {
    pub fn label(self) -> &'static str {
        match self {
            Self::Logical => "logical",
            Self::Allocated => "allocated",
            Self::Files => "files",
            Self::Unique => "unique",
        }
    }

    pub(crate) fn metrics_value(self, metrics: &InventoryMetrics) -> u64 {
        self.value(
            metrics.logical_bytes,
            metrics.allocated_bytes,
            metrics.unique_logical_bytes,
            metrics.files,
        )
    }

    pub(crate) fn value(
        self,
        logical_bytes: u64,
        allocated_bytes: Option<u64>,
        unique_logical_bytes: Option<u64>,
        files: u64,
    ) -> u64 {
        match self {
            Self::Logical => logical_bytes,
            Self::Allocated => allocated_bytes.unwrap_or(logical_bytes),
            Self::Files => files,
            Self::Unique => unique_logical_bytes.unwrap_or(logical_bytes),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InventoryEntryKind {
    File,
    Directory,
    Other,
}

impl InventoryEntryKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Directory => "directory",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InventoryGroupKind {
    Type,
    Extension,
    Depth,
    Age,
}

impl InventoryGroupKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Type => "type",
            Self::Extension => "extension",
            Self::Depth => "depth",
            Self::Age => "age",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InventoryGroup {
    pub kind: InventoryGroupKind,
    pub key: String,
    pub label: String,
    pub metrics: InventoryMetrics,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InventoryRoot {
    pub path: PathBuf,
    pub status: InventoryRootStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InventoryRootStatus {
    Scanned,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InventoryFile {
    pub path: PathBuf,
    pub root: PathBuf,
    pub size_bytes: u64,
    pub modified_at_unix_seconds: Option<u64>,
    pub identity: Option<FileIdentity>,
    pub role: InventoryEntryRole,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InventoryDirectory {
    pub path: PathBuf,
    pub root: PathBuf,
    pub depth: usize,
    pub role: InventoryEntryRole,
    pub is_empty: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InventoryEntryRole {
    Scanned,
    Reference,
    Protected,
}

impl InventoryEntryRole {
    pub fn label(self) -> &'static str {
        match self {
            Self::Scanned => "scanned",
            Self::Reference => "reference",
            Self::Protected => "protected",
        }
    }

    pub fn is_keep_candidate(self) -> bool {
        matches!(self, Self::Reference | Self::Protected)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct FileIdentity {
    pub device: u64,
    pub inode: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct InventoryDiagnostic {
    pub kind: InventoryDiagnosticKind,
    pub path: PathBuf,
    pub detail: String,
}

impl InventoryDiagnostic {
    pub fn new(kind: InventoryDiagnosticKind, path: PathBuf, detail: impl Into<String>) -> Self {
        Self {
            kind,
            path,
            detail: detail.into(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InventoryDiagnosticSummary {
    pub total: u64,
    pub retained: u64,
    pub truncated: u64,
    pub by_kind: Vec<InventoryDiagnosticKindSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InventoryDiagnosticKindSummary {
    pub kind: InventoryDiagnosticKind,
    pub count: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InventoryDiagnosticKind {
    RootMissing,
    RootMetadataReadSkipped,
    RootNotDirectory,
    ReparsePointSkipped,
    DirectoryReadSkipped,
    DirectoryEntryReadSkipped,
    MetadataReadSkipped,
    Fallback,
    ScanFailed,
    Excluded,
}

impl InventoryDiagnosticKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::RootMissing => "root-missing",
            Self::RootMetadataReadSkipped => "root-metadata-read-skipped",
            Self::RootNotDirectory => "root-not-directory",
            Self::ReparsePointSkipped => "reparse-point-skipped",
            Self::DirectoryReadSkipped => "directory-read-skipped",
            Self::DirectoryEntryReadSkipped => "directory-entry-read-skipped",
            Self::MetadataReadSkipped => "metadata-read-skipped",
            Self::Fallback => "fallback",
            Self::ScanFailed => "scan-failed",
            Self::Excluded => "excluded",
        }
    }
}

pub fn build_inventory(
    request: &InventoryRequest,
    cancellation: &ScanCancellationToken,
) -> Result<Inventory> {
    let mut inventory = Inventory::default();

    for root in &request.roots {
        check_cancelled(cancellation)?;
        scan_root(root, request, cancellation, &mut inventory)?;
    }

    inventory
        .files
        .sort_by(|left, right| left.path.cmp(&right.path));
    inventory
        .directories
        .sort_by(|left, right| left.path.cmp(&right.path));
    inventory.diagnostics.sort();
    Ok(inventory)
}

fn scan_root(
    root: &Path,
    request: &InventoryRequest,
    cancellation: &ScanCancellationToken,
    inventory: &mut Inventory,
) -> Result<()> {
    let metadata = match std::fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            push_root_skip(
                inventory,
                root,
                InventoryDiagnosticKind::RootMissing,
                "inventory root does not exist",
            );
            return Ok(());
        }
        Err(err) => {
            push_root_skip(
                inventory,
                root,
                InventoryDiagnosticKind::RootMetadataReadSkipped,
                format!("inventory root metadata could not be read: {err}"),
            );
            return Ok(());
        }
    };

    if !metadata.is_dir() {
        push_root_skip(
            inventory,
            root,
            InventoryDiagnosticKind::RootNotDirectory,
            "inventory root is not a directory",
        );
        return Ok(());
    }

    if is_reparse_like(&metadata) {
        push_root_skip(
            inventory,
            root,
            InventoryDiagnosticKind::ReparsePointSkipped,
            "inventory root is a symlink or reparse point",
        );
        return Ok(());
    }

    inventory.roots.push(InventoryRoot {
        path: root.to_path_buf(),
        status: InventoryRootStatus::Scanned,
        reason: None,
    });

    let walker = inventory_walk_builder(root).build();
    for entry in walker {
        check_cancelled(cancellation)?;
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                inventory.diagnostics.push(InventoryDiagnostic::new(
                    InventoryDiagnosticKind::DirectoryEntryReadSkipped,
                    root.to_path_buf(),
                    format!("inventory directory entry could not be read: {err}"),
                ));
                continue;
            }
        };

        let path = entry.path();
        if path == root {
            continue;
        }

        if request
            .exclude_paths
            .iter()
            .any(|excluded| path_is_same_or_child(excluded, path))
        {
            inventory.diagnostics.push(InventoryDiagnostic::new(
                InventoryDiagnosticKind::Excluded,
                path.to_path_buf(),
                "inventory entry matched an excluded path",
            ));
            continue;
        }

        let metadata = match std::fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(err) => {
                inventory.diagnostics.push(InventoryDiagnostic::new(
                    InventoryDiagnosticKind::MetadataReadSkipped,
                    path.to_path_buf(),
                    format!("inventory entry metadata could not be read: {err}"),
                ));
                continue;
            }
        };

        if is_reparse_like(&metadata) {
            inventory.diagnostics.push(InventoryDiagnostic::new(
                InventoryDiagnosticKind::ReparsePointSkipped,
                path.to_path_buf(),
                "inventory entry is a symlink or reparse point",
            ));
            continue;
        }

        let role = entry_role(path, request);
        if metadata.is_file() {
            inventory.files.push(InventoryFile {
                path: path.to_path_buf(),
                root: root.to_path_buf(),
                size_bytes: metadata.len(),
                modified_at_unix_seconds: modified_at_unix_seconds(&metadata),
                identity: file_identity(&metadata),
                role,
            });
        } else if metadata.is_dir() {
            inventory.directories.push(InventoryDirectory {
                path: path.to_path_buf(),
                root: root.to_path_buf(),
                depth: path_depth_from_root(root, path),
                role,
                is_empty: directory_is_empty(path, request),
            });
        }
    }

    Ok(())
}

fn inventory_walk_builder(path: &Path) -> WalkBuilder {
    let mut builder = WalkBuilder::new(path);
    builder.standard_filters(false).follow_links(false);
    builder
}

fn push_root_skip(
    inventory: &mut Inventory,
    root: &Path,
    kind: InventoryDiagnosticKind,
    detail: impl Into<String>,
) {
    let detail = detail.into();
    inventory.roots.push(InventoryRoot {
        path: root.to_path_buf(),
        status: InventoryRootStatus::Skipped,
        reason: Some(detail.clone()),
    });
    inventory
        .diagnostics
        .push(InventoryDiagnostic::new(kind, root.to_path_buf(), detail));
}

fn entry_role(path: &Path, request: &InventoryRequest) -> InventoryEntryRole {
    if request
        .protected_roots
        .iter()
        .any(|protected| paths_overlap(path, protected))
    {
        return InventoryEntryRole::Protected;
    }

    if request
        .reference_roots
        .iter()
        .any(|reference| paths_overlap(path, reference))
    {
        return InventoryEntryRole::Reference;
    }

    InventoryEntryRole::Scanned
}

fn directory_is_empty(path: &Path, request: &InventoryRequest) -> bool {
    let Ok(entries) = std::fs::read_dir(path) else {
        return false;
    };

    for entry in entries {
        let Ok(entry) = entry else {
            return false;
        };
        let entry_path = entry.path();
        if request
            .exclude_paths
            .iter()
            .any(|excluded| path_is_same_or_child(excluded, &entry_path))
        {
            continue;
        }
        return false;
    }

    true
}

fn path_depth_from_root(root: &Path, path: &Path) -> usize {
    path.strip_prefix(root)
        .map(|relative| relative.components().count())
        .unwrap_or_else(|_| path.components().count())
}

fn modified_at_unix_seconds(metadata: &std::fs::Metadata) -> Option<u64> {
    metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
}

#[cfg(unix)]
fn file_identity(metadata: &std::fs::Metadata) -> Option<FileIdentity> {
    use std::os::unix::fs::MetadataExt;

    Some(FileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

#[cfg(not(unix))]
fn file_identity(_metadata: &std::fs::Metadata) -> Option<FileIdentity> {
    None
}

fn add_optional_bytes(
    left: Option<u64>,
    left_files: u64,
    right: Option<u64>,
    right_files: u64,
) -> Option<u64> {
    if right_files == 0 {
        return left;
    }

    match (left, right) {
        (None, Some(right)) if left_files == 0 => Some(right),
        (Some(left), Some(right)) => Some(left.saturating_add(right)),
        _ => None,
    }
}

fn check_cancelled(cancellation: &ScanCancellationToken) -> Result<()> {
    if cancellation.is_cancelled() {
        return Err(RebeccaError::OperationCancelled(
            "inventory scan was cancelled".to_string(),
        ));
    }

    Ok(())
}

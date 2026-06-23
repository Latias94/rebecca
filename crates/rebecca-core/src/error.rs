use std::path::{Path, PathBuf};

use thiserror::Error;

pub type Result<T> = std::result::Result<T, RebeccaError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanFailureKind {
    NotFound,
    PermissionDenied,
    InvalidInput,
    DirectoryLoop,
    MetadataUnavailable,
    DirectoryTraversal,
}

impl ScanFailureKind {
    fn label(self) -> &'static str {
        match self {
            Self::NotFound => "not-found",
            Self::PermissionDenied => "permission-denied",
            Self::InvalidInput => "invalid-input",
            Self::DirectoryLoop => "directory-loop",
            Self::MetadataUnavailable => "metadata-unavailable",
            Self::DirectoryTraversal => "directory-traversal",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanFailurePhase {
    RootMetadata,
    DirectoryWalk,
    EntryMetadata,
}

impl ScanFailurePhase {
    fn label(self) -> &'static str {
        match self {
            Self::RootMetadata => "root-metadata",
            Self::DirectoryWalk => "directory-walk",
            Self::EntryMetadata => "entry-metadata",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanFailure {
    pub kind: ScanFailureKind,
    pub phase: ScanFailurePhase,
    pub path: PathBuf,
    pub message: String,
}

impl ScanFailure {
    pub fn from_io(path: &Path, phase: ScanFailurePhase, err: &std::io::Error) -> Self {
        Self {
            kind: classify_io_error(err),
            phase,
            path: path.to_path_buf(),
            message: err.to_string(),
        }
    }

    pub fn directory_walk(path: &Path, err: &ignore::Error) -> Self {
        Self::from_ignore(path, ScanFailurePhase::DirectoryWalk, err)
    }

    pub fn from_ignore(path: &Path, phase: ScanFailurePhase, err: &ignore::Error) -> Self {
        let kind = classify_ignore_error(err).unwrap_or(match phase {
            ScanFailurePhase::DirectoryWalk => ScanFailureKind::DirectoryTraversal,
            ScanFailurePhase::EntryMetadata | ScanFailurePhase::RootMetadata => {
                ScanFailureKind::MetadataUnavailable
            }
        });

        Self {
            kind,
            phase,
            path: ignore_error_path(err).unwrap_or(path).to_path_buf(),
            message: err.to_string(),
        }
    }
}

impl std::fmt::Display for ScanFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{} during {} at {}: {}",
            self.kind.label(),
            self.phase.label(),
            self.path.display(),
            self.message
        )
    }
}

fn classify_io_error(err: &std::io::Error) -> ScanFailureKind {
    match err.kind() {
        std::io::ErrorKind::NotFound => ScanFailureKind::NotFound,
        std::io::ErrorKind::PermissionDenied => ScanFailureKind::PermissionDenied,
        std::io::ErrorKind::InvalidInput => ScanFailureKind::InvalidInput,
        _ => ScanFailureKind::MetadataUnavailable,
    }
}

fn classify_ignore_error(err: &ignore::Error) -> Option<ScanFailureKind> {
    match err {
        ignore::Error::Partial(errors) if errors.len() == 1 => {
            errors.first().and_then(classify_ignore_error)
        }
        ignore::Error::WithLineNumber { err, .. }
        | ignore::Error::WithPath { err, .. }
        | ignore::Error::WithDepth { err, .. } => classify_ignore_error(err),
        ignore::Error::Loop { .. } => Some(ScanFailureKind::DirectoryLoop),
        ignore::Error::Io(err) => Some(classify_io_error(err)),
        _ => None,
    }
}

fn ignore_error_path(err: &ignore::Error) -> Option<&Path> {
    match err {
        ignore::Error::Partial(errors) if errors.len() == 1 => {
            errors.first().and_then(ignore_error_path)
        }
        ignore::Error::WithLineNumber { err, .. } | ignore::Error::WithDepth { err, .. } => {
            ignore_error_path(err)
        }
        ignore::Error::WithPath { path, .. } => Some(path),
        ignore::Error::Loop { child, .. } => Some(child),
        _ => None,
    }
}

#[derive(Debug, Error)]
pub enum RebeccaError {
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("toml error: {0}")]
    Toml(#[from] toml::de::Error),

    #[error("could not locate the current user's standard directories")]
    UserDirsUnavailable,

    #[error("invalid rule id: {0}")]
    InvalidRuleId(String),

    #[error("invalid rule catalog: {0}")]
    RuleCatalogInvalid(String),

    #[error("path template expansion failed: {0}")]
    PathExpansionFailed(String),

    #[error("cleanup target was blocked by safety policy: {0}")]
    SafetyBlocked(String),

    #[error("scan failed: {0}")]
    ScanFailed(ScanFailure),

    #[error("operation cancelled: {0}")]
    OperationCancelled(String),

    #[error("platform feature is not available: {0}")]
    PlatformUnavailable(String),

    #[error("history is unavailable: {0}")]
    HistoryUnavailable(String),

    #[error("history record was corrupted: {0}")]
    HistoryCorrupted(String),

    #[error("cleanup execution failed: {0}")]
    ExecutionFailed(String),
}

#[cfg(test)]
mod tests {
    use super::{ScanFailure, ScanFailureKind, ScanFailurePhase};

    #[test]
    fn ignore_error_failure_keeps_nested_path_and_io_kind() {
        let root = std::path::Path::new("root");
        let nested = std::path::PathBuf::from("root/cache");
        let err = ignore::Error::WithPath {
            path: nested.clone(),
            err: Box::new(ignore::Error::Io(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "denied",
            ))),
        };

        let failure = ScanFailure::from_ignore(root, ScanFailurePhase::DirectoryWalk, &err);

        assert_eq!(failure.kind, ScanFailureKind::PermissionDenied);
        assert_eq!(failure.phase, ScanFailurePhase::DirectoryWalk);
        assert_eq!(failure.path, nested);
    }
}

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

mod catalog;
mod context;
mod definitions;
mod discovery;
mod policy;

pub(crate) use self::catalog::project_artifact_policy_matches_selectors;
pub use self::catalog::{
    all_project_artifact_definitions, project_artifact_definition_for_dir_name,
    project_artifact_definitions, project_artifact_matches_selectors,
    validate_project_artifact_selectors,
};
pub use self::discovery::{
    discover_project_artifacts, discover_project_artifacts_with_diagnostics,
    recently_modified_reason,
};
pub use self::policy::{
    ProjectArtifactPolicy, ProjectArtifactRanking, all_project_artifact_policies,
    policy_for_rule_id,
};

use crate::model::DEFAULT_PROJECT_ARTIFACT_MAX_DEPTH;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProjectArtifactDefinition {
    pub directory_name: &'static str,
    pub rule_id: &'static str,
    pub restore_hint: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectArtifactContextMatch {
    pub matched_context: String,
    pub project_root: PathBuf,
    pub project_anchor: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectArtifactCandidate {
    pub definition: ProjectArtifactDefinition,
    pub policy: &'static ProjectArtifactPolicy,
    pub path: PathBuf,
    pub context: ProjectArtifactContextMatch,
    pub modified_at_unix_seconds: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProjectArtifactDiscoveryReport {
    pub candidates: Vec<ProjectArtifactCandidate>,
    pub diagnostics: Vec<ProjectArtifactDiscoveryDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ProjectArtifactDiscoveryDiagnostic {
    pub kind: ProjectArtifactDiscoveryDiagnosticKind,
    pub path: PathBuf,
    pub detail: String,
}

impl ProjectArtifactDiscoveryDiagnostic {
    pub fn new(
        kind: ProjectArtifactDiscoveryDiagnosticKind,
        path: PathBuf,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            path,
            detail: detail.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProjectArtifactDiscoveryDiagnosticKind {
    RootMissing,
    RootMetadataReadSkipped,
    RootNotDirectory,
    ReparsePointSkipped,
    DirectoryReadSkipped,
    DirectoryEntryReadSkipped,
    MetadataReadSkipped,
}

impl ProjectArtifactDiscoveryDiagnosticKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::RootMissing => "root-missing",
            Self::RootMetadataReadSkipped => "root-metadata-read-skipped",
            Self::RootNotDirectory => "root-not-directory",
            Self::ReparsePointSkipped => "reparse-point-skipped",
            Self::DirectoryReadSkipped => "directory-read-skipped",
            Self::DirectoryEntryReadSkipped => "directory-entry-read-skipped",
            Self::MetadataReadSkipped => "metadata-read-skipped",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectArtifactScanOptions {
    pub roots: Vec<PathBuf>,
    pub max_depth: usize,
}

impl ProjectArtifactScanOptions {
    pub fn new(roots: Vec<PathBuf>) -> Self {
        Self {
            roots,
            max_depth: DEFAULT_PROJECT_ARTIFACT_MAX_DEPTH,
        }
    }

    pub fn with_max_depth(mut self, max_depth: usize) -> Self {
        self.max_depth = max_depth;
        self
    }
}

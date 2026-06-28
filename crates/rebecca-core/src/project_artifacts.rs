use std::path::PathBuf;

mod catalog;
mod discovery;

pub use self::catalog::{
    all_project_artifact_definitions, project_artifact_definition_for_dir_name,
    project_artifact_definitions, project_artifact_matches_selectors,
    validate_project_artifact_selectors,
};
pub use self::discovery::{discover_project_artifacts, recently_modified_reason};

use crate::model::DEFAULT_PROJECT_ARTIFACT_MAX_DEPTH;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProjectArtifactDefinition {
    pub directory_name: &'static str,
    pub rule_id: &'static str,
    pub restore_hint: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectArtifactCandidate {
    pub definition: ProjectArtifactDefinition,
    pub path: PathBuf,
    pub modified_at_unix_seconds: Option<u64>,
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

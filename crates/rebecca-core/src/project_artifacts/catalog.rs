use std::sync::OnceLock;

use crate::error::{RebeccaError, Result};

use super::policy::{
    CACHEDIR_TAG_POLICY, PROJECT_ARTIFACT_POLICIES, all_project_artifact_policies,
    policy_for_definition, policy_for_dir_name, policy_matches_selector,
};
use super::{ProjectArtifactDefinition, ProjectArtifactPolicy};

const PRUNED_SCAN_DIRS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    ".trash",
    "$recycle.bin",
    "library",
    "applications",
];

pub fn project_artifact_definitions() -> &'static [ProjectArtifactDefinition] {
    static DEFINITIONS: OnceLock<Vec<ProjectArtifactDefinition>> = OnceLock::new();

    DEFINITIONS
        .get_or_init(|| {
            PROJECT_ARTIFACT_POLICIES
                .iter()
                .map(|policy| policy.definition)
                .collect()
        })
        .as_slice()
}

pub fn all_project_artifact_definitions() -> impl Iterator<Item = ProjectArtifactDefinition> {
    project_artifact_definitions()
        .iter()
        .copied()
        .chain([CACHEDIR_TAG_POLICY.definition])
}

pub fn project_artifact_definition_for_dir_name(name: &str) -> Option<ProjectArtifactDefinition> {
    policy_for_dir_name(name).map(|policy| policy.definition)
}

pub fn validate_project_artifact_selectors(selectors: &[String]) -> Result<()> {
    for selector in selectors {
        if selector.trim().is_empty() {
            return Err(RebeccaError::InvalidProjectArtifactSelector(
                "selector cannot be empty".to_string(),
            ));
        }

        let known =
            all_project_artifact_policies().any(|policy| policy_matches_selector(policy, selector));
        if !known {
            return Err(RebeccaError::InvalidProjectArtifactSelector(
                selector.clone(),
            ));
        }
    }

    Ok(())
}

pub fn project_artifact_matches_selectors(
    definition: ProjectArtifactDefinition,
    selectors: &[String],
) -> bool {
    let Some(policy) = policy_for_definition(definition) else {
        return false;
    };
    project_artifact_policy_matches_selectors(policy, selectors)
}

pub(crate) fn project_artifact_policy_matches_selectors(
    policy: &ProjectArtifactPolicy,
    selectors: &[String],
) -> bool {
    selectors.is_empty()
        || selectors
            .iter()
            .any(|selector| policy_matches_selector(policy, selector))
}

pub(super) fn policy_for_directory_name(name: &str) -> Option<&'static ProjectArtifactPolicy> {
    policy_for_dir_name(name)
}

pub(super) fn is_known_project_artifact_dir_name(name: &str) -> bool {
    policy_for_dir_name(name).is_some()
}

pub(super) fn should_prune_scan_dir(name: &str) -> bool {
    PRUNED_SCAN_DIRS
        .iter()
        .any(|pruned| pruned.eq_ignore_ascii_case(name))
}

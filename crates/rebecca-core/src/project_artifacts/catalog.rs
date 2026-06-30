use std::path::Path;
use std::sync::OnceLock;

use crate::error::{RebeccaError, Result};

use super::context::project_artifact_context_match;
use super::definitions::{CACHEDIR_TAG_DEFINITION, PROJECT_ARTIFACT_RULES, ProjectArtifactRule};
use super::{ProjectArtifactContextMatch, ProjectArtifactDefinition};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ProjectArtifactRuleMatch {
    pub(super) definition: ProjectArtifactDefinition,
    pub(super) context: ProjectArtifactContextMatch,
}

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
            PROJECT_ARTIFACT_RULES
                .iter()
                .map(|rule| rule.definition)
                .collect()
        })
        .as_slice()
}

pub fn all_project_artifact_definitions() -> impl Iterator<Item = ProjectArtifactDefinition> {
    project_artifact_definitions()
        .iter()
        .copied()
        .chain([CACHEDIR_TAG_DEFINITION])
}

pub fn project_artifact_definition_for_dir_name(name: &str) -> Option<ProjectArtifactDefinition> {
    project_artifact_rule_for_dir_name(name).map(|rule| rule.definition)
}

pub fn validate_project_artifact_selectors(selectors: &[String]) -> Result<()> {
    for selector in selectors {
        if selector.trim().is_empty() {
            return Err(RebeccaError::InvalidProjectArtifactSelector(
                "selector cannot be empty".to_string(),
            ));
        }

        let known = all_project_artifact_definitions()
            .any(|definition| project_artifact_matches_selector(definition, selector));
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
    selectors.is_empty()
        || selectors
            .iter()
            .any(|selector| project_artifact_matches_selector(definition, selector))
}

pub(super) fn rule_match_for_directory(dir: &Path, name: &str) -> Option<ProjectArtifactRuleMatch> {
    let rule = project_artifact_rule_for_dir_name(name)?;
    let context = project_artifact_context_match(dir, rule.context)?;
    Some(ProjectArtifactRuleMatch {
        definition: rule.definition,
        context,
    })
}

pub(super) fn cachedir_tag_definition() -> ProjectArtifactDefinition {
    CACHEDIR_TAG_DEFINITION
}

pub(super) fn is_known_project_artifact_dir_name(name: &str) -> bool {
    project_artifact_rule_for_dir_name(name).is_some()
}

pub(super) fn should_prune_scan_dir(name: &str) -> bool {
    PRUNED_SCAN_DIRS
        .iter()
        .any(|pruned| pruned.eq_ignore_ascii_case(name))
}

fn project_artifact_rule_for_dir_name(name: &str) -> Option<ProjectArtifactRule> {
    PROJECT_ARTIFACT_RULES
        .iter()
        .copied()
        .find(|rule| rule.definition.directory_name.eq_ignore_ascii_case(name))
}

fn project_artifact_matches_selector(
    definition: ProjectArtifactDefinition,
    selector: &str,
) -> bool {
    let selector = selector.trim();
    if selector.eq_ignore_ascii_case(definition.rule_id)
        || selector.eq_ignore_ascii_case(definition.directory_name)
    {
        return true;
    }

    let rule_suffix = definition
        .rule_id
        .strip_prefix("windows.project-artifact-")
        .unwrap_or(definition.rule_id);
    if selector.eq_ignore_ascii_case(rule_suffix) {
        return true;
    }

    selector_alias(selector) == selector_alias(definition.directory_name)
}

fn selector_alias(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace(['_', '.'], "-")
}

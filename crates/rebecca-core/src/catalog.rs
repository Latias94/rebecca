use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::project_artifacts::ProjectArtifactPolicy;
use crate::safety_catalog::{ProtectedCategoryKnowledge, WarningKind};
use crate::{RuleDefinition, SafetyLevel};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CatalogItemKind {
    CleanupRule,
    ProjectArtifact,
    Warning,
    SafetyCategory,
    ActionKind,
}

impl CatalogItemKind {
    pub fn label(&self) -> &'static str {
        match self {
            Self::CleanupRule => "cleanup-rule",
            Self::ProjectArtifact => "project-artifact",
            Self::Warning => "warning",
            Self::SafetyCategory => "safety-category",
            Self::ActionKind => "action-kind",
        }
    }
}

impl std::str::FromStr for CatalogItemKind {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "cleanup-rule" | "rule" | "rules" => Ok(Self::CleanupRule),
            "project-artifact" | "artifact" | "artifacts" => Ok(Self::ProjectArtifact),
            "warning" | "warnings" => Ok(Self::Warning),
            "safety-category" | "safety" | "category" => Ok(Self::SafetyCategory),
            "action-kind" | "action" | "actions" => Ok(Self::ActionKind),
            _ => Err(format!("unknown catalog kind {value}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum CatalogItem {
    CleanupRule(CleanupRuleCatalogItem),
    ProjectArtifact(ProjectArtifactCatalogItem),
    Warning(WarningCatalogItem),
    SafetyCategory(SafetyCategoryCatalogItem),
    ActionKind(ActionKindCatalogItem),
}

impl CatalogItem {
    pub fn kind(&self) -> CatalogItemKind {
        match self {
            Self::CleanupRule(_) => CatalogItemKind::CleanupRule,
            Self::ProjectArtifact(_) => CatalogItemKind::ProjectArtifact,
            Self::Warning(_) => CatalogItemKind::Warning,
            Self::SafetyCategory(_) => CatalogItemKind::SafetyCategory,
            Self::ActionKind(_) => CatalogItemKind::ActionKind,
        }
    }

    pub fn id(&self) -> &str {
        match self {
            Self::CleanupRule(item) => &item.id,
            Self::ProjectArtifact(item) => &item.rule_id,
            Self::Warning(item) => &item.id,
            Self::SafetyCategory(item) => &item.id,
            Self::ActionKind(item) => &item.id,
        }
    }

    pub fn matches_query(&self, query: &CatalogQuery) -> bool {
        if let Some(kind) = &query.kind
            && self.kind() != *kind
        {
            return false;
        }

        match self {
            Self::CleanupRule(item) => query.matches_cleanup_rule(item),
            Self::ProjectArtifact(item) => query.matches_project_artifact(item),
            Self::Warning(item) => query.matches_warning(item),
            Self::SafetyCategory(item) => query.matches_safety_category(item),
            Self::ActionKind(item) => query.matches_action_kind(item),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CleanupRuleCatalogItem {
    pub id: String,
    pub category: String,
    pub name: String,
    pub safety_level: SafetyLevel,
    pub restore_hint: Option<String>,
    pub warnings: Vec<String>,
    pub search_kinds: Vec<String>,
    pub targets: usize,
}

impl From<&RuleDefinition> for CleanupRuleCatalogItem {
    fn from(rule: &RuleDefinition) -> Self {
        Self {
            id: rule.id.clone(),
            category: rule.category.clone(),
            name: rule.name.clone(),
            safety_level: rule.safety_level,
            restore_hint: rule.restore_hint.clone(),
            warnings: rule.warnings.clone(),
            search_kinds: rule
                .path_templates
                .iter()
                .map(|target| target.search_kind().label().to_string())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
            targets: rule.path_templates.len(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProjectArtifactCatalogItem {
    pub artifact: String,
    pub aliases: Vec<String>,
    pub rule_id: String,
    pub rule_suffix: String,
    pub restore_hint: String,
    pub default_min_age_days: u64,
    pub trim_eligible: bool,
    pub deletion_style: String,
    pub ranking: String,
}

impl From<&ProjectArtifactPolicy> for ProjectArtifactCatalogItem {
    fn from(policy: &ProjectArtifactPolicy) -> Self {
        let definition = policy.definition;
        let rule_suffix = project_artifact_rule_suffix(definition.rule_id);
        Self {
            artifact: policy.artifact.to_string(),
            aliases: policy
                .aliases
                .iter()
                .map(|alias| alias.to_string())
                .collect(),
            rule_id: definition.rule_id.to_string(),
            rule_suffix: rule_suffix.to_string(),
            restore_hint: definition.restore_hint.to_string(),
            default_min_age_days: policy.default_min_age_days,
            trim_eligible: policy.trim_eligible,
            deletion_style: policy.deletion_style_label().to_string(),
            ranking: policy.ranking.label().to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WarningCatalogItem {
    pub id: String,
    pub description: String,
}

impl From<&WarningKind> for WarningCatalogItem {
    fn from(warning: &WarningKind) -> Self {
        Self {
            id: warning.id().to_string(),
            description: warning.description().to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SafetyCategoryCatalogItem {
    pub id: String,
    pub description: String,
}

impl From<&ProtectedCategoryKnowledge> for SafetyCategoryCatalogItem {
    fn from(category: &ProtectedCategoryKnowledge) -> Self {
        Self {
            id: category.id().label().to_string(),
            description: category.description().to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ActionKindCatalogItem {
    pub id: String,
    pub description: String,
}

impl ActionKindCatalogItem {
    pub fn delete() -> Self {
        Self {
            id: "delete".to_string(),
            description: "Delete a resolved cleanup target through the active backend.".to_string(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CatalogQuery {
    pub kind: Option<CatalogItemKind>,
    pub categories: Vec<String>,
    pub rule_ids: Vec<String>,
    pub artifacts: Vec<String>,
    pub warnings: Vec<String>,
    pub safety_level: Option<SafetyLevel>,
}

impl CatalogQuery {
    pub fn matches_cleanup_rule(&self, item: &CleanupRuleCatalogItem) -> bool {
        matches_any(&self.categories, &item.category)
            && matches_any(&self.rule_ids, &item.id)
            && (self.warnings.is_empty()
                || self.warnings.iter().any(|warning| {
                    item.warnings
                        .iter()
                        .any(|item| item.eq_ignore_ascii_case(warning))
                }))
            && self
                .safety_level
                .is_none_or(|safety_level| item.safety_level == safety_level)
            && self.artifacts.is_empty()
    }

    pub fn matches_project_artifact(&self, item: &ProjectArtifactCatalogItem) -> bool {
        (self.artifacts.is_empty()
            || self.artifacts.iter().any(|artifact| {
                item.artifact.eq_ignore_ascii_case(artifact)
                    || item.rule_id.eq_ignore_ascii_case(artifact)
                    || item.rule_suffix.eq_ignore_ascii_case(artifact)
                    || item
                        .aliases
                        .iter()
                        .any(|alias| alias.eq_ignore_ascii_case(artifact))
            }))
            && self.categories.is_empty()
            && matches_any(&self.rule_ids, &item.rule_id)
            && self.warnings.is_empty()
            && self.safety_level.is_none()
    }

    pub fn matches_warning(&self, item: &WarningCatalogItem) -> bool {
        (if self.warnings.is_empty() && self.rule_ids.is_empty() {
            true
        } else {
            self.warnings
                .iter()
                .chain(&self.rule_ids)
                .any(|warning| item.id.eq_ignore_ascii_case(warning))
        }) && self.categories.is_empty()
            && self.artifacts.is_empty()
            && self.safety_level.is_none()
    }

    pub fn matches_safety_category(&self, item: &SafetyCategoryCatalogItem) -> bool {
        (if self.categories.is_empty() && self.rule_ids.is_empty() {
            true
        } else {
            self.categories
                .iter()
                .chain(&self.rule_ids)
                .any(|category| item.id.eq_ignore_ascii_case(category))
        }) && self.artifacts.is_empty()
            && self.warnings.is_empty()
            && self.safety_level.is_none()
    }

    pub fn matches_action_kind(&self, item: &ActionKindCatalogItem) -> bool {
        matches_any(&self.rule_ids, &item.id)
            && self.categories.is_empty()
            && self.artifacts.is_empty()
            && self.warnings.is_empty()
            && self.safety_level.is_none()
    }
}

pub fn filter_catalog_items(items: Vec<CatalogItem>, query: &CatalogQuery) -> Vec<CatalogItem> {
    let mut filtered = items
        .into_iter()
        .filter(|item| item.matches_query(query))
        .collect::<Vec<_>>();
    filtered.sort_by(|left, right| {
        left.kind()
            .label()
            .cmp(right.kind().label())
            .then_with(|| left.id().cmp(right.id()))
    });
    filtered
}

fn project_artifact_rule_suffix(rule_id: &str) -> &str {
    rule_id
        .strip_prefix("windows.project-artifact-")
        .unwrap_or(rule_id)
}

fn matches_any(selected: &[String], value: &str) -> bool {
    selected.is_empty() || selected.iter().any(|item| item.eq_ignore_ascii_case(value))
}

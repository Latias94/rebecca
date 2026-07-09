use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::app_leftovers::{AppLeftoverAdviceContext, AppLeftoverCandidate};
use crate::applications::ApplicationDiscovery;
use crate::discovery::{
    DiscoveryIndex, TargetResolution, resolve_rule_target_with_applications_and_index,
};
use crate::disk_map::{
    DiskMapMetrics, DiskMapReport, DiskMapWorkspaceInsight, DiskMapWorkspaceInsightKind,
};
use crate::environment::Environment;
use crate::error::Result;
use crate::model::{PlanRequest, RuleDefinition, SafetyLevel};
use crate::path_overlap::{PathRelation, path_relation};
use crate::project_artifacts::{ProjectArtifactCandidate, recently_modified_reason};
use crate::protection::{
    ProtectedCategory, ProtectionAssessment, ProtectionBlock, ProtectionBlockKind, ProtectionPolicy,
};

const CLEANUP_ADVICE_SAMPLE_LIMIT: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CleanupAdviceStatus {
    Cleanable,
    MaybeCleanable,
    ReviewOnly,
    ContainsCleanable,
    Protected,
    Unknown,
}

impl CleanupAdviceStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Cleanable => "cleanable",
            Self::MaybeCleanable => "maybe-cleanable",
            Self::ReviewOnly => "review-only",
            Self::ContainsCleanable => "contains-cleanable",
            Self::Protected => "protected",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CleanupAdviceSource {
    CleanupRule,
    ProjectArtifact,
    AppLeftover,
    WorkspaceInsight,
    Protection,
}

impl CleanupAdviceSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::CleanupRule => "cleanup-rule",
            Self::ProjectArtifact => "project-artifact",
            Self::AppLeftover => "app-leftover",
            Self::WorkspaceInsight => "workspace-insight",
            Self::Protection => "protection",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CleanupAdviceRelation {
    Exact,
    Descendant,
    Ancestor,
}

impl CleanupAdviceRelation {
    pub fn label(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Descendant => "descendant",
            Self::Ancestor => "ancestor",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanupAdviceCommand {
    pub command: String,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanupManualGuidance {
    pub reason: String,
    pub manual_review_hint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_tool_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanupAdviceEvidence {
    pub status: CleanupAdviceStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<CleanupAdviceSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relation: Option<CleanupAdviceRelation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub safety_level: Option<SafetyLevel>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_flags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_warnings: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protection_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matched_path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_leftover: Option<AppLeftoverAdviceContext>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggested_command: Option<CleanupAdviceCommand>,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanupAdvice {
    pub status: CleanupAdviceStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<CleanupAdviceSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relation: Option<CleanupAdviceRelation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub safety_level: Option<SafetyLevel>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_flags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_warnings: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protection_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matched_path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_leftover: Option<AppLeftoverAdviceContext>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<CleanupAdviceEvidence>,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggested_command: Option<CleanupAdviceCommand>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manual_guidance: Option<CleanupManualGuidance>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CleanupAdviceActionKind {
    RebeccaCommand,
    ManualReview,
    Protected,
}

impl CleanupAdviceActionKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::RebeccaCommand => "rebecca-command",
            Self::ManualReview => "manual-review",
            Self::Protected => "protected",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanupAdviceAction {
    pub id: String,
    pub kind: CleanupAdviceActionKind,
    pub status: CleanupAdviceStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<CleanupAdviceSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    pub owner_path: PathBuf,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sample_paths: Vec<PathBuf>,
    pub sample_path_count: u64,
    pub omitted_sample_path_count: u64,
    pub covered_path_count: u64,
    pub logical_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allocated_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unique_logical_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unique_allocated_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_flags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_warnings: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggested_command: Option<CleanupAdviceCommand>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manual_guidance: Option<CleanupManualGuidance>,
    pub reason: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanupAdviceActionSummary {
    pub total_items: u64,
    pub cleanup_actions: u64,
    pub manual_review_items: u64,
    pub protected_items: u64,
    pub cleanable_actions: u64,
    pub maybe_cleanable_actions: u64,
    pub contains_cleanable_actions: u64,
    pub cleanable_logical_bytes: u64,
    pub maybe_cleanable_logical_bytes: u64,
    pub contains_cleanable_logical_bytes: u64,
    pub manual_review_logical_bytes: u64,
    pub protected_logical_bytes: u64,
}

impl CleanupAdviceActionSummary {
    pub fn is_empty(&self) -> bool {
        self.total_items == 0
    }

    fn from_items(
        cleanup_actions: &[CleanupAdviceAction],
        manual_review_items: &[CleanupAdviceAction],
    ) -> Self {
        let mut summary = Self {
            total_items: cleanup_actions
                .len()
                .saturating_add(manual_review_items.len()) as u64,
            cleanup_actions: cleanup_actions.len() as u64,
            manual_review_items: manual_review_items
                .iter()
                .filter(|action| action.kind == CleanupAdviceActionKind::ManualReview)
                .count() as u64,
            protected_items: manual_review_items
                .iter()
                .filter(|action| action.kind == CleanupAdviceActionKind::Protected)
                .count() as u64,
            ..Self::default()
        };
        for action in cleanup_actions {
            match action.status {
                CleanupAdviceStatus::Cleanable => {
                    summary.cleanable_actions += 1;
                    summary.cleanable_logical_bytes = summary
                        .cleanable_logical_bytes
                        .saturating_add(action.logical_bytes);
                }
                CleanupAdviceStatus::MaybeCleanable => {
                    summary.maybe_cleanable_actions += 1;
                    summary.maybe_cleanable_logical_bytes = summary
                        .maybe_cleanable_logical_bytes
                        .saturating_add(action.logical_bytes);
                }
                CleanupAdviceStatus::ReviewOnly => {
                    summary.manual_review_logical_bytes = summary
                        .manual_review_logical_bytes
                        .saturating_add(action.logical_bytes);
                }
                CleanupAdviceStatus::ContainsCleanable => {
                    summary.contains_cleanable_actions += 1;
                    summary.contains_cleanable_logical_bytes = summary
                        .contains_cleanable_logical_bytes
                        .saturating_add(action.logical_bytes);
                }
                CleanupAdviceStatus::Protected => {
                    summary.protected_logical_bytes = summary
                        .protected_logical_bytes
                        .saturating_add(action.logical_bytes);
                }
                CleanupAdviceStatus::Unknown => {}
            }
        }
        for action in manual_review_items {
            match action.status {
                CleanupAdviceStatus::ReviewOnly => {
                    summary.manual_review_logical_bytes = summary
                        .manual_review_logical_bytes
                        .saturating_add(action.logical_bytes);
                }
                CleanupAdviceStatus::Protected => {
                    summary.protected_logical_bytes = summary
                        .protected_logical_bytes
                        .saturating_add(action.logical_bytes);
                }
                _ => {}
            }
        }
        summary
    }
}

pub fn summarize_cleanup_advice_items(
    cleanup_actions: &[CleanupAdviceAction],
    manual_review_items: &[CleanupAdviceAction],
) -> CleanupAdviceActionSummary {
    CleanupAdviceActionSummary::from_items(cleanup_actions, manual_review_items)
}

impl CleanupAdvice {
    fn unknown() -> Self {
        Self {
            status: CleanupAdviceStatus::Unknown,
            source: None,
            relation: None,
            rule_id: None,
            category: None,
            safety_level: None,
            required_flags: Vec::new(),
            required_warnings: Vec::new(),
            protection_kind: None,
            matched_path: None,
            app_leftover: None,
            evidence: Vec::new(),
            reason: "no cleanup rule or protection policy matched this path".to_string(),
            suggested_command: None,
            manual_guidance: None,
        }
    }

    fn protected(kind: String, reason: String) -> Self {
        let evidence = CleanupAdviceEvidence {
            status: CleanupAdviceStatus::Protected,
            source: Some(CleanupAdviceSource::Protection),
            relation: None,
            rule_id: None,
            category: None,
            safety_level: None,
            required_flags: Vec::new(),
            required_warnings: Vec::new(),
            protection_kind: Some(kind.clone()),
            matched_path: None,
            app_leftover: None,
            suggested_command: None,
            reason: reason.clone(),
        };
        Self {
            status: CleanupAdviceStatus::Protected,
            source: Some(CleanupAdviceSource::Protection),
            relation: None,
            rule_id: None,
            category: None,
            safety_level: None,
            required_flags: Vec::new(),
            required_warnings: Vec::new(),
            protection_kind: Some(kind),
            matched_path: None,
            app_leftover: None,
            evidence: vec![evidence],
            reason,
            suggested_command: None,
            manual_guidance: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CleanupAdviceBuildRequest<'a> {
    plan_request: PlanRequest,
    protection_policy: ProtectionPolicy<'a>,
}

impl<'a> CleanupAdviceBuildRequest<'a> {
    pub fn new(plan_request: PlanRequest, protection_policy: ProtectionPolicy<'a>) -> Self {
        Self {
            plan_request,
            protection_policy,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CleanupAdviceIndex<'a> {
    protection_policy: ProtectionPolicy<'a>,
    targets: Vec<CleanupAdviceTarget>,
}

impl<'a> CleanupAdviceIndex<'a> {
    pub fn build<E, A>(
        request: CleanupAdviceBuildRequest<'a>,
        rules: &[RuleDefinition],
        env: &E,
        applications: &A,
    ) -> Result<Self>
    where
        E: Environment,
        A: ApplicationDiscovery + ?Sized,
    {
        let mut discovery_index = DiscoveryIndex::new();
        let mut seen = BTreeSet::new();
        let selection = request.plan_request.selection();
        selection.validate_against_rules(rules)?;
        let mut targets = Vec::new();

        for rule in rules {
            if rule.platform != request.plan_request.platform || !selection.matches_rule(rule) {
                continue;
            }

            let required_flags = required_safety_flags(&request.plan_request, rule.safety_level);
            let required_warnings = request.plan_request.missing_warning_gates(&rule.warnings);

            for spec in &rule.path_templates {
                let resolved = resolve_rule_target_with_applications_and_index(
                    spec,
                    env,
                    applications,
                    &mut discovery_index,
                )?;
                let TargetResolution::Paths(paths) = resolved else {
                    continue;
                };

                for path in paths {
                    let dedupe_key = comparable_path_key(&path);
                    if !seen.insert((rule.id.to_ascii_lowercase(), dedupe_key)) {
                        continue;
                    }

                    targets.push(CleanupAdviceTarget {
                        source: CleanupAdviceSource::CleanupRule,
                        status_override: None,
                        priority: 0,
                        impact_logical_bytes: 0,
                        path,
                        rule_id: Some(rule.id.clone()),
                        category: Some(rule.category.clone()),
                        safety_level: Some(rule.safety_level),
                        required_flags: required_flags.clone(),
                        required_warnings: required_warnings.clone(),
                        reason: None,
                        suggested_command: cleanup_rule_command(&rule.id),
                        manual_review_hint: None,
                        external_tool_hint: None,
                        app_leftover: None,
                        app_leftover_target_block: None,
                    });
                }
            }
        }

        Ok(Self {
            protection_policy: request.protection_policy,
            targets,
        })
    }

    pub fn advise_path(&self, path: &Path) -> CleanupAdvice {
        let mut candidates = self
            .targets
            .iter()
            .filter_map(|target| target.advice_for(path, self.protection_policy))
            .collect::<Vec<_>>();

        if !candidates.is_empty() {
            candidates.sort_by(|left, right| right.cmp(left));
            let primary = candidates.remove(0);
            let evidence = std::iter::once(primary.evidence.clone())
                .chain(candidates.into_iter().map(|candidate| candidate.evidence))
                .collect();
            return primary.into_advice(evidence);
        }

        match self.protection_policy.assess_path(path) {
            ProtectionAssessment::Allowed => {}
            ProtectionAssessment::Blocked(block) => {
                return CleanupAdvice::protected(block.kind.label().to_string(), block.message);
            }
        }

        CleanupAdvice::unknown()
    }

    pub fn add_project_artifact_candidates(
        &mut self,
        candidates: impl IntoIterator<Item = ProjectArtifactCandidate>,
        min_age_days: u64,
    ) {
        for candidate in candidates {
            let recent_reason = recently_modified_reason(&candidate.path, min_age_days);
            let mut required_flags = Vec::new();
            if recent_reason.is_some() {
                required_flags.push("--min-age-days 0".to_string());
            }
            let reason = recent_reason.unwrap_or_else(|| {
                format!(
                    "path is a project artifact {} anchored by {}",
                    candidate.policy.artifact,
                    candidate.context.project_anchor.display()
                )
            });

            self.targets.push(CleanupAdviceTarget {
                source: CleanupAdviceSource::ProjectArtifact,
                status_override: None,
                priority: 0,
                impact_logical_bytes: 0,
                path: candidate.path,
                rule_id: Some(candidate.definition.rule_id.to_string()),
                category: Some("project-artifact".to_string()),
                safety_level: None,
                required_flags,
                required_warnings: Vec::new(),
                reason: Some(reason),
                suggested_command: Some(CleanupAdviceCommand {
                    command: "rebecca".to_string(),
                    args: vec![
                        "purge".to_string(),
                        "--dry-run".to_string(),
                        "--root".to_string(),
                        candidate.context.project_root.display().to_string(),
                        "--artifact".to_string(),
                        candidate.policy.artifact.to_string(),
                    ],
                }),
                manual_review_hint: None,
                external_tool_hint: None,
                app_leftover: None,
                app_leftover_target_block: None,
            });
        }
    }

    pub fn add_app_leftover_candidates(
        &mut self,
        candidates: impl IntoIterator<Item = AppLeftoverCandidate>,
    ) {
        let mut seen = BTreeSet::new();
        for candidate in candidates {
            let rule_id = candidate.rule_id().to_string();
            let dedupe_key = comparable_path_key(&candidate.path);
            if !seen.insert((rule_id.to_ascii_lowercase(), dedupe_key)) {
                continue;
            }

            self.targets.push(CleanupAdviceTarget {
                source: CleanupAdviceSource::AppLeftover,
                status_override: None,
                priority: 0,
                impact_logical_bytes: 0,
                path: candidate.path.clone(),
                rule_id: Some(rule_id),
                category: Some("app-leftover".to_string()),
                safety_level: None,
                required_flags: Vec::new(),
                required_warnings: Vec::new(),
                reason: None,
                suggested_command: app_leftover_command(),
                manual_review_hint: None,
                external_tool_hint: None,
                app_leftover: Some(candidate.advice_context()),
                app_leftover_target_block: self
                    .protection_policy
                    .assess_existing_app_leftover_block(&candidate.path),
            });
        }
    }

    pub fn annotate_disk_map_report(&self, report: &mut DiskMapReport) {
        let mut index = self.clone();
        index.add_workspace_insight_candidates(
            report
                .workspace_insights
                .iter()
                .map(workspace_insight_candidate_from_disk_map),
        );

        for entry in &mut report.top_entries {
            entry.cleanup_advice = Some(index.advise_path(&entry.path));
        }

        let (actions, manual_review_items, summary) = build_disk_map_cleanup_advice_actions(report);
        report.cleanup_actions = actions;
        report.manual_review_items = manual_review_items;
        report.cleanup_advice_summary = summary;
    }

    pub fn add_workspace_insight_candidates(
        &mut self,
        candidates: impl IntoIterator<Item = WorkspaceInsightCandidate>,
    ) {
        let mut seen = BTreeSet::new();
        let mut candidates = candidates.into_iter().collect::<Vec<_>>();
        candidates.sort_by(|left, right| {
            right
                .logical_bytes
                .cmp(&left.logical_bytes)
                .then_with(|| right.priority.cmp(&left.priority))
                .then_with(|| left.path.cmp(&right.path))
        });
        for candidate in candidates {
            if !seen.insert((
                candidate.rule_id.to_string(),
                comparable_path_key(&candidate.path),
            )) {
                continue;
            }

            self.targets.push(CleanupAdviceTarget {
                source: CleanupAdviceSource::WorkspaceInsight,
                status_override: Some(CleanupAdviceStatus::ReviewOnly),
                priority: candidate.priority,
                impact_logical_bytes: candidate.logical_bytes,
                path: candidate.path,
                rule_id: Some(candidate.rule_id.to_string()),
                category: Some("workspace-review".to_string()),
                safety_level: None,
                required_flags: Vec::new(),
                required_warnings: Vec::new(),
                reason: Some(candidate.reason.to_string()),
                suggested_command: None,
                manual_review_hint: Some(candidate.manual_review_hint.to_string()),
                external_tool_hint: candidate.external_tool_hint.map(str::to_string),
                app_leftover: None,
                app_leftover_target_block: None,
            });
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceInsightCandidate {
    pub path: PathBuf,
    pub rule_id: &'static str,
    pub reason: &'static str,
    pub manual_review_hint: &'static str,
    pub external_tool_hint: Option<&'static str>,
    pub priority: u8,
    pub logical_bytes: u64,
}

#[derive(Debug, Clone)]
struct CleanupAdviceTarget {
    source: CleanupAdviceSource,
    status_override: Option<CleanupAdviceStatus>,
    priority: u8,
    impact_logical_bytes: u64,
    path: PathBuf,
    rule_id: Option<String>,
    category: Option<String>,
    safety_level: Option<SafetyLevel>,
    required_flags: Vec<String>,
    required_warnings: Vec<String>,
    reason: Option<String>,
    suggested_command: Option<CleanupAdviceCommand>,
    manual_review_hint: Option<String>,
    external_tool_hint: Option<String>,
    app_leftover: Option<AppLeftoverAdviceContext>,
    app_leftover_target_block: Option<ProtectionBlock>,
}

impl CleanupAdviceTarget {
    fn advice_for(
        &self,
        entry_path: &Path,
        protection_policy: ProtectionPolicy<'_>,
    ) -> Option<CleanupAdviceCandidate> {
        let relation = match path_relation(entry_path, &self.path) {
            PathRelation::Same => CleanupAdviceRelation::Exact,
            PathRelation::Descendant => CleanupAdviceRelation::Descendant,
            PathRelation::Ancestor => CleanupAdviceRelation::Ancestor,
            PathRelation::Unrelated => return None,
        };

        if let Some(block) = self.protection_block(entry_path, relation, protection_policy) {
            return Some(CleanupAdviceCandidate {
                rank: advice_rank(
                    CleanupAdviceStatus::Protected,
                    relation,
                    self.impact_logical_bytes,
                    self.priority,
                ),
                evidence: CleanupAdviceEvidence {
                    status: CleanupAdviceStatus::Protected,
                    source: Some(CleanupAdviceSource::Protection),
                    relation: Some(relation),
                    rule_id: None,
                    category: None,
                    safety_level: None,
                    required_flags: Vec::new(),
                    required_warnings: Vec::new(),
                    protection_kind: Some(block.kind.label().to_string()),
                    matched_path: Some(self.path.clone()),
                    app_leftover: None,
                    suggested_command: None,
                    reason: block.message,
                },
                suggested_command: None,
                manual_guidance: None,
            });
        }

        let default_status = if matches!(relation, CleanupAdviceRelation::Ancestor) {
            CleanupAdviceStatus::ContainsCleanable
        } else if self.required_flags.is_empty() && self.required_warnings.is_empty() {
            CleanupAdviceStatus::Cleanable
        } else {
            CleanupAdviceStatus::MaybeCleanable
        };
        let status = self.status_override.unwrap_or(default_status);
        let reason = self.advice_reason(status, relation);
        let manual_guidance = self.manual_guidance(status, relation, &reason);

        Some(CleanupAdviceCandidate {
            rank: advice_rank(status, relation, self.impact_logical_bytes, self.priority),
            evidence: CleanupAdviceEvidence {
                status,
                source: Some(self.source),
                relation: Some(relation),
                rule_id: self.rule_id.clone(),
                category: self.category.clone(),
                safety_level: self.safety_level,
                required_flags: self.required_flags.clone(),
                required_warnings: self.required_warnings.clone(),
                protection_kind: None,
                matched_path: Some(self.path.clone()),
                app_leftover: self.app_leftover.clone(),
                suggested_command: self.suggested_command.clone(),
                reason,
            },
            suggested_command: self.suggested_command.clone(),
            manual_guidance,
        })
    }

    fn protection_block(
        &self,
        entry_path: &Path,
        relation: CleanupAdviceRelation,
        protection_policy: ProtectionPolicy<'_>,
    ) -> Option<ProtectionBlock> {
        let assessment = match (self.source, relation) {
            (
                CleanupAdviceSource::AppLeftover,
                CleanupAdviceRelation::Exact | CleanupAdviceRelation::Descendant,
            ) => return self.app_leftover_target_block.clone(),
            (CleanupAdviceSource::AppLeftover, CleanupAdviceRelation::Ancestor) => {
                if let Some(block) = self.app_leftover_target_block.clone() {
                    return Some(block);
                }
                match protection_policy.assess_path(entry_path) {
                    ProtectionAssessment::Blocked(block)
                        if is_application_durable_data_block(&block) =>
                    {
                        ProtectionAssessment::Allowed
                    }
                    assessment => assessment,
                }
            }
            _ => protection_policy.assess_path(entry_path),
        };

        match assessment {
            ProtectionAssessment::Allowed => None,
            ProtectionAssessment::Blocked(block) => Some(block),
        }
    }

    fn advice_reason(
        &self,
        status: CleanupAdviceStatus,
        relation: CleanupAdviceRelation,
    ) -> String {
        if let Some(reason) = &self.reason
            && matches!(relation, CleanupAdviceRelation::Exact)
        {
            return reason.clone();
        }

        match self.source {
            CleanupAdviceSource::CleanupRule => {
                cleanup_rule_advice_reason(status, relation, self.rule_id.as_deref())
            }
            CleanupAdviceSource::ProjectArtifact => {
                project_artifact_advice_reason(status, relation, self.rule_id.as_deref())
            }
            CleanupAdviceSource::AppLeftover => {
                app_leftover_advice_reason(status, relation, self.app_leftover.as_ref())
            }
            CleanupAdviceSource::WorkspaceInsight => self
                .reason
                .clone()
                .unwrap_or_else(|| workspace_insight_advice_reason(relation)),
            CleanupAdviceSource::Protection => "path matched protection policy".to_string(),
        }
    }

    fn manual_guidance(
        &self,
        status: CleanupAdviceStatus,
        relation: CleanupAdviceRelation,
        reason: &str,
    ) -> Option<CleanupManualGuidance> {
        if status != CleanupAdviceStatus::ReviewOnly {
            return None;
        }

        let manual_review_hint = self
            .manual_review_hint
            .clone()
            .unwrap_or_else(|| workspace_insight_manual_review_hint(relation));
        Some(CleanupManualGuidance {
            reason: reason.to_string(),
            manual_review_hint,
            external_tool_hint: self.external_tool_hint.clone(),
            evidence_path: Some(self.path.clone()),
        })
    }
}

#[derive(Debug, Clone)]
struct CleanupAdviceCandidate {
    rank: (u8, u8, u64, u8),
    evidence: CleanupAdviceEvidence,
    suggested_command: Option<CleanupAdviceCommand>,
    manual_guidance: Option<CleanupManualGuidance>,
}

impl CleanupAdviceCandidate {
    fn into_advice(self, evidence: Vec<CleanupAdviceEvidence>) -> CleanupAdvice {
        let primary = self.evidence;
        CleanupAdvice {
            status: primary.status,
            source: primary.source,
            relation: primary.relation,
            rule_id: primary.rule_id,
            category: primary.category,
            safety_level: primary.safety_level,
            required_flags: primary.required_flags,
            required_warnings: primary.required_warnings,
            protection_kind: primary.protection_kind,
            matched_path: primary.matched_path,
            app_leftover: primary.app_leftover,
            evidence,
            reason: primary.reason,
            suggested_command: self.suggested_command,
            manual_guidance: self.manual_guidance,
        }
    }
}

impl Ord for CleanupAdviceCandidate {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.rank
            .cmp(&other.rank)
            .then_with(|| self.evidence.status.cmp(&other.evidence.status))
            .then_with(|| self.evidence.source.cmp(&other.evidence.source))
            .then_with(|| self.evidence.relation.cmp(&other.evidence.relation))
            .then_with(|| self.evidence.rule_id.cmp(&other.evidence.rule_id))
            .then_with(|| self.evidence.matched_path.cmp(&other.evidence.matched_path))
    }
}

impl PartialOrd for CleanupAdviceCandidate {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for CleanupAdviceCandidate {
    fn eq(&self, other: &Self) -> bool {
        self.rank == other.rank
            && self.evidence.status == other.evidence.status
            && self.evidence.source == other.evidence.source
            && self.evidence.relation == other.evidence.relation
            && self.evidence.rule_id == other.evidence.rule_id
            && self.evidence.matched_path == other.evidence.matched_path
    }
}

impl Eq for CleanupAdviceCandidate {}

fn advice_rank(
    status: CleanupAdviceStatus,
    relation: CleanupAdviceRelation,
    impact_logical_bytes: u64,
    priority: u8,
) -> (u8, u8, u64, u8) {
    let status_rank = match status {
        CleanupAdviceStatus::Cleanable => 5,
        CleanupAdviceStatus::MaybeCleanable => 4,
        CleanupAdviceStatus::ReviewOnly => 3,
        CleanupAdviceStatus::ContainsCleanable => 2,
        CleanupAdviceStatus::Protected => 6,
        CleanupAdviceStatus::Unknown => 0,
    };
    let relation_rank = match relation {
        CleanupAdviceRelation::Exact => 3,
        CleanupAdviceRelation::Descendant => 2,
        CleanupAdviceRelation::Ancestor => 1,
    };

    (status_rank, relation_rank, impact_logical_bytes, priority)
}

fn cleanup_rule_advice_reason(
    status: CleanupAdviceStatus,
    relation: CleanupAdviceRelation,
    rule_id: Option<&str>,
) -> String {
    let rule_id = rule_id.unwrap_or("matched rule");
    match (status, relation) {
        (CleanupAdviceStatus::Cleanable, CleanupAdviceRelation::Exact) => {
            format!("path is a direct target of cleanup rule {rule_id}")
        }
        (CleanupAdviceStatus::Cleanable, CleanupAdviceRelation::Descendant) => {
            format!("path is inside cleanup rule target {rule_id}")
        }
        (CleanupAdviceStatus::MaybeCleanable, CleanupAdviceRelation::Exact) => {
            format!("cleanup rule {rule_id} requires additional opt-in")
        }
        (CleanupAdviceStatus::MaybeCleanable, CleanupAdviceRelation::Descendant) => {
            format!(
                "path is inside cleanup rule target {rule_id}, but the rule requires additional opt-in"
            )
        }
        (CleanupAdviceStatus::ContainsCleanable, CleanupAdviceRelation::Ancestor) => {
            format!("path contains a target matched by cleanup rule {rule_id}")
        }
        _ => format!("path matched cleanup rule {rule_id}"),
    }
}

fn project_artifact_advice_reason(
    status: CleanupAdviceStatus,
    relation: CleanupAdviceRelation,
    rule_id: Option<&str>,
) -> String {
    let rule_id = rule_id.unwrap_or("project artifact policy");
    match (status, relation) {
        (CleanupAdviceStatus::Cleanable, CleanupAdviceRelation::Exact) => {
            format!("path is a direct project artifact target of {rule_id}")
        }
        (CleanupAdviceStatus::Cleanable, CleanupAdviceRelation::Descendant) => {
            format!("path is inside project artifact target {rule_id}")
        }
        (CleanupAdviceStatus::MaybeCleanable, CleanupAdviceRelation::Exact) => {
            format!("project artifact target {rule_id} requires additional opt-in")
        }
        (CleanupAdviceStatus::MaybeCleanable, CleanupAdviceRelation::Descendant) => {
            format!(
                "path is inside project artifact target {rule_id}, but the target requires additional opt-in"
            )
        }
        (CleanupAdviceStatus::ContainsCleanable, CleanupAdviceRelation::Ancestor) => {
            format!("path contains project artifact target {rule_id}")
        }
        _ => format!("path matched project artifact policy {rule_id}"),
    }
}

fn app_leftover_advice_reason(
    status: CleanupAdviceStatus,
    relation: CleanupAdviceRelation,
    context: Option<&AppLeftoverAdviceContext>,
) -> String {
    let app_name = context
        .map(|context| context.app.display_name.as_str())
        .unwrap_or("installed application");
    let leaf = context
        .map(|context| context.target_leaf.as_str())
        .filter(|leaf| !leaf.is_empty())
        .unwrap_or("cache");

    match (status, relation) {
        (CleanupAdviceStatus::Cleanable, CleanupAdviceRelation::Exact) => {
            format!("path is a rebuildable {leaf} app leftover for {app_name}")
        }
        (CleanupAdviceStatus::Cleanable, CleanupAdviceRelation::Descendant) => {
            format!("path is inside a rebuildable {leaf} app leftover for {app_name}")
        }
        (CleanupAdviceStatus::ContainsCleanable, CleanupAdviceRelation::Ancestor) => {
            format!("path contains rebuildable app leftover data for {app_name}")
        }
        _ => format!("path matched app leftover policy for {app_name}"),
    }
}

fn workspace_insight_advice_reason(relation: CleanupAdviceRelation) -> String {
    match relation {
        CleanupAdviceRelation::Exact => {
            "path needs manual review; Rebecca will not clean it automatically".to_string()
        }
        CleanupAdviceRelation::Descendant => {
            "path is inside a manual-review workspace area; Rebecca will not clean it automatically"
                .to_string()
        }
        CleanupAdviceRelation::Ancestor => {
            "path contains a manual-review workspace area; Rebecca will not clean it automatically"
                .to_string()
        }
    }
}

fn workspace_insight_manual_review_hint(relation: CleanupAdviceRelation) -> String {
    match relation {
        CleanupAdviceRelation::Exact => {
            "Open the path in your workspace, confirm ownership and regeneration cost, then remove it manually only if it is no longer needed."
                .to_string()
        }
        CleanupAdviceRelation::Descendant => {
            "This entry is inside a manual-review workspace area; review the nearest workspace owner before deleting anything."
                .to_string()
        }
        CleanupAdviceRelation::Ancestor => {
            "This entry contains manual-review workspace data; inspect the evidence path before running any cleanup command."
                .to_string()
        }
    }
}

fn cleanup_rule_command(rule_id: &str) -> Option<CleanupAdviceCommand> {
    Some(CleanupAdviceCommand {
        command: "rebecca".to_string(),
        args: vec![
            "clean".to_string(),
            "--dry-run".to_string(),
            "--rule".to_string(),
            rule_id.to_string(),
        ],
    })
}

fn app_leftover_command() -> Option<CleanupAdviceCommand> {
    Some(CleanupAdviceCommand {
        command: "rebecca".to_string(),
        args: vec![
            "apps".to_string(),
            "clean".to_string(),
            "--dry-run".to_string(),
        ],
    })
}

fn is_application_durable_data_block(block: &ProtectionBlock) -> bool {
    matches!(
        block.kind,
        ProtectionBlockKind::ProtectedCategory(ProtectedCategory::ApplicationDurableData)
    )
}

fn required_safety_flags(request: &PlanRequest, safety_level: SafetyLevel) -> Vec<String> {
    if request.allows_safety_level(safety_level) {
        Vec::new()
    } else {
        safety_level
            .opt_in_flag()
            .map(|flag| vec![flag.to_string()])
            .unwrap_or_default()
    }
}

fn comparable_path_key(path: &Path) -> String {
    let value = path.to_string_lossy().replace('\\', "/");
    if cfg!(windows) {
        value.to_ascii_lowercase()
    } else {
        value
    }
}

pub fn build_disk_map_cleanup_advice_actions(
    report: &DiskMapReport,
) -> (
    Vec<CleanupAdviceAction>,
    Vec<CleanupAdviceAction>,
    CleanupAdviceActionSummary,
) {
    let mut builder = CleanupAdviceActionBuilder::default();
    for entry in &report.top_entries {
        if let Some(advice) = entry.cleanup_advice.as_ref() {
            builder.add_entry_advice(advice, &entry.path, entry_metrics(entry));
        }
    }
    for insight in &report.workspace_insights {
        builder.add_workspace_insight(insight);
    }

    let (actions, manual_review_items) = builder.finish();
    let summary = CleanupAdviceActionSummary::from_items(&actions, &manual_review_items);
    (actions, manual_review_items, summary)
}

#[derive(Default)]
struct CleanupAdviceActionBuilder {
    actions: BTreeMap<String, CleanupAdviceActionAccumulator>,
}

impl CleanupAdviceActionBuilder {
    fn add_entry_advice(
        &mut self,
        advice: &CleanupAdvice,
        entry_path: &Path,
        metrics: DiskMapMetrics,
    ) {
        let Some(kind) = action_kind_for_advice(advice) else {
            return;
        };
        let owner_path = advice_owner_path(advice, entry_path);
        let evidence_path = advice
            .manual_guidance
            .as_ref()
            .and_then(|guidance| guidance.evidence_path.as_deref())
            .or(advice.matched_path.as_deref())
            .unwrap_or(entry_path);
        let (suggested_command, required_flags, required_warnings) =
            if kind == CleanupAdviceActionKind::RebeccaCommand {
                action_command_and_gates(advice)
            } else {
                (None, Vec::new(), Vec::new())
            };
        let id = cleanup_action_id(
            kind,
            advice.source,
            advice.rule_id.as_deref(),
            suggested_command.as_ref(),
            &owner_path,
        );
        let mut accumulator = CleanupAdviceActionAccumulator::new(
            id.clone(),
            kind,
            advice.status,
            advice.source,
            advice.rule_id.clone(),
            advice.category.clone(),
            owner_path,
            required_flags,
            required_warnings,
            suggested_command,
            advice.manual_guidance.clone(),
            advice.reason.clone(),
        );
        accumulator.add_evidence_path(evidence_path);
        if advice_can_measure_entry(advice, entry_path, &accumulator.owner_path) {
            accumulator.add_measured_path(entry_path, metrics);
        }
        self.merge_accumulator(id, accumulator);
        self.add_entry_evidence_actions(advice, entry_path, metrics);
    }

    fn add_entry_evidence_actions(
        &mut self,
        advice: &CleanupAdvice,
        entry_path: &Path,
        metrics: DiskMapMetrics,
    ) {
        for evidence in &advice.evidence {
            if !cleanup_evidence_can_be_action(evidence) {
                continue;
            }
            let Some(suggested_command) = evidence.suggested_command.clone() else {
                continue;
            };
            let owner_path = evidence
                .matched_path
                .clone()
                .unwrap_or_else(|| entry_path.to_path_buf());
            let id = cleanup_action_id(
                CleanupAdviceActionKind::RebeccaCommand,
                evidence.source,
                evidence.rule_id.as_deref(),
                Some(&suggested_command),
                &owner_path,
            );
            let mut accumulator = CleanupAdviceActionAccumulator::new(
                id.clone(),
                CleanupAdviceActionKind::RebeccaCommand,
                evidence.status,
                evidence.source,
                evidence.rule_id.clone(),
                evidence.category.clone(),
                owner_path,
                evidence.required_flags.clone(),
                evidence.required_warnings.clone(),
                Some(suggested_command),
                None,
                evidence.reason.clone(),
            );
            accumulator.add_evidence_path(evidence.matched_path.as_deref().unwrap_or(entry_path));
            if evidence_can_measure_entry(evidence, entry_path, &accumulator.owner_path) {
                accumulator.add_measured_path(entry_path, metrics);
            }
            self.merge_accumulator(id, accumulator);
        }
    }

    fn add_workspace_insight(&mut self, insight: &DiskMapWorkspaceInsight) {
        let metadata = workspace_insight_metadata(insight.kind);
        let guidance = CleanupManualGuidance {
            reason: metadata.reason.to_string(),
            manual_review_hint: metadata.manual_review_hint.to_string(),
            external_tool_hint: metadata.external_tool_hint.map(str::to_string),
            evidence_path: Some(insight.path.clone()),
        };
        let id = cleanup_action_id(
            CleanupAdviceActionKind::ManualReview,
            Some(CleanupAdviceSource::WorkspaceInsight),
            Some(metadata.rule_id),
            None,
            &insight.owner_path,
        );
        let mut accumulator = CleanupAdviceActionAccumulator::new(
            id.clone(),
            CleanupAdviceActionKind::ManualReview,
            CleanupAdviceStatus::ReviewOnly,
            Some(CleanupAdviceSource::WorkspaceInsight),
            Some(metadata.rule_id.to_string()),
            Some("workspace-review".to_string()),
            insight.owner_path.clone(),
            Vec::new(),
            Vec::new(),
            None,
            Some(guidance),
            metadata.reason.to_string(),
        );
        accumulator.add_evidence_path(&insight.path);
        accumulator.add_measured_path(&insight.path, insight.metrics);
        self.merge_accumulator(id, accumulator);
    }

    fn merge_accumulator(&mut self, id: String, accumulator: CleanupAdviceActionAccumulator) {
        if let Some(existing) = self.actions.get_mut(&id) {
            existing.merge(accumulator);
        } else {
            self.actions.insert(id, accumulator);
        }
    }

    fn finish(self) -> (Vec<CleanupAdviceAction>, Vec<CleanupAdviceAction>) {
        let mut actions = self
            .actions
            .into_values()
            .map(CleanupAdviceActionAccumulator::finish)
            .collect::<Vec<_>>();
        actions.sort_by(|left, right| {
            action_sort_rank(right)
                .cmp(&action_sort_rank(left))
                .then_with(|| right.logical_bytes.cmp(&left.logical_bytes))
                .then_with(|| left.owner_path.cmp(&right.owner_path))
                .then_with(|| left.id.cmp(&right.id))
        });
        let mut cleanup_actions = Vec::new();
        let mut manual_review_items = Vec::new();
        for action in actions {
            if action.kind == CleanupAdviceActionKind::RebeccaCommand {
                cleanup_actions.push(action);
            } else {
                manual_review_items.push(action);
            }
        }
        (cleanup_actions, manual_review_items)
    }
}

#[derive(Debug, Clone)]
struct CleanupAdviceActionAccumulator {
    id: String,
    kind: CleanupAdviceActionKind,
    status: CleanupAdviceStatus,
    source: Option<CleanupAdviceSource>,
    rule_id: Option<String>,
    category: Option<String>,
    owner_path: PathBuf,
    evidence_paths: BTreeSet<PathBuf>,
    measured_paths: Vec<(PathBuf, DiskMapMetrics)>,
    required_flags: Vec<String>,
    required_warnings: Vec<String>,
    suggested_command: Option<CleanupAdviceCommand>,
    manual_guidance: Option<CleanupManualGuidance>,
    reason: String,
}

impl CleanupAdviceActionAccumulator {
    #[allow(clippy::too_many_arguments)]
    fn new(
        id: String,
        kind: CleanupAdviceActionKind,
        status: CleanupAdviceStatus,
        source: Option<CleanupAdviceSource>,
        rule_id: Option<String>,
        category: Option<String>,
        owner_path: PathBuf,
        required_flags: Vec<String>,
        required_warnings: Vec<String>,
        suggested_command: Option<CleanupAdviceCommand>,
        manual_guidance: Option<CleanupManualGuidance>,
        reason: String,
    ) -> Self {
        Self {
            id,
            kind,
            status,
            source,
            rule_id,
            category,
            owner_path,
            evidence_paths: BTreeSet::new(),
            measured_paths: Vec::new(),
            required_flags,
            required_warnings,
            suggested_command,
            manual_guidance,
            reason,
        }
    }

    fn add_evidence_path(&mut self, path: &Path) {
        self.evidence_paths.insert(path.to_path_buf());
    }

    fn add_measured_path(&mut self, path: &Path, metrics: DiskMapMetrics) {
        if metrics.logical_bytes == 0
            && metrics.allocated_bytes.is_none()
            && metrics.unique_logical_bytes.is_none()
            && metrics.unique_allocated_bytes.is_none()
        {
            return;
        }

        let mut retained = Vec::new();
        let mut existing_paths = std::mem::take(&mut self.measured_paths).into_iter();
        while let Some((existing_path, existing_metrics)) = existing_paths.next() {
            match path_relation(path, &existing_path) {
                PathRelation::Same | PathRelation::Descendant => {
                    retained.push((existing_path, existing_metrics));
                    retained.extend(existing_paths);
                    self.measured_paths = retained;
                    return;
                }
                PathRelation::Ancestor => {}
                PathRelation::Unrelated => retained.push((existing_path, existing_metrics)),
            }
        }
        retained.push((path.to_path_buf(), metrics));
        self.measured_paths = retained;
    }

    fn merge(&mut self, other: Self) {
        self.evidence_paths.extend(other.evidence_paths);
        for (path, metrics) in other.measured_paths {
            self.add_measured_path(&path, metrics);
        }
        if cleanup_status_rank(other.status) > cleanup_status_rank(self.status) {
            self.status = other.status;
            self.source = other.source;
            self.rule_id = other.rule_id;
            self.category = other.category;
            self.reason = other.reason;
        }
        merge_strings(&mut self.required_flags, other.required_flags);
        merge_strings(&mut self.required_warnings, other.required_warnings);
        if self.suggested_command.is_none() {
            self.suggested_command = other.suggested_command;
        }
        if self.manual_guidance.is_none() {
            self.manual_guidance = other.manual_guidance;
        }
    }

    fn finish(self) -> CleanupAdviceAction {
        let metrics = sum_action_metrics(&self.measured_paths);
        let sample_path_count = self.evidence_paths.len() as u64;
        let mut sample_paths = self.evidence_paths.into_iter().collect::<Vec<_>>();
        let omitted_sample_path_count = sample_paths
            .len()
            .saturating_sub(CLEANUP_ADVICE_SAMPLE_LIMIT)
            as u64;
        sample_paths.truncate(CLEANUP_ADVICE_SAMPLE_LIMIT);
        CleanupAdviceAction {
            id: self.id,
            kind: self.kind,
            status: self.status,
            source: self.source,
            rule_id: self.rule_id,
            category: self.category,
            owner_path: self.owner_path,
            sample_paths,
            sample_path_count,
            omitted_sample_path_count,
            covered_path_count: self.measured_paths.len() as u64,
            logical_bytes: metrics.logical_bytes,
            allocated_bytes: metrics.allocated_bytes,
            unique_logical_bytes: metrics.unique_logical_bytes,
            unique_allocated_bytes: metrics.unique_allocated_bytes,
            required_flags: self.required_flags,
            required_warnings: self.required_warnings,
            suggested_command: self.suggested_command,
            manual_guidance: self.manual_guidance,
            reason: self.reason,
        }
    }
}

fn action_kind_for_advice(advice: &CleanupAdvice) -> Option<CleanupAdviceActionKind> {
    match advice.status {
        CleanupAdviceStatus::Cleanable
        | CleanupAdviceStatus::MaybeCleanable
        | CleanupAdviceStatus::ContainsCleanable => action_command_and_gates(advice)
            .0
            .as_ref()
            .map(|_| CleanupAdviceActionKind::RebeccaCommand),
        CleanupAdviceStatus::ReviewOnly => Some(CleanupAdviceActionKind::ManualReview),
        CleanupAdviceStatus::Protected => Some(CleanupAdviceActionKind::Protected),
        CleanupAdviceStatus::Unknown => None,
    }
}

fn advice_owner_path(advice: &CleanupAdvice, entry_path: &Path) -> PathBuf {
    advice
        .manual_guidance
        .as_ref()
        .and_then(|guidance| guidance.evidence_path.clone())
        .or_else(|| advice.matched_path.clone())
        .unwrap_or_else(|| entry_path.to_path_buf())
}

fn advice_can_measure_entry(advice: &CleanupAdvice, entry_path: &Path, owner_path: &Path) -> bool {
    if matches!(advice.relation, Some(CleanupAdviceRelation::Ancestor)) {
        return false;
    }
    matches!(
        path_relation(entry_path, owner_path),
        PathRelation::Same | PathRelation::Descendant
    )
}

fn action_command_and_gates(
    advice: &CleanupAdvice,
) -> (Option<CleanupAdviceCommand>, Vec<String>, Vec<String>) {
    if let Some(command) = advice.suggested_command.clone() {
        return (
            Some(command),
            advice.required_flags.clone(),
            advice.required_warnings.clone(),
        );
    }

    advice
        .evidence
        .iter()
        .find_map(|evidence| {
            if !matches!(
                evidence.status,
                CleanupAdviceStatus::Cleanable
                    | CleanupAdviceStatus::MaybeCleanable
                    | CleanupAdviceStatus::ContainsCleanable
            ) {
                return None;
            }
            let command = evidence.suggested_command.clone()?;
            Some((
                Some(command),
                evidence.required_flags.clone(),
                evidence.required_warnings.clone(),
            ))
        })
        .unwrap_or((None, Vec::new(), Vec::new()))
}

fn cleanup_evidence_can_be_action(evidence: &CleanupAdviceEvidence) -> bool {
    matches!(
        evidence.status,
        CleanupAdviceStatus::Cleanable
            | CleanupAdviceStatus::MaybeCleanable
            | CleanupAdviceStatus::ContainsCleanable
    )
}

fn evidence_can_measure_entry(
    evidence: &CleanupAdviceEvidence,
    entry_path: &Path,
    owner_path: &Path,
) -> bool {
    if matches!(evidence.relation, Some(CleanupAdviceRelation::Ancestor)) {
        return false;
    }
    matches!(
        path_relation(entry_path, owner_path),
        PathRelation::Same | PathRelation::Descendant
    )
}

fn entry_metrics(entry: &crate::disk_map::DiskMapEntry) -> DiskMapMetrics {
    DiskMapMetrics {
        logical_bytes: entry.logical_bytes,
        allocated_bytes: entry.allocated_bytes,
        unique_logical_bytes: entry.unique_logical_bytes,
        unique_allocated_bytes: entry.unique_allocated_bytes,
        files: entry.files,
        directories: entry.directories,
    }
}

fn sum_action_metrics(paths: &[(PathBuf, DiskMapMetrics)]) -> DiskMapMetrics {
    let mut metrics = DiskMapMetrics::default();
    for (_, path_metrics) in paths {
        metrics.add(*path_metrics);
    }
    metrics
}

fn merge_strings(target: &mut Vec<String>, source: Vec<String>) {
    for value in source {
        if target.iter().all(|existing| existing != &value) {
            target.push(value);
        }
    }
}

fn cleanup_action_id(
    kind: CleanupAdviceActionKind,
    source: Option<CleanupAdviceSource>,
    rule_id: Option<&str>,
    command: Option<&CleanupAdviceCommand>,
    owner_path: &Path,
) -> String {
    let path_key = comparable_path_key(owner_path);
    let command_key = command
        .map(|command| format!("{} {}", command.command, command.args.join(" ")))
        .unwrap_or_default();
    let identity = format!(
        "{}|{}|{}|{}|{}",
        kind.label(),
        source.map(CleanupAdviceSource::label).unwrap_or("unknown"),
        rule_id.unwrap_or("none"),
        command_key,
        path_key
    );
    format!("{}-{:016x}", kind.label(), stable_hash64(&identity))
}

fn stable_hash64(value: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn action_sort_rank(action: &CleanupAdviceAction) -> (u8, u8) {
    let kind_rank = match action.kind {
        CleanupAdviceActionKind::RebeccaCommand => 3,
        CleanupAdviceActionKind::ManualReview => 2,
        CleanupAdviceActionKind::Protected => 1,
    };
    (kind_rank, cleanup_status_rank(action.status))
}

fn cleanup_status_rank(status: CleanupAdviceStatus) -> u8 {
    match status {
        CleanupAdviceStatus::Cleanable => 6,
        CleanupAdviceStatus::MaybeCleanable => 5,
        CleanupAdviceStatus::ReviewOnly => 4,
        CleanupAdviceStatus::ContainsCleanable => 3,
        CleanupAdviceStatus::Protected => 2,
        CleanupAdviceStatus::Unknown => 1,
    }
}

fn workspace_insight_candidate_from_disk_map(
    insight: &DiskMapWorkspaceInsight,
) -> WorkspaceInsightCandidate {
    let metadata = workspace_insight_metadata(insight.kind);
    WorkspaceInsightCandidate {
        path: insight.owner_path.clone(),
        rule_id: metadata.rule_id,
        reason: metadata.reason,
        manual_review_hint: metadata.manual_review_hint,
        external_tool_hint: metadata.external_tool_hint,
        priority: metadata.priority,
        logical_bytes: insight.metrics.logical_bytes,
    }
}

#[derive(Debug, Clone, Copy)]
struct WorkspaceInsightMetadata {
    rule_id: &'static str,
    reason: &'static str,
    manual_review_hint: &'static str,
    external_tool_hint: Option<&'static str>,
    priority: u8,
}

fn workspace_insight_metadata(kind: DiskMapWorkspaceInsightKind) -> WorkspaceInsightMetadata {
    match kind {
        DiskMapWorkspaceInsightKind::GitObjectStore => WorkspaceInsightMetadata {
            rule_id: "workspace.git-object-store",
            reason: "Git repository history can be large; Rebecca does not delete Git history.",
            manual_review_hint: "Review remotes, LFS data, stale worktrees, and branch retention before pruning or deleting a clone.",
            external_tool_hint: Some(
                "git gc, git prune, git worktree prune, and git lfs prune can help after you decide the repository state is safe.",
            ),
            priority: 100,
        },
        DiskMapWorkspaceInsightKind::SvnPristineStore => WorkspaceInsightMetadata {
            rule_id: "workspace.svn-pristine-store",
            reason: "Subversion pristine data can be large; Rebecca does not delete SVN history.",
            manual_review_hint: "Review checkout age and local modifications before running SVN cleanup workflows or removing the checkout.",
            external_tool_hint: Some(
                "svn cleanup and a fresh checkout are safer than deleting .svn internals by hand.",
            ),
            priority: 95,
        },
        DiskMapWorkspaceInsightKind::UnityLibraryCache => WorkspaceInsightMetadata {
            rule_id: "workspace.unity-library-cache",
            reason: "Unity Library is rebuildable but expensive to regenerate; Rebecca only reports it as review-only.",
            manual_review_hint: "Close Unity, confirm the project can regenerate Library, and expect the next open/import to take time.",
            external_tool_hint: Some(
                "Unity will rebuild Library after deletion, but project-specific generated state may affect import time.",
            ),
            priority: 80,
        },
        DiskMapWorkspaceInsightKind::VcpkgBuildCache => WorkspaceInsightMetadata {
            rule_id: "workspace.vcpkg-build-cache",
            reason: "vcpkg buildtrees, packages, and downloads can be large; Rebecca does not delete vcpkg internals automatically.",
            manual_review_hint: "Confirm no vcpkg build is running and decide whether build artifacts or downloaded archives are still useful.",
            external_tool_hint: Some(
                "Use vcpkg cleanup or remove selected vcpkg cache directories after builds finish.",
            ),
            priority: 75,
        },
        DiskMapWorkspaceInsightKind::ReferenceRepositoryCache => WorkspaceInsightMetadata {
            rule_id: "workspace.reference-repository-cache",
            reason: "Reference repositories are project context, not disposable cache.",
            manual_review_hint: "Review whether these clones are still needed by agents, docs, tests, or local benchmarking before removing them.",
            external_tool_hint: None,
            priority: 70,
        },
        DiskMapWorkspaceInsightKind::GeneratedOutputTree => WorkspaceInsightMetadata {
            rule_id: "workspace.generated-output-tree",
            reason: "Generated output trees can be large, but project workflows decide whether they are disposable.",
            manual_review_hint: "Confirm the output can be regenerated and is not the only copy of a release artifact or exported data.",
            external_tool_hint: None,
            priority: 60,
        },
        DiskMapWorkspaceInsightKind::LocalMirrorData => WorkspaceInsightMetadata {
            rule_id: "workspace.local-mirror-data",
            reason: "Local mirror data is often intentionally retained source material.",
            manual_review_hint: "Review ownership, sync state, and regeneration cost before deleting mirrored data.",
            external_tool_hint: None,
            priority: 55,
        },
        DiskMapWorkspaceInsightKind::GameLibraryData => WorkspaceInsightMetadata {
            rule_id: "workspace.game-library-data",
            reason: "Game library data is durable application content; Rebecca does not delete installed games.",
            manual_review_hint: "Review the owning launcher, uninstall unused games there, or move the library through the launcher before deleting files by hand.",
            external_tool_hint: Some(
                "Steam, Battle.net, and game launchers usually provide safer uninstall or move-library workflows.",
            ),
            priority: 85,
        },
    }
}

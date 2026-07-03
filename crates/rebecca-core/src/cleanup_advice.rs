use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::applications::ApplicationDiscovery;
use crate::discovery::{
    DiscoveryIndex, TargetResolution, resolve_rule_target_with_applications_and_index,
};
use crate::disk_map::DiskMapReport;
use crate::environment::Environment;
use crate::error::Result;
use crate::model::{PlanRequest, RuleDefinition, SafetyLevel};
use crate::path_overlap::{PathRelation, path_relation};
use crate::project_artifacts::{ProjectArtifactCandidate, recently_modified_reason};
use crate::protection::{ProtectionAssessment, ProtectionPolicy};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CleanupAdviceStatus {
    Cleanable,
    MaybeCleanable,
    ContainsCleanable,
    Protected,
    Unknown,
}

impl CleanupAdviceStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Cleanable => "cleanable",
            Self::MaybeCleanable => "maybe-cleanable",
            Self::ContainsCleanable => "contains-cleanable",
            Self::Protected => "protected",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CleanupAdviceSource {
    CleanupRule,
    ProjectArtifact,
    AppLeftover,
    Protection,
}

impl CleanupAdviceSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::CleanupRule => "cleanup-rule",
            Self::ProjectArtifact => "project-artifact",
            Self::AppLeftover => "app-leftover",
            Self::Protection => "protection",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggested_command: Option<CleanupAdviceCommand>,
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
            reason: "no cleanup rule or protection policy matched this path".to_string(),
            suggested_command: None,
        }
    }

    fn protected(kind: String, reason: String) -> Self {
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
            reason,
            suggested_command: None,
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
                        path,
                        rule_id: Some(rule.id.clone()),
                        category: Some(rule.category.clone()),
                        safety_level: Some(rule.safety_level),
                        required_flags: required_flags.clone(),
                        required_warnings: required_warnings.clone(),
                        reason: None,
                        suggested_command: cleanup_rule_command(&rule.id),
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
        match self.protection_policy.assess_path(path) {
            ProtectionAssessment::Allowed => {}
            ProtectionAssessment::Blocked(block) => {
                return CleanupAdvice::protected(block.kind.label().to_string(), block.message);
            }
        }

        self.targets
            .iter()
            .filter_map(|target| target.advice_for(path))
            .max_by_key(CleanupAdviceCandidate::rank)
            .map(CleanupAdviceCandidate::into_advice)
            .unwrap_or_else(CleanupAdvice::unknown)
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
            });
        }
    }

    pub fn annotate_disk_map_report(&self, report: &mut DiskMapReport) {
        for entry in &mut report.top_entries {
            entry.cleanup_advice = Some(self.advise_path(&entry.path));
        }
    }
}

#[derive(Debug, Clone)]
struct CleanupAdviceTarget {
    source: CleanupAdviceSource,
    path: PathBuf,
    rule_id: Option<String>,
    category: Option<String>,
    safety_level: Option<SafetyLevel>,
    required_flags: Vec<String>,
    required_warnings: Vec<String>,
    reason: Option<String>,
    suggested_command: Option<CleanupAdviceCommand>,
}

impl CleanupAdviceTarget {
    fn advice_for(&self, entry_path: &Path) -> Option<CleanupAdviceCandidate> {
        let relation = match path_relation(entry_path, &self.path) {
            PathRelation::Same => CleanupAdviceRelation::Exact,
            PathRelation::Descendant => CleanupAdviceRelation::Descendant,
            PathRelation::Ancestor => CleanupAdviceRelation::Ancestor,
            PathRelation::Unrelated => return None,
        };

        let status = if matches!(relation, CleanupAdviceRelation::Ancestor) {
            CleanupAdviceStatus::ContainsCleanable
        } else if self.required_flags.is_empty() && self.required_warnings.is_empty() {
            CleanupAdviceStatus::Cleanable
        } else {
            CleanupAdviceStatus::MaybeCleanable
        };

        Some(CleanupAdviceCandidate {
            rank: advice_rank(status, relation),
            advice: CleanupAdvice {
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
                reason: self.advice_reason(status, relation),
                suggested_command: self.suggested_command.clone(),
            },
        })
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
            CleanupAdviceSource::AppLeftover => format!(
                "path matched app leftover policy {}",
                self.rule_id.as_deref().unwrap_or("matched policy")
            ),
            CleanupAdviceSource::Protection => "path matched protection policy".to_string(),
        }
    }
}

#[derive(Debug, Clone)]
struct CleanupAdviceCandidate {
    rank: (u8, u8),
    advice: CleanupAdvice,
}

impl CleanupAdviceCandidate {
    fn rank(&self) -> (u8, u8) {
        self.rank
    }

    fn into_advice(self) -> CleanupAdvice {
        self.advice
    }
}

fn advice_rank(status: CleanupAdviceStatus, relation: CleanupAdviceRelation) -> (u8, u8) {
    let status_rank = match status {
        CleanupAdviceStatus::Cleanable => 4,
        CleanupAdviceStatus::MaybeCleanable => 3,
        CleanupAdviceStatus::ContainsCleanable => 2,
        CleanupAdviceStatus::Protected => 5,
        CleanupAdviceStatus::Unknown => 0,
    };
    let relation_rank = match relation {
        CleanupAdviceRelation::Exact => 3,
        CleanupAdviceRelation::Descendant => 2,
        CleanupAdviceRelation::Ancestor => 1,
    };

    (status_rank, relation_rank)
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

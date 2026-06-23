use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub use crate::path_template::PathTemplate;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Platform {
    Windows,
    Linux,
    Macos,
}

impl Platform {
    pub fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::Macos
        } else {
            Self::Linux
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SafetyLevel {
    Safe,
    Moderate,
    Risky,
    Dangerous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DeletePolicy {
    RecycleBin,
    Permanent,
    Command,
    ReviewOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DeleteMode {
    DryRun,
    RecycleBin,
    Permanent,
}

impl DeleteMode {
    pub fn is_dry_run(self) -> bool {
        matches!(self, Self::DryRun)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "kebab-case")]
pub enum RuleTargetSpec {
    Template(PathTemplate),
    ExactPath(PathBuf),
}

impl RuleTargetSpec {
    pub fn template(template: impl Into<String>) -> Self {
        Self::Template(PathTemplate::new(template))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleDefinition {
    pub id: String,
    pub platform: Platform,
    pub category: String,
    pub name: String,
    pub safety_level: SafetyLevel,
    pub path_templates: Vec<RuleTargetSpec>,
    pub delete_policy: DeletePolicy,
    pub restore_hint: Option<String>,
    pub provenance: RuleProvenance,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleProvenance {
    pub source: RuleSource,
    pub license: String,
    pub notes: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RuleSource {
    Owned,
    ReferenceOnly,
    UserDefined,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanRequest {
    pub platform: Platform,
    pub mode: DeleteMode,
    pub selected_categories: Vec<String>,
    pub selected_rule_ids: Vec<String>,
    pub allow_moderate: bool,
    pub allow_risky: bool,
}

impl PlanRequest {
    pub fn new(mode: DeleteMode) -> Self {
        Self::for_platform(Platform::current(), mode)
    }

    pub fn for_platform(platform: Platform, mode: DeleteMode) -> Self {
        Self {
            platform,
            mode,
            selected_categories: Vec::new(),
            selected_rule_ids: Vec::new(),
            allow_moderate: false,
            allow_risky: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TargetStatus {
    Allowed,
    Skipped,
    Blocked,
    Failed,
    Completed,
}

impl TargetStatus {
    pub fn is_executable(self) -> bool {
        matches!(self, Self::Allowed)
    }
}

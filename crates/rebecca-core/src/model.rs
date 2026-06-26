use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub use crate::path_template::PathTemplate;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Platform {
    Windows,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SafetyLevel {
    Safe,
    Moderate,
    Risky,
    Dangerous,
}

impl SafetyLevel {
    pub fn label(self) -> &'static str {
        match self {
            Self::Safe => "safe",
            Self::Moderate => "moderate",
            Self::Risky => "risky",
            Self::Dangerous => "dangerous",
        }
    }

    pub fn opt_in_flag(self) -> Option<&'static str> {
        match self {
            Self::Safe => None,
            Self::Moderate => Some("--allow-moderate"),
            Self::Risky | Self::Dangerous => Some("--allow-risky"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DeleteMode {
    DryRun,
    RecycleBin,
}

impl DeleteMode {
    pub fn is_dry_run(self) -> bool {
        matches!(self, Self::DryRun)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CleanupWorkflow {
    Rules,
    AppLeftovers,
}

impl CleanupWorkflow {
    pub fn label(self) -> &'static str {
        match self {
            Self::Rules => "cleanup",
            Self::AppLeftovers => "app leftovers",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Rules => "Cleanup",
            Self::AppLeftovers => "App leftovers",
        }
    }
}

impl Default for CleanupWorkflow {
    fn default() -> Self {
        Self::Rules
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "kebab-case")]
pub enum RuleTargetSpec {
    Template(PathTemplate),
    ExactPath(PathBuf),
    GlobTemplate(PathTemplate),
    SteamInstallTemplate(PathTemplate),
    SteamLibraryTemplate(PathTemplate),
}

impl RuleTargetSpec {
    pub fn template(template: impl Into<String>) -> Self {
        Self::Template(PathTemplate::new(template))
    }

    pub fn glob_template(template: impl Into<String>) -> Self {
        Self::GlobTemplate(PathTemplate::new(template))
    }

    pub fn steam_install_template(template: impl Into<String>) -> Self {
        Self::SteamInstallTemplate(PathTemplate::new(template))
    }

    pub fn steam_library_template(template: impl Into<String>) -> Self {
        Self::SteamLibraryTemplate(PathTemplate::new(template))
    }

    pub fn placeholder_path(&self) -> PathBuf {
        match self {
            Self::Template(template)
            | Self::GlobTemplate(template)
            | Self::SteamInstallTemplate(template)
            | Self::SteamLibraryTemplate(template) => PathBuf::from(template.raw()),
            Self::ExactPath(path) => path.clone(),
        }
    }

    pub fn dedupe_key(&self, platform: Platform) -> String {
        let target = match self {
            Self::Template(template) => format!("template:{}", template.raw()),
            Self::ExactPath(path) => format!("exact-path:{}", path.display()),
            Self::GlobTemplate(template) => format!("glob-template:{}", template.raw()),
            Self::SteamInstallTemplate(template) => {
                format!("steam-install-template:{}", template.raw())
            }
            Self::SteamLibraryTemplate(template) => {
                format!("steam-library-template:{}", template.raw())
            }
        }
        .replace('\\', "/");

        format!("{platform:?}:{}", target.to_ascii_lowercase())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleSelection {
    pub categories: Vec<String>,
    pub rule_ids: Vec<String>,
}

impl RuleSelection {
    pub fn new(categories: Vec<String>, rule_ids: Vec<String>) -> Self {
        Self {
            categories,
            rule_ids,
        }
    }

    pub fn rule_ids(&self) -> &[String] {
        &self.rule_ids
    }

    pub fn from_request(request: &PlanRequest) -> Self {
        Self::new(
            request.selected_categories.clone(),
            request.selected_rule_ids.clone(),
        )
    }

    pub fn matches_rule(&self, rule: &RuleDefinition) -> bool {
        let selected_category = self.matches_any(&self.categories, &rule.category);
        let selected_id = self.matches_any(&self.rule_ids, &rule.id);

        selected_category && selected_id
    }

    pub fn validate_against_rules(
        &self,
        rules: &[RuleDefinition],
    ) -> Result<(), crate::RebeccaError> {
        for selected in &self.categories {
            let known = rules
                .iter()
                .any(|rule| rule.category.eq_ignore_ascii_case(selected));
            if !known {
                return Err(crate::RebeccaError::InvalidCategory(selected.clone()));
            }
        }

        for selected in self.rule_ids() {
            let known = rules
                .iter()
                .any(|rule| rule.id.eq_ignore_ascii_case(selected));
            if !known {
                return Err(crate::RebeccaError::InvalidRuleId(selected.clone()));
            }
        }

        Ok(())
    }

    fn matches_any(&self, selected: &[String], value: &str) -> bool {
        selected.is_empty() || selected.iter().any(|item| item.eq_ignore_ascii_case(value))
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanRequest {
    pub platform: Platform,
    pub mode: DeleteMode,
    #[serde(default)]
    pub workflow: CleanupWorkflow,
    pub selected_categories: Vec<String>,
    pub selected_rule_ids: Vec<String>,
    pub allow_moderate: bool,
    pub allow_risky: bool,
}

impl PlanRequest {
    pub fn for_platform(platform: Platform, mode: DeleteMode) -> Self {
        Self {
            platform,
            mode,
            workflow: CleanupWorkflow::Rules,
            selected_categories: Vec::new(),
            selected_rule_ids: Vec::new(),
            allow_moderate: false,
            allow_risky: false,
        }
    }

    pub fn selection(&self) -> RuleSelection {
        RuleSelection::from_request(self)
    }

    pub fn with_workflow(mut self, workflow: CleanupWorkflow) -> Self {
        self.workflow = workflow;
        self
    }

    pub fn allows_safety_level(&self, level: SafetyLevel) -> bool {
        match level {
            SafetyLevel::Safe => true,
            SafetyLevel::Moderate => self.allow_moderate || self.allow_risky,
            SafetyLevel::Risky | SafetyLevel::Dangerous => self.allow_risky,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
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

    pub fn is_issue(self) -> bool {
        matches!(self, Self::Skipped | Self::Blocked | Self::Failed)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Allowed => "allowed",
            Self::Skipped => "skipped",
            Self::Blocked => "blocked",
            Self::Failed => "failed",
            Self::Completed => "completed",
        }
    }
}

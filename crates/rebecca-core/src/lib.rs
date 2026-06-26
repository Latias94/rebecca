pub mod app_leftovers;
pub mod applications;
pub mod cache;
pub mod config;
pub mod discovery;
pub mod environment;
pub mod error;
pub mod executor;
pub mod history;
pub mod model;
mod path_overlap;
pub mod path_template;
pub mod plan;
pub mod planner;
pub mod project_artifacts;
pub mod protection;
pub mod safety;
pub mod scan;
pub mod scan_cache;

pub use error::{RebeccaError, Result, ScanFailure, ScanFailureKind, ScanFailurePhase};
pub use model::{
    CleanupWorkflow, DEFAULT_PROJECT_ARTIFACT_MAX_DEPTH, DEFAULT_PROJECT_ARTIFACT_MIN_AGE_DAYS,
    DeleteMode, PathTemplate, PlanRequest, Platform, RuleDefinition, RuleProvenance, RuleSelection,
    RuleSource, RuleTargetSpec, SafetyLevel, TargetStatus,
};
pub use plan::{
    CleanupIssueSummary, CleanupPlan, CleanupSummary, CleanupTarget, CleanupTargetIssueReason,
};

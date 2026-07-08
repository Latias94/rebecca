pub mod app_leftovers;
pub mod applications;
pub mod cache;
pub mod catalog;
pub mod cleanup_advice;
pub mod config;
pub mod discovery;
pub mod disk_map;
pub mod disk_session;
pub mod environment;
pub mod error;
pub mod execution;
pub mod executor;
pub mod external_rules;
pub mod history;
pub mod inspect;
pub mod inventory;
pub mod lint;
mod macos_paths;
pub mod manifest;
pub mod model;
mod parallelism;
mod path_overlap;
pub mod path_template;
pub mod plan;
pub mod planner;
pub mod progress;
pub mod project_artifacts;
pub mod protection;
pub mod safety;
pub mod safety_catalog;
pub mod scan;
pub mod scan_cache;
pub mod warnings;

pub use cleanup_advice::{
    CleanupAdvice, CleanupAdviceBuildRequest, CleanupAdviceCommand, CleanupAdviceEvidence,
    CleanupAdviceIndex, CleanupAdviceRelation, CleanupAdviceSource, CleanupAdviceStatus,
};
pub use error::{RebeccaError, Result, ScanFailure, ScanFailureKind, ScanFailurePhase};
pub use execution::{
    ExecutionActionReport, ExecutionReport, ExecutionSummary, ExecutionWarning,
    ExecutionWarningKind,
};
pub use model::{
    CleanupWorkflow, DEFAULT_PROJECT_ARTIFACT_MAX_DEPTH, DEFAULT_PROJECT_ARTIFACT_MIN_AGE_DAYS,
    DeleteMode, PathTemplate, PlanRequest, Platform, RuleDefinition, RuleProvenance,
    RuleSearchKind, RuleSelection, RuleSource, RuleTargetSpec, SafetyLevel, TargetStatus,
};
pub use plan::{
    CleanupIssueSummary, CleanupPlan, CleanupSummary, CleanupTarget, CleanupTargetDeletionStyle,
    CleanupTargetIssueReason, EstimateProvenance, EstimateSource,
};
pub use warnings::WarningSummary;

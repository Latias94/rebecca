use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use rebecca_core::execution::ExecutionReport;
use rebecca_core::plan::{
    CleanupPlan, CleanupSummary, CleanupTarget, CleanupTargetIssueReason, EstimateProvenance,
    EstimateSource,
};
use rebecca_core::{CleanupWorkflow, DeleteMode, Platform, TargetStatus};
use serde::Serialize;

use crate::output::format_shell_command;

const CLEANUP_RECEIPT_SCHEMA: &str = "rebecca.cleanup-receipt.v1";

#[derive(Debug, Clone, Serialize)]
pub(crate) struct CleanupReceipt {
    schema: &'static str,
    generated_by: String,
    generated_at_unix_seconds: u64,
    command: &'static str,
    invocation: CleanupReceiptInvocation,
    request: CleanupReceiptRequest,
    selected_gates: CleanupReceiptSelectedGates,
    platform: Platform,
    workflow: CleanupWorkflow,
    mode: DeleteMode,
    destination: CleanupReceiptDestination,
    destination_label: &'static str,
    summary: CleanupSummary,
    execution_report: ExecutionReport,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_plan: Option<CleanupReceiptSourcePlan>,
    #[serde(skip_serializing_if = "Option::is_none")]
    revalidation: Option<CleanupReceiptRevalidationSummary>,
    targets: Vec<CleanupReceiptTarget>,
    next_steps: Vec<CleanupReceiptNextStep>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct CleanupReceiptContext {
    invocation: CleanupReceiptInvocation,
    source_plan: Option<CleanupReceiptSourcePlan>,
    revalidation: Option<CleanupReceiptRevalidationSummary>,
}

#[derive(Debug, Clone, Serialize)]
struct CleanupReceiptInvocation {
    command: &'static str,
    argv: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    working_directory: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize)]
struct CleanupReceiptRequest {
    platform: Platform,
    mode: DeleteMode,
    workflow: CleanupWorkflow,
    selected_categories: Vec<String>,
    selected_rule_ids: Vec<String>,
    project_artifact_roots: Vec<PathBuf>,
    project_artifact_max_depth: usize,
    project_artifact_min_age_days: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    project_artifact_reclaim_limit_bytes: Option<u64>,
    project_artifact_selectors: Vec<String>,
    allowed_warnings: Vec<String>,
    allow_moderate: bool,
    allow_risky: bool,
}

#[derive(Debug, Clone, Serialize)]
struct CleanupReceiptSelectedGates {
    allow_moderate: bool,
    allow_risky: bool,
    allowed_warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "kebab-case")]
enum CleanupReceiptDestination {
    WindowsRecycleBin,
    SystemTrash,
    PermanentDelete,
}

#[derive(Debug, Clone, Serialize)]
struct CleanupReceiptSourcePlan {
    path: PathBuf,
    schema: String,
    generated_at_unix_seconds: u64,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct CleanupReceiptRevalidationSummary {
    total_targets: usize,
    executable_targets: usize,
    changed_targets: usize,
    skipped_targets: usize,
    blocked_targets: usize,
}

#[derive(Debug, Clone, Serialize)]
struct CleanupReceiptTarget {
    target_index: usize,
    rule_id: String,
    path: PathBuf,
    status: TargetStatus,
    estimated_bytes: u64,
    estimate: CleanupReceiptTargetEstimate,
    freed_bytes: u64,
    pending_reclaim_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    restore_hint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct CleanupReceiptTargetEstimate {
    source: EstimateSource,
    provenance: EstimateProvenance,
}

#[derive(Debug, Clone, Serialize)]
struct CleanupReceiptNextStep {
    kind: &'static str,
    label: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    command: Option<String>,
}

pub(crate) fn write_cleanup_receipt(
    plan: &CleanupPlan,
    command: &'static str,
    path: &Path,
    context: &CleanupReceiptContext,
) -> Result<()> {
    let receipt = CleanupReceipt::from_executed_plan(plan, command, context)?;
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create receipt directory {}", parent.display()))?;
    }
    let bytes = serde_json::to_vec_pretty(&receipt).context("failed to encode cleanup receipt")?;
    fs::write(path, bytes)
        .with_context(|| format!("failed to write cleanup receipt {}", path.display()))?;
    Ok(())
}

pub(crate) fn print_receipt_guidance(path: &Path, plan: &CleanupPlan) {
    println!();
    println!("Receipt: {}", path.display());
    match plan.request.mode {
        DeleteMode::RecoverableDelete if plan.summary.pending_reclaim_bytes > 0 => {
            println!(
                "Preview pending trash space: {}",
                format_shell_command("rebecca", &["trash".into(), "empty".into()])
            );
            println!(
                "Empty after review: {}",
                format_shell_command("rebecca", &["trash".into(), "empty".into(), "--yes".into()])
            );
        }
        DeleteMode::RecoverableDelete => {
            println!("Files were moved to trash when targets completed.");
        }
        DeleteMode::PermanentDelete => {
            println!("Permanent deletion bypassed the system trash or Recycle Bin.");
        }
        DeleteMode::DryRun => {}
    }
}

impl CleanupReceipt {
    fn from_executed_plan(
        plan: &CleanupPlan,
        command: &'static str,
        context: &CleanupReceiptContext,
    ) -> Result<Self> {
        if plan.request.mode.is_dry_run() {
            return Err(anyhow!(
                "cleanup receipts require an executed cleanup request"
            ));
        }

        let destination = CleanupReceiptDestination::from_mode_and_platform(
            plan.request.mode,
            plan.request.platform,
        );
        let execution_report = plan
            .execution_report
            .clone()
            .unwrap_or_else(|| ExecutionReport::from_targets(&plan.targets));
        Ok(Self {
            schema: CLEANUP_RECEIPT_SCHEMA,
            generated_by: format!("rebecca {}", env!("CARGO_PKG_VERSION")),
            generated_at_unix_seconds: unix_now(),
            command,
            invocation: context.invocation.clone(),
            request: CleanupReceiptRequest::from_request(&plan.request),
            selected_gates: CleanupReceiptSelectedGates::from_request(&plan.request),
            platform: plan.request.platform,
            workflow: plan.request.workflow,
            mode: plan.request.mode,
            destination,
            destination_label: destination.label(),
            summary: plan.summary.clone(),
            execution_report,
            source_plan: context.source_plan.clone(),
            revalidation: context.revalidation.clone(),
            targets: plan
                .targets
                .iter()
                .enumerate()
                .map(CleanupReceiptTarget::from_indexed_target)
                .collect(),
            next_steps: next_steps_for_plan(plan),
        })
    }
}

impl CleanupReceiptContext {
    pub(crate) fn capture(command: &'static str) -> Self {
        Self {
            invocation: CleanupReceiptInvocation::capture(command),
            source_plan: None,
            revalidation: None,
        }
    }

    pub(crate) fn with_source_plan(
        mut self,
        path: PathBuf,
        schema: impl Into<String>,
        generated_at_unix_seconds: u64,
    ) -> Self {
        self.source_plan = Some(CleanupReceiptSourcePlan {
            path,
            schema: schema.into(),
            generated_at_unix_seconds,
        });
        self
    }

    pub(crate) fn with_revalidation(
        mut self,
        revalidation: CleanupReceiptRevalidationSummary,
    ) -> Self {
        self.revalidation = Some(revalidation);
        self
    }
}

impl CleanupReceiptInvocation {
    fn capture(command: &'static str) -> Self {
        let argv = std::env::args_os()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        Self {
            command,
            argv,
            working_directory: std::env::current_dir().ok(),
        }
    }
}

impl CleanupReceiptRequest {
    fn from_request(request: &rebecca_core::PlanRequest) -> Self {
        Self {
            platform: request.platform,
            mode: request.mode,
            workflow: request.workflow,
            selected_categories: request.selected_categories.clone(),
            selected_rule_ids: request.selected_rule_ids.clone(),
            project_artifact_roots: request.project_artifact_roots.clone(),
            project_artifact_max_depth: request.project_artifact_max_depth,
            project_artifact_min_age_days: request.project_artifact_min_age_days,
            project_artifact_reclaim_limit_bytes: request.project_artifact_reclaim_limit_bytes,
            project_artifact_selectors: request.project_artifact_selectors.clone(),
            allowed_warnings: request.allowed_warnings.clone(),
            allow_moderate: request.allow_moderate,
            allow_risky: request.allow_risky,
        }
    }
}

impl CleanupReceiptSelectedGates {
    fn from_request(request: &rebecca_core::PlanRequest) -> Self {
        Self {
            allow_moderate: request.allow_moderate,
            allow_risky: request.allow_risky,
            allowed_warnings: request.allowed_warnings.clone(),
        }
    }
}

impl CleanupReceiptDestination {
    fn from_mode_and_platform(mode: DeleteMode, platform: Platform) -> Self {
        match mode {
            DeleteMode::RecoverableDelete if platform == Platform::Windows => {
                Self::WindowsRecycleBin
            }
            DeleteMode::RecoverableDelete => Self::SystemTrash,
            DeleteMode::PermanentDelete => Self::PermanentDelete,
            DeleteMode::DryRun => unreachable!("cleanup receipts are not produced for dry runs"),
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::WindowsRecycleBin => "Windows Recycle Bin",
            Self::SystemTrash => "system trash",
            Self::PermanentDelete => "permanent delete",
        }
    }
}

impl CleanupReceiptRevalidationSummary {
    pub(crate) fn from_revalidated_plan(plan: &CleanupPlan) -> Self {
        let mut summary = Self {
            total_targets: plan.targets.len(),
            executable_targets: 0,
            changed_targets: 0,
            skipped_targets: 0,
            blocked_targets: 0,
        };

        for target in &plan.targets {
            if target.status.is_executable() {
                summary.executable_targets += 1;
            }
            if target.reason_code == Some(CleanupTargetIssueReason::SavedPlanTargetChanged) {
                summary.changed_targets += 1;
            }
            match target.status {
                TargetStatus::Skipped => summary.skipped_targets += 1,
                TargetStatus::Blocked => summary.blocked_targets += 1,
                _ => {}
            }
        }

        summary
    }
}

impl CleanupReceiptTarget {
    fn from_indexed_target((target_index, target): (usize, &CleanupTarget)) -> Self {
        Self {
            target_index,
            rule_id: target.rule_id.clone(),
            path: target.path.clone(),
            status: target.status,
            estimated_bytes: target.estimated_bytes,
            estimate: CleanupReceiptTargetEstimate::from_target(target),
            freed_bytes: target.freed_bytes,
            pending_reclaim_bytes: target.pending_reclaim_bytes,
            restore_hint: target.restore_hint.clone(),
            reason_code: target.reason_code.map(|reason| reason.label().to_string()),
            reason: target.reason.clone(),
        }
    }
}

impl CleanupReceiptTargetEstimate {
    fn from_target(target: &CleanupTarget) -> Self {
        Self {
            source: target.estimate_source,
            provenance: target.estimate_provenance.clone(),
        }
    }
}

fn next_steps_for_plan(plan: &CleanupPlan) -> Vec<CleanupReceiptNextStep> {
    let mut steps = Vec::new();
    if plan.request.mode == DeleteMode::RecoverableDelete && plan.summary.pending_reclaim_bytes > 0
    {
        steps.push(CleanupReceiptNextStep {
            kind: "preview-trash",
            label: "Preview the system trash or Recycle Bin before emptying pending space.",
            command: Some(format_shell_command(
                "rebecca",
                &["trash".into(), "empty".into()],
            )),
        });
        steps.push(CleanupReceiptNextStep {
            kind: "empty-trash-after-review",
            label: "After reviewing the preview, empty the system trash or Recycle Bin to free pending space.",
            command: Some(format_shell_command(
                "rebecca",
                &["trash".into(), "empty".into(), "--yes".into()],
            )),
        });
    }
    if plan.summary.failed_targets > 0 || plan.summary.blocked_targets > 0 {
        steps.push(CleanupReceiptNextStep {
            kind: "review-issues",
            label: "Review failed or blocked targets before retrying.",
            command: None,
        });
    }
    if plan.request.mode == DeleteMode::PermanentDelete && plan.summary.completed_targets > 0 {
        steps.push(CleanupReceiptNextStep {
            kind: "permanent-delete-complete",
            label: "Permanent deletion cannot be restored from trash.",
            command: None,
        });
    }
    steps
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use rebecca::core::execution::ExecutionReport;
use rebecca::core::plan::{CleanupPlan, CleanupSummary};
use rebecca::core::{CleanupWorkflow, DeleteMode, Platform, TargetStatus};
use serde::Serialize;

use crate::output::format_shell_command;

const CLEANUP_RECEIPT_SCHEMA: &str = "rebecca.cleanup-receipt.v1";

#[derive(Debug, Clone, Serialize)]
pub(crate) struct CleanupReceipt {
    schema: &'static str,
    generated_by: String,
    generated_at_unix_seconds: u64,
    command: &'static str,
    platform: Platform,
    workflow: CleanupWorkflow,
    mode: DeleteMode,
    destination: CleanupReceiptDestination,
    summary: CleanupSummary,
    execution_report: ExecutionReport,
    targets: Vec<CleanupReceiptTarget>,
    next_steps: Vec<CleanupReceiptNextStep>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "kebab-case")]
enum CleanupReceiptDestination {
    WindowsRecycleBin,
    SystemTrash,
    PermanentDelete,
}

#[derive(Debug, Clone, Serialize)]
struct CleanupReceiptTarget {
    target_index: usize,
    rule_id: String,
    path: PathBuf,
    status: TargetStatus,
    estimated_bytes: u64,
    freed_bytes: u64,
    pending_reclaim_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
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
) -> Result<()> {
    let receipt = CleanupReceipt::from_executed_plan(plan, command)?;
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
                "Free pending space: {}",
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
    fn from_executed_plan(plan: &CleanupPlan, command: &'static str) -> Result<Self> {
        if plan.request.mode.is_dry_run() {
            return Err(anyhow!(
                "cleanup receipts require an executed cleanup request"
            ));
        }

        let execution_report = plan
            .execution_report
            .clone()
            .unwrap_or_else(|| ExecutionReport::from_targets(&plan.targets));
        Ok(Self {
            schema: CLEANUP_RECEIPT_SCHEMA,
            generated_by: format!("rebecca {}", env!("CARGO_PKG_VERSION")),
            generated_at_unix_seconds: unix_now(),
            command,
            platform: plan.request.platform,
            workflow: plan.request.workflow,
            mode: plan.request.mode,
            destination: CleanupReceiptDestination::from_mode_and_platform(
                plan.request.mode,
                plan.request.platform,
            ),
            summary: plan.summary.clone(),
            execution_report,
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
}

impl CleanupReceiptTarget {
    fn from_indexed_target((target_index, target): (usize, &rebecca::core::CleanupTarget)) -> Self {
        Self {
            target_index,
            rule_id: target.rule_id.clone(),
            path: target.path.clone(),
            status: target.status,
            estimated_bytes: target.estimated_bytes,
            freed_bytes: target.freed_bytes,
            pending_reclaim_bytes: target.pending_reclaim_bytes,
            reason_code: target.reason_code.map(|reason| reason.label().to_string()),
            reason: target.reason.clone(),
        }
    }
}

fn next_steps_for_plan(plan: &CleanupPlan) -> Vec<CleanupReceiptNextStep> {
    let mut steps = Vec::new();
    if plan.request.mode == DeleteMode::RecoverableDelete && plan.summary.pending_reclaim_bytes > 0
    {
        steps.push(CleanupReceiptNextStep {
            kind: "empty-trash",
            label: "Empty trash to free the pending space.",
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

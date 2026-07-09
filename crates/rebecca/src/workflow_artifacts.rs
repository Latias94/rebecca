use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use rebecca_core::DeleteMode;
use rebecca_core::execution::ExecutionReport;
use rebecca_core::plan::CleanupPlan;

use crate::cleanup_receipt::CleanupReceiptContext;
use crate::cli::OutputMode;
use crate::output::format_shell_command;
use crate::workflow_execution::record_execution_report;

#[derive(Debug, Clone)]
pub(crate) struct WorkflowArtifacts<'a> {
    command: &'static str,
    output_mode: OutputMode,
    save_plan: Option<&'a Path>,
    receipt: Option<&'a Path>,
    receipt_context: CleanupReceiptContext,
}

impl<'a> WorkflowArtifacts<'a> {
    pub(crate) fn new(
        command: &'static str,
        output_mode: OutputMode,
        save_plan: Option<&'a Path>,
        receipt: Option<&'a Path>,
    ) -> Self {
        Self {
            command,
            output_mode,
            save_plan,
            receipt,
            receipt_context: CleanupReceiptContext::capture(command),
        }
    }

    pub(crate) fn with_receipt_context(mut self, receipt_context: CleanupReceiptContext) -> Self {
        self.receipt_context = receipt_context;
        self
    }

    pub(crate) fn validate_for_mode(&self, mode: DeleteMode) -> Result<()> {
        if self.save_plan.is_some() && !mode.is_dry_run() {
            return Err(anyhow!("--save-plan only works with preview plans"));
        }
        if self.receipt.is_some() && mode.is_dry_run() {
            return Err(anyhow!("--receipt requires --yes"));
        }
        Ok(())
    }

    pub(crate) fn write_preview_plan(&self, plan: &CleanupPlan) -> Result<()> {
        if let Some(path) = self.save_plan {
            crate::saved_plan::write_saved_plan(plan, path)?;
        }
        Ok(())
    }

    pub(crate) fn print_preview_guidance(&self) {
        if !self.output_mode.is_human() {
            return;
        }
        if let Some(path) = self.save_plan {
            print_saved_plan_guidance(path);
        }
    }

    pub(crate) fn write_execution_receipt(&self, plan: &CleanupPlan) -> Result<()> {
        if let Some(path) = self.receipt {
            crate::cleanup_receipt::write_cleanup_receipt(
                plan,
                self.command,
                path,
                &self.receipt_context,
            )?;
        }
        Ok(())
    }

    pub(crate) fn print_execution_guidance(&self, plan: &CleanupPlan) {
        if !self.output_mode.is_human() {
            return;
        }
        if let Some(path) = self.receipt {
            crate::cleanup_receipt::print_receipt_guidance(path, plan);
        }
    }

    pub(crate) fn record_execution(
        &self,
        plan: &mut CleanupPlan,
        mut execution_report: ExecutionReport,
        history_file: PathBuf,
    ) -> Result<()> {
        let warning = record_execution_report(plan, &mut execution_report, history_file);
        if let Some(warning) = warning {
            eprintln!("Warning: {}", warning.message);
        }
        self.write_execution_receipt(plan)
    }
}

fn print_saved_plan_guidance(path: &Path) {
    println!();
    println!("Saved plan: {}", path.display());
    println!(
        "Review later: {}",
        format_shell_command(
            "rebecca",
            &[
                "plan".to_string(),
                "inspect".to_string(),
                path.display().to_string()
            ],
        )
    );
    println!(
        "Execute later: {}",
        format_shell_command(
            "rebecca",
            &[
                "plan".to_string(),
                "run".to_string(),
                path.display().to_string(),
                "--yes".to_string(),
            ],
        )
    );
}

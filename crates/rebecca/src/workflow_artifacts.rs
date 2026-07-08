use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use rebecca::core::DeleteMode;
use rebecca::core::execution::ExecutionReport;
use rebecca::core::history::HistoryStore;
use rebecca::core::plan::CleanupPlan;

use crate::cli::OutputMode;
use crate::output::format_shell_command;

#[derive(Debug, Clone, Copy)]
pub(crate) struct WorkflowArtifacts<'a> {
    command: &'static str,
    output_mode: OutputMode,
    save_plan: Option<&'a Path>,
    receipt: Option<&'a Path>,
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
        }
    }

    pub(crate) fn validate_for_mode(self, mode: DeleteMode) -> Result<()> {
        if self.save_plan.is_some() && !mode.is_dry_run() {
            return Err(anyhow!("--save-plan only works with preview plans"));
        }
        if self.receipt.is_some() && mode.is_dry_run() {
            return Err(anyhow!("--receipt requires --yes"));
        }
        Ok(())
    }

    pub(crate) fn write_preview_plan(self, plan: &CleanupPlan) -> Result<()> {
        if let Some(path) = self.save_plan {
            crate::saved_plan::write_saved_plan(plan, path)?;
        }
        Ok(())
    }

    pub(crate) fn print_preview_guidance(self) {
        if !self.output_mode.is_human() {
            return;
        }
        if let Some(path) = self.save_plan {
            print_saved_plan_guidance(path);
        }
    }

    pub(crate) fn write_execution_receipt(self, plan: &CleanupPlan) -> Result<()> {
        if let Some(path) = self.receipt {
            crate::cleanup_receipt::write_cleanup_receipt(plan, self.command, path)?;
        }
        Ok(())
    }

    pub(crate) fn print_execution_guidance(self, plan: &CleanupPlan) {
        if !self.output_mode.is_human() {
            return;
        }
        if let Some(path) = self.receipt {
            crate::cleanup_receipt::print_receipt_guidance(path, plan);
        }
    }

    pub(crate) fn record_execution(
        self,
        plan: &mut CleanupPlan,
        mut execution_report: ExecutionReport,
        history_file: PathBuf,
    ) -> Result<()> {
        let history_append = HistoryStore::new(history_file).append_plan_report(plan);
        if let Some(warning) = history_append.warning {
            eprintln!("Warning: {}", warning.message);
            execution_report.push_warning(warning);
        }
        plan.execution_report = Some(execution_report);
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

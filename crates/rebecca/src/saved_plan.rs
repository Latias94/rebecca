use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use rebecca::core::config::{AppRuntimeConfig, load_runtime_config};
use rebecca::core::history::HistoryStore;
use rebecca::core::plan::{CleanupPlan, CleanupTarget, CleanupTargetIssueReason};
use rebecca::core::protection::ProtectionPolicy;
use rebecca::core::safety::{PATH_DOES_NOT_EXIST_REASON, is_reparse_like};
use rebecca::core::{CleanupWorkflow, DeleteMode, Platform, TargetStatus};
use serde::{Deserialize, Serialize};

use crate::clean::{execute_plan, merged_protected_paths};
use crate::cli::OutputMode;
use crate::output::{
    CliApiContract, HumanPlanRenderer, WorkflowOutputContract, format_bytes, format_shell_command,
};
use crate::runtime::CliRuntime;
use crate::{output, render};

const SAVED_PLAN_SCHEMA: &str = "rebecca.saved-cleanup-plan.v1";

#[derive(Debug)]
pub(crate) struct SavedPlanInspectOptions {
    pub(crate) output_mode: OutputMode,
    pub(crate) file: PathBuf,
}

#[derive(Debug)]
pub(crate) struct SavedPlanRunOptions {
    pub(crate) output_mode: OutputMode,
    pub(crate) file: PathBuf,
    pub(crate) yes: bool,
    pub(crate) permanent: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SavedCleanupPlan {
    pub(crate) schema: String,
    pub(crate) generated_by: String,
    pub(crate) generated_at_unix_seconds: u64,
    pub(crate) plan: CleanupPlan,
    pub(crate) target_fingerprints: Vec<SavedCleanupTargetFingerprint>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SavedCleanupTargetFingerprint {
    pub(crate) target_index: usize,
    pub(crate) rule_id: String,
    pub(crate) path: PathBuf,
    pub(crate) status: TargetStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) metadata: Option<SavedPathMetadata>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SavedPathMetadata {
    pub(crate) kind: SavedPathKind,
    pub(crate) len: u64,
    pub(crate) modified_at_unix_seconds: Option<u64>,
    pub(crate) readonly: bool,
    pub(crate) reparse_like: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum SavedPathKind {
    File,
    Directory,
    Symlink,
    Other,
}

pub(crate) fn write_saved_plan(plan: &CleanupPlan, path: &Path) -> Result<()> {
    let saved = SavedCleanupPlan::from_plan(plan)?;
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create plan directory {}", parent.display()))?;
    }
    let bytes = serde_json::to_vec_pretty(&saved).context("failed to encode saved cleanup plan")?;
    fs::write(path, bytes)
        .with_context(|| format!("failed to write saved cleanup plan {}", path.display()))?;
    Ok(())
}

pub(crate) fn inspect(options: SavedPlanInspectOptions) -> Result<()> {
    let saved = read_saved_plan(&options.file)?;
    saved.validate_schema()?;

    output::print_command_success_with_contract(
        CliApiContract::v1("plan inspect", "saved-cleanup-plan"),
        options.output_mode,
        || &saved,
        || {
            print_saved_plan_human(&saved, &options.file)?;
            Ok(())
        },
    )
}

pub(crate) fn run_with_runtime(options: SavedPlanRunOptions, runtime: &CliRuntime) -> Result<()> {
    if options.permanent && !options.yes {
        return Err(anyhow!("--permanent requires --yes"));
    }

    let saved = read_saved_plan(&options.file)?;
    let mode = if options.yes {
        if options.permanent {
            DeleteMode::PermanentDelete
        } else {
            DeleteMode::RecoverableDelete
        }
    } else {
        DeleteMode::DryRun
    };
    let mut plan = saved.revalidated_plan(mode)?;
    let contract = plan_run_contract(plan.request.workflow);
    let human_renderer = human_renderer_for_workflow(plan.request.workflow);

    if mode.is_dry_run() || plan.summary.allowed_targets == 0 {
        output::print_plan_with_events(
            &plan,
            contract,
            options.output_mode,
            human_renderer,
            None,
            None,
        )?;
        if options.output_mode.is_human() && !options.yes {
            print_plan_run_guidance(&options.file);
        }
        return Ok(());
    }

    let runtime_config = load_runtime_config()?;
    execute_saved_plan(&mut plan, &runtime_config, runtime, mode)?;
    output::print_plan_with_events(
        &plan,
        contract,
        options.output_mode,
        human_renderer,
        None,
        None,
    )
}

fn read_saved_plan(path: &Path) -> Result<SavedCleanupPlan> {
    let bytes = fs::read(path)
        .with_context(|| format!("failed to read saved cleanup plan {}", path.display()))?;
    serde_json::from_slice(&bytes)
        .with_context(|| format!("failed to parse saved cleanup plan {}", path.display()))
}

fn execute_saved_plan(
    plan: &mut CleanupPlan,
    runtime_config: &AppRuntimeConfig,
    runtime: &CliRuntime,
    mode: DeleteMode,
) -> Result<()> {
    let safety_knowledge =
        rebecca::rules::builtin_safety_knowledge_for_platform(plan.request.platform)?;
    let protected_storage = runtime_config.app_paths.storage_entries();
    let protected_paths = merged_protected_paths(runtime_config.protected_paths.as_slice(), &[])?;
    let mut execution_policy = ProtectionPolicy::new()
        .with_safety_knowledge(&safety_knowledge)
        .with_protected_storage(&protected_storage);
    if !protected_paths.is_empty() {
        execution_policy = execution_policy.with_protected_paths(&protected_paths);
    }

    let mut report = execute_plan(plan, execution_policy, runtime.cancellation(), mode)?;
    let history_append =
        HistoryStore::new(runtime_config.app_paths.history_file.clone()).append_plan_report(plan);
    if let Some(warning) = history_append.warning {
        eprintln!("Warning: {}", warning.message);
        report.push_warning(warning);
    }
    plan.execution_report = Some(report);
    Ok(())
}

impl SavedCleanupPlan {
    fn from_plan(plan: &CleanupPlan) -> Result<Self> {
        if !plan.request.mode.is_dry_run() {
            return Err(anyhow!("only preview plans can be saved"));
        }

        Ok(Self {
            schema: SAVED_PLAN_SCHEMA.to_string(),
            generated_by: format!("rebecca {}", env!("CARGO_PKG_VERSION")),
            generated_at_unix_seconds: unix_now(),
            plan: plan.clone(),
            target_fingerprints: plan
                .targets
                .iter()
                .enumerate()
                .map(|(target_index, target)| {
                    SavedCleanupTargetFingerprint::from_target(target_index, target)
                })
                .collect(),
        })
    }

    fn validate_schema(&self) -> Result<()> {
        if self.schema != SAVED_PLAN_SCHEMA {
            return Err(anyhow!(
                "unsupported saved plan schema {}; expected {SAVED_PLAN_SCHEMA}",
                self.schema
            ));
        }
        Ok(())
    }

    fn revalidated_plan(&self, mode: DeleteMode) -> Result<CleanupPlan> {
        self.validate_schema()?;
        if !self.plan.request.mode.is_dry_run() {
            return Err(anyhow!("saved cleanup plan must contain a dry-run plan"));
        }
        if self.plan.request.platform != Platform::current() {
            return Err(anyhow!(
                "saved plan was created for {} but this host is {}",
                self.plan.request.platform.label(),
                Platform::current().label()
            ));
        }

        let mut plan = self.plan.clone();
        plan.request.mode = mode;
        plan.execution_report = None;
        let fingerprints = self.fingerprints_by_index();
        for (target_index, target) in plan.targets.iter_mut().enumerate() {
            target.mode = mode;
            target.freed_bytes = 0;
            target.pending_reclaim_bytes = 0;
            if target.status.is_executable() {
                revalidate_saved_target(target_index, target, fingerprints.get(&target_index));
            }
        }
        plan.recompute_summary();
        Ok(plan)
    }

    fn fingerprints_by_index(&self) -> BTreeMap<usize, &SavedCleanupTargetFingerprint> {
        self.target_fingerprints
            .iter()
            .map(|fingerprint| (fingerprint.target_index, fingerprint))
            .collect()
    }
}

impl SavedCleanupTargetFingerprint {
    fn from_target(target_index: usize, target: &CleanupTarget) -> Self {
        Self {
            target_index,
            rule_id: target.rule_id.clone(),
            path: target.path.clone(),
            status: target.status,
            metadata: SavedPathMetadata::from_path(&target.path),
        }
    }
}

impl SavedPathMetadata {
    fn from_path(path: &Path) -> Option<Self> {
        let metadata = fs::symlink_metadata(path).ok()?;
        Some(Self {
            kind: SavedPathKind::from_metadata(&metadata),
            len: metadata.len(),
            modified_at_unix_seconds: metadata.modified().ok().and_then(system_time_to_unix),
            readonly: metadata.permissions().readonly(),
            reparse_like: is_reparse_like(&metadata),
        })
    }

    fn mismatch_reason(&self, current: &Self) -> Option<String> {
        if self.reparse_like || current.reparse_like {
            return Some("target is or became a symlink/reparse point".to_string());
        }
        if self.kind != current.kind {
            return Some(format!(
                "target type changed from {} to {}",
                self.kind.label(),
                current.kind.label()
            ));
        }
        if self.kind == SavedPathKind::File && self.len != current.len {
            return Some(format!(
                "file length changed from {} to {} bytes",
                self.len, current.len
            ));
        }
        if self.modified_at_unix_seconds.is_some()
            && current.modified_at_unix_seconds.is_some()
            && self.modified_at_unix_seconds != current.modified_at_unix_seconds
        {
            return Some("target modification time changed".to_string());
        }
        None
    }
}

impl SavedPathKind {
    fn from_metadata(metadata: &fs::Metadata) -> Self {
        let file_type = metadata.file_type();
        if file_type.is_symlink() {
            Self::Symlink
        } else if file_type.is_dir() {
            Self::Directory
        } else if file_type.is_file() {
            Self::File
        } else {
            Self::Other
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Directory => "directory",
            Self::Symlink => "symlink",
            Self::Other => "other",
        }
    }
}

fn revalidate_saved_target(
    target_index: usize,
    target: &mut CleanupTarget,
    fingerprint: Option<&&SavedCleanupTargetFingerprint>,
) {
    let Some(fingerprint) = fingerprint.copied() else {
        mark_saved_plan_target_changed(target, "saved plan is missing the target fingerprint");
        return;
    };

    if fingerprint.rule_id != target.rule_id
        || fingerprint.path != target.path
        || fingerprint.status != TargetStatus::Allowed
    {
        mark_saved_plan_target_changed(
            target,
            format!("saved plan target {target_index} no longer matches its fingerprint"),
        );
        return;
    }

    let Some(saved_metadata) = &fingerprint.metadata else {
        mark_saved_plan_target_changed(target, "saved plan target was not fingerprinted");
        return;
    };
    let Some(current_metadata) = SavedPathMetadata::from_path(&target.path) else {
        target.mark_skipped_with_reason_code(
            CleanupTargetIssueReason::ExecutionTargetMissing,
            PATH_DOES_NOT_EXIST_REASON,
        );
        return;
    };
    if current_metadata.reparse_like {
        target.mark_blocked_with_reason_code(
            CleanupTargetIssueReason::SafetyPolicyBlocked,
            "saved plan target became a symlink or reparse point",
        );
        return;
    }
    if let Some(reason) = saved_metadata.mismatch_reason(&current_metadata) {
        mark_saved_plan_target_changed(target, reason);
    }
}

fn mark_saved_plan_target_changed(target: &mut CleanupTarget, reason: impl Into<String>) {
    target.mark_skipped_with_reason_code(CleanupTargetIssueReason::SavedPlanTargetChanged, reason);
}

fn print_saved_plan_human(saved: &SavedCleanupPlan, file: &Path) -> Result<()> {
    println!("Saved cleanup plan");
    println!("File: {}", file.display());
    println!("Schema: {}", saved.schema);
    println!("Generated by: {}", saved.generated_by);
    println!(
        "Generated at unix seconds: {}",
        saved.generated_at_unix_seconds
    );
    println!("Platform: {}", saved.plan.request.platform.label());
    println!("Workflow: {}", saved.plan.request.workflow.title());
    println!(
        "Targets: {} total, {} eligible, {} ({}) estimated",
        saved.plan.summary.total_targets,
        saved.plan.summary.allowed_targets,
        saved.plan.summary.estimated_bytes,
        format_bytes(saved.plan.summary.estimated_bytes)
    );
    println!(
        "Revalidate: {}",
        format_shell_command(
            "rebecca",
            &[
                "plan".to_string(),
                "run".to_string(),
                file.display().to_string()
            ],
        )
    );
    println!(
        "Execute: {}",
        format_shell_command(
            "rebecca",
            &[
                "plan".to_string(),
                "run".to_string(),
                file.display().to_string(),
                "--yes".to_string(),
            ],
        )
    );
    Ok(())
}

fn print_plan_run_guidance(file: &Path) {
    println!();
    println!(
        "Execute saved plan: {}",
        format_shell_command(
            "rebecca",
            &[
                "plan".to_string(),
                "run".to_string(),
                file.display().to_string(),
                "--yes".to_string(),
            ],
        )
    );
    println!(
        "Skip the trash: {}",
        format_shell_command(
            "rebecca",
            &[
                "plan".to_string(),
                "run".to_string(),
                file.display().to_string(),
                "--yes".to_string(),
                "--permanent".to_string(),
            ],
        )
    );
}

fn plan_run_contract(workflow: CleanupWorkflow) -> WorkflowOutputContract {
    WorkflowOutputContract::v1("plan run", payload_kind_for_workflow(workflow))
}

fn payload_kind_for_workflow(workflow: CleanupWorkflow) -> &'static str {
    match workflow {
        CleanupWorkflow::Rules => "cleanup-plan",
        CleanupWorkflow::AppLeftovers => "app-leftovers-cleanup-plan",
        CleanupWorkflow::ProjectArtifacts => "project-artifact-cleanup-plan",
    }
}

fn human_renderer_for_workflow(workflow: CleanupWorkflow) -> HumanPlanRenderer {
    match workflow {
        CleanupWorkflow::Rules | CleanupWorkflow::AppLeftovers => render::clean::print_plan,
        CleanupWorkflow::ProjectArtifacts => render::purge::print_plan,
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn system_time_to_unix(time: SystemTime) -> Option<u64> {
    time.duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .ok()
}

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use indicatif::ProgressBar;
use rebecca::core::config::{AppRuntimeConfig, AppStorageEntry};
use rebecca::core::environment::SystemEnvironment;
use rebecca::core::external_rules::ExternalRuleStore;
use rebecca::core::plan::CleanupPlan;
use rebecca::core::planner::{
    PlanBuildContext, PlanProgressEvent, build_cleanup_plan_with_context,
};
use rebecca::core::protection::ProtectionPolicy;
use rebecca::core::safety_catalog::SafetyKnowledge;
use rebecca::core::scan::ScanBackendKind;
use rebecca::core::scan_cache::ScanCacheStore;
use rebecca::core::{PlanRequest, RebeccaError, RuleDefinition, TargetStatus};

use crate::clean_view::ScanCacheProgressSummary;
use crate::cli::{OutputMode, ProgressDetail};
use crate::output::{NdjsonEventWriter, WorkflowOutputContract, format_bytes};
use crate::progress::{
    HumanProgressThrottle, PROGRESS_PATH_MAX_CHARS, compact_progress_path, format_scan_counters,
    stderr_spinner,
};
use crate::runtime::CliRuntime;
use crate::text::format_count;

#[derive(Debug, Clone, Copy)]
pub(crate) enum WorkflowRuleSource<'a> {
    RuleCatalog(&'a [RuleDefinition]),
    NativeWorkflow,
}

impl<'a> WorkflowRuleSource<'a> {
    fn rules(self) -> &'a [RuleDefinition] {
        match self {
            Self::RuleCatalog(rules) => rules,
            Self::NativeWorkflow => &[],
        }
    }
}

pub(crate) struct WorkflowPlanBuildOptions<'a> {
    pub(crate) request: &'a PlanRequest,
    pub(crate) rule_source: WorkflowRuleSource<'a>,
    pub(crate) runtime_config: &'a AppRuntimeConfig,
    pub(crate) runtime: &'a CliRuntime,
    pub(crate) output_mode: OutputMode,
    pub(crate) no_progress: bool,
    pub(crate) progress_detail: ProgressDetail,
    pub(crate) scan_cache: bool,
    pub(crate) scan_backend: ScanBackendKind,
    pub(crate) exclude_paths: &'a [PathBuf],
    pub(crate) output_contract: WorkflowOutputContract,
}

pub(crate) enum WorkflowPlanBuildOutcome {
    Built(Box<WorkflowPlanBuild>),
    PlannerError {
        err: RebeccaError,
        event_writer: Option<NdjsonEventWriter>,
    },
}

pub(crate) struct WorkflowPlanBuild {
    pub(crate) plan: CleanupPlan,
    pub(crate) scan_cache_summary: Option<ScanCacheProgressSummary>,
    pub(crate) event_writer: Option<NdjsonEventWriter>,
    pub(crate) execution_guards: WorkflowExecutionGuards,
}

pub(crate) struct WorkflowExecutionGuards {
    safety_knowledge: SafetyKnowledge,
    protected_storage: Vec<AppStorageEntry>,
    protected_paths: Vec<PathBuf>,
}

impl WorkflowExecutionGuards {
    pub(crate) fn for_request(
        request: &PlanRequest,
        runtime_config: &AppRuntimeConfig,
        exclude_paths: &[PathBuf],
    ) -> Result<Self> {
        Ok(Self {
            safety_knowledge: rebecca::rules::builtin_safety_knowledge_for_platform(
                request.platform,
            )?,
            protected_storage: runtime_config.app_paths.storage_entries(),
            protected_paths: merged_protected_paths(
                runtime_config.protected_paths.as_slice(),
                exclude_paths,
            )?,
        })
    }

    pub(crate) fn protection_policy(&self) -> ProtectionPolicy<'_> {
        let mut policy = ProtectionPolicy::new()
            .with_safety_knowledge(&self.safety_knowledge)
            .with_protected_storage(&self.protected_storage);
        if !self.protected_paths.is_empty() {
            policy = policy.with_protected_paths(&self.protected_paths);
        }
        policy
    }
}

pub(crate) fn build_workflow_plan(
    options: WorkflowPlanBuildOptions<'_>,
) -> Result<WorkflowPlanBuildOutcome> {
    let execution_guards = WorkflowExecutionGuards::for_request(
        options.request,
        options.runtime_config,
        options.exclude_paths,
    )?;
    let mut progress = PlanProgressReporter::new(
        options.output_mode.is_human() && !options.no_progress,
        options.progress_detail,
        options
            .output_mode
            .is_ndjson()
            .then(|| NdjsonEventWriter::with_contract(options.output_contract)),
    );
    let applications = crate::info::application_discovery();
    let scan_cache_store = options
        .scan_cache
        .then(|| ScanCacheStore::from_app_paths(&options.runtime_config.app_paths));
    let mut context = PlanBuildContext::new(options.runtime.cancellation())
        .with_scan_backend(options.scan_backend)
        .with_safety_knowledge(&execution_guards.safety_knowledge)
        .with_protected_storage(&execution_guards.protected_storage);
    if !execution_guards.protected_paths.is_empty() {
        context = context.with_protected_paths(&execution_guards.protected_paths);
    }
    if options.scan_cache {
        context = context.with_scan_cache_policy(options.runtime_config.scan_cache_policy);
        if let Some(store) = &scan_cache_store {
            context = context.with_scan_cache(store);
        }
    }
    progress.started()?;
    let combined_rules;
    let rules = match options.rule_source {
        WorkflowRuleSource::RuleCatalog(rules) => {
            let external_rules = ExternalRuleStore::default_for_state_dir(
                &options.runtime_config.app_paths.state_dir,
            )
            .load_enabled_rules();
            for diagnostic in &external_rules.diagnostics {
                eprintln!("Warning: external rule skipped: {}", diagnostic.message);
            }
            if external_rules.rules.is_empty() {
                rules
            } else {
                combined_rules = rules
                    .iter()
                    .cloned()
                    .chain(external_rules.rules)
                    .collect::<Vec<_>>();
                &combined_rules
            }
        }
        WorkflowRuleSource::NativeWorkflow => options.rule_source.rules(),
    };
    let plan_result = build_cleanup_plan_with_context(
        options.request,
        rules,
        &SystemEnvironment,
        applications.as_ref(),
        context,
        |event| progress.on_event(event),
    );
    progress.finish();
    if let Some(err) = progress.take_event_error() {
        return Err(err);
    }

    let scan_cache_summary = options
        .output_mode
        .is_human()
        .then(|| progress.scan_cache_summary());
    let event_writer = progress.into_event_writer();

    match plan_result {
        Ok(plan) => Ok(WorkflowPlanBuildOutcome::Built(Box::new(
            WorkflowPlanBuild {
                plan,
                scan_cache_summary,
                event_writer,
                execution_guards,
            },
        ))),
        Err(err) => Ok(WorkflowPlanBuildOutcome::PlannerError { err, event_writer }),
    }
}

pub(crate) fn merged_protected_paths(
    config_paths: &[PathBuf],
    cli_paths: &[PathBuf],
) -> Result<Vec<PathBuf>> {
    let mut merged = Vec::with_capacity(config_paths.len() + cli_paths.len());
    for path in config_paths.iter().chain(cli_paths) {
        rebecca::core::config::validate_user_protected_path(path)
            .map_err(|message| anyhow!("invalid protected path {}: {message}", path.display()))?;
        if merged.iter().all(|existing| existing != path) {
            merged.push(path.clone());
        }
    }
    Ok(merged)
}

struct PlanProgressReporter {
    bar: Option<ProgressBar>,
    detail: ProgressDetail,
    event_writer: Option<NdjsonEventWriter>,
    event_error: Option<anyhow::Error>,
    scanned_targets: u64,
    planned_bytes: u64,
    current_target_started_at: Instant,
    human_file_progress: HumanProgressThrottle,
    scan_cache_summary: ScanCacheProgressSummary,
}

impl PlanProgressReporter {
    fn new(enabled: bool, detail: ProgressDetail, event_writer: Option<NdjsonEventWriter>) -> Self {
        let now = Instant::now();
        let bar = stderr_spinner(enabled, "plan | building cleanup plan | Ctrl+C cancels");

        Self {
            bar,
            detail,
            event_writer,
            event_error: None,
            scanned_targets: 0,
            planned_bytes: 0,
            current_target_started_at: now,
            human_file_progress: HumanProgressThrottle::new(),
            scan_cache_summary: ScanCacheProgressSummary::default(),
        }
    }

    fn started(&mut self) -> Result<()> {
        if let Some(writer) = &mut self.event_writer {
            writer.emit_started()?;
        }
        Ok(())
    }

    fn on_event(&mut self, event: PlanProgressEvent<'_>) {
        self.record_event(event);

        if self.event_error.is_none()
            && let Some(writer) = &mut self.event_writer
            && self.detail.should_emit_machine_event(event)
            && let Err(err) = writer.emit_plan_progress(event)
        {
            self.event_error = Some(err);
        }

        let Some(bar) = &self.bar else {
            return;
        };

        match event {
            PlanProgressEvent::TargetScanning { rule_id, path } => {
                self.current_target_started_at = Instant::now();
                bar.set_message(target_scanning_message(
                    self.scanned_targets.saturating_add(1),
                    rule_id,
                    path,
                ));
            }
            PlanProgressEvent::TargetFinished {
                status,
                estimated_bytes,
                ..
            } => {
                self.scanned_targets = self.scanned_targets.saturating_add(1);
                self.planned_bytes = self.planned_bytes.saturating_add(estimated_bytes);
                bar.set_message(target_finished_message(
                    self.scanned_targets,
                    self.planned_bytes,
                    status,
                    estimated_bytes,
                ));
                bar.tick();
            }
            PlanProgressEvent::FileMeasured {
                rule_id,
                files_scanned,
                bytes_scanned,
                ..
            } => {
                if self.detail.includes_file_events() && self.human_file_progress.should_refresh() {
                    bar.set_message(file_progress_message(
                        rule_id,
                        files_scanned,
                        bytes_scanned,
                        self.current_target_started_at.elapsed(),
                    ));
                }
            }
            PlanProgressEvent::ScanCacheHit {
                rule_id,
                path,
                estimated_bytes,
            } => {
                bar.set_message(scan_cache_hit_message(
                    self.scan_cache_summary,
                    rule_id,
                    path,
                    estimated_bytes,
                ));
                bar.tick();
            }
            PlanProgressEvent::ScanCacheMiss {
                rule_id,
                path,
                reason,
                ..
            } => {
                bar.set_message(scan_cache_miss_message(
                    self.scan_cache_summary,
                    rule_id,
                    path,
                    reason.label(),
                ));
                bar.tick();
            }
            PlanProgressEvent::ScanCacheWriteSkipped { rule_id, path } => {
                bar.set_message(scan_cache_write_skipped_message(
                    self.scan_cache_summary,
                    rule_id,
                    path,
                ));
                bar.tick();
            }
            PlanProgressEvent::ScanCachePruned { report } => {
                bar.set_message(scan_cache_pruned_message(
                    self.scan_cache_summary,
                    report.pruned as u64,
                    report.inspected as u64,
                ));
                bar.tick();
            }
        }
    }

    fn record_event(&mut self, event: PlanProgressEvent<'_>) {
        match event {
            PlanProgressEvent::ScanCacheHit { .. } => {
                self.scan_cache_summary.hits = self.scan_cache_summary.hits.saturating_add(1);
            }
            PlanProgressEvent::ScanCacheMiss { pruned, .. } => {
                self.scan_cache_summary.misses = self.scan_cache_summary.misses.saturating_add(1);
                if pruned {
                    self.scan_cache_summary.pruned =
                        self.scan_cache_summary.pruned.saturating_add(1);
                }
            }
            PlanProgressEvent::ScanCacheWriteSkipped { .. } => {
                self.scan_cache_summary.write_skipped =
                    self.scan_cache_summary.write_skipped.saturating_add(1);
            }
            PlanProgressEvent::ScanCachePruned { report } => {
                self.scan_cache_summary.pruned = self
                    .scan_cache_summary
                    .pruned
                    .saturating_add(report.pruned as u64);
            }
            _ => {}
        }
    }

    fn scan_cache_summary(&self) -> ScanCacheProgressSummary {
        self.scan_cache_summary
    }

    fn finish(&self) {
        if let Some(bar) = &self.bar {
            bar.finish_and_clear();
        }
    }

    fn take_event_error(&mut self) -> Option<anyhow::Error> {
        self.event_error.take()
    }

    fn into_event_writer(self) -> Option<NdjsonEventWriter> {
        self.event_writer
    }
}

fn target_scanning_message(next_target: u64, rule_id: &str, path: &Path) -> String {
    format!(
        "plan | target {next_target} | scanning {rule_id} | {} | Ctrl+C cancels",
        compact_progress_path(path, PROGRESS_PATH_MAX_CHARS)
    )
}

fn target_finished_message(
    scanned_targets: u64,
    planned_bytes: u64,
    status: TargetStatus,
    estimated_bytes: u64,
) -> String {
    format!(
        "plan | {} | {} found | last {}, {}",
        format_count(scanned_targets, "target", "targets"),
        format_bytes(planned_bytes),
        status.label(),
        format_bytes(estimated_bytes)
    )
}

fn file_progress_message(
    rule_id: &str,
    files_scanned: u64,
    bytes_scanned: u64,
    elapsed: Duration,
) -> String {
    format!(
        "scan | {rule_id} | {}",
        format_scan_counters(files_scanned, 0, bytes_scanned, elapsed)
    )
}

fn scan_cache_hit_message(
    summary: ScanCacheProgressSummary,
    rule_id: &str,
    path: &Path,
    estimated_bytes: u64,
) -> String {
    format!(
        "cache | {} | hit {rule_id} | {} | {}",
        scan_cache_counts(summary),
        compact_progress_path(path, PROGRESS_PATH_MAX_CHARS),
        format_bytes(estimated_bytes)
    )
}

fn scan_cache_miss_message(
    summary: ScanCacheProgressSummary,
    rule_id: &str,
    path: &Path,
    reason_label: &str,
) -> String {
    format!(
        "cache | {} | miss {rule_id} | {} | {reason_label}",
        scan_cache_counts(summary),
        compact_progress_path(path, PROGRESS_PATH_MAX_CHARS)
    )
}

fn scan_cache_write_skipped_message(
    summary: ScanCacheProgressSummary,
    rule_id: &str,
    path: &Path,
) -> String {
    format!(
        "cache | {} | skip write {rule_id} | {}",
        scan_cache_counts(summary),
        compact_progress_path(path, PROGRESS_PATH_MAX_CHARS)
    )
}

fn scan_cache_pruned_message(
    summary: ScanCacheProgressSummary,
    pruned: u64,
    inspected: u64,
) -> String {
    format!(
        "cache | {} | pruned {} after {}",
        scan_cache_counts(summary),
        format_count(pruned, "record", "records"),
        format_count(inspected, "inspection", "inspections")
    )
}

fn scan_cache_counts(summary: ScanCacheProgressSummary) -> String {
    format!(
        "{}, {}",
        format_count(summary.hits, "hit", "hits"),
        format_count(summary.misses, "miss", "misses")
    )
}

impl ProgressDetail {
    fn should_emit_machine_event(self, event: PlanProgressEvent<'_>) -> bool {
        !matches!(event, PlanProgressEvent::FileMeasured { .. }) || self.includes_file_events()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_target_scanning_message_keeps_path_bounded() {
        let long_path = Path::new(
            r"C:\Users\Rebecca\AppData\Local\VeryLongVendorName\VeryLongProductName\Cache\Nested\Target",
        );

        let message = target_scanning_message(3, "windows.user-temp", long_path);

        assert!(message.starts_with("plan | target 3 | scanning windows.user-temp | "));
        let path = message
            .strip_prefix("plan | target 3 | scanning windows.user-temp | ")
            .expect("progress message should keep the path as the final segment");
        let path = path
            .strip_suffix(" | Ctrl+C cancels")
            .expect("progress message should include cancellation hint");
        assert!(path.starts_with("..."));
        assert!(path.ends_with(r"\Cache\Nested\Target"));
        assert!(path.chars().count() <= PROGRESS_PATH_MAX_CHARS);
    }

    #[test]
    fn progress_target_finished_message_summarizes_targets_and_bytes() {
        let message = target_finished_message(2, 1536, TargetStatus::Allowed, 512);

        assert_eq!(
            message,
            "plan | 2 targets | 1.50 KiB found | last allowed, 512 B"
        );
    }

    #[test]
    fn progress_file_message_includes_scan_rates() {
        let message = file_progress_message("windows.user-temp", 4, 20, Duration::from_secs(1));

        assert_eq!(
            message,
            "scan | windows.user-temp | 4 files | 0 dirs | 20 B | 4.0 files/s, 20 B/s"
        );
    }

    #[test]
    fn progress_cache_hit_message_includes_cache_counts() {
        let summary = ScanCacheProgressSummary {
            hits: 1,
            misses: 0,
            write_skipped: 0,
            pruned: 0,
        };

        let message =
            scan_cache_hit_message(summary, "windows.edge-cache", Path::new(r"C:\Cache"), 2048);

        assert_eq!(
            message,
            r"cache | 1 hit, 0 misses | hit windows.edge-cache | C:\Cache | 2.00 KiB"
        );
    }

    #[test]
    fn compact_progress_text_handles_tiny_widths() {
        assert_eq!(crate::progress::compact_progress_text("abcdef", 2), "..");
        assert_eq!(crate::progress::compact_progress_text("abcdef", 4), "...f");
    }
}

use anyhow::Result;
use rebecca::core::RuleDefinition;
use rebecca::core::plan::{CleanupIssueSummary, CleanupPlan};
use rebecca::core::planner::PlanProgressEvent;
use serde::Serialize;
use serde_json::json;
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::clean_view::ScanCacheProgressSummary;
use crate::cli::OutputMode;
use crate::text::format_count;

const API_VERSION: &str = "rebecca.cli.v1";

pub(crate) type HumanPlanRenderer =
    fn(&CleanupPlan, Option<ScanCacheProgressSummary>) -> Result<()>;
pub(crate) type WorkflowSuccessRenderer = fn(
    &CleanupPlan,
    WorkflowOutputContract,
    OutputMode,
    HumanPlanRenderer,
    Option<ScanCacheProgressSummary>,
    Option<NdjsonEventWriter>,
) -> Result<()>;

#[derive(Debug, Clone, Copy)]
pub(crate) struct WorkflowOutputContract {
    pub(crate) command: &'static str,
    pub(crate) payload_kind: &'static str,
}

#[derive(Debug, Serialize)]
struct SuccessEnvelope<'a, T: Serialize + ?Sized> {
    api_version: &'static str,
    kind: &'static str,
    command: &'a str,
    payload_kind: &'a str,
    generated_at_unix_seconds: u64,
    data: &'a T,
}

#[derive(Debug, Serialize)]
struct ErrorEnvelope<'a> {
    api_version: &'static str,
    kind: &'static str,
    command: &'a str,
    generated_at_unix_seconds: u64,
    error: ApiErrorBody<'a>,
}

#[derive(Debug, Serialize)]
struct ApiErrorBody<'a> {
    code: &'static str,
    title: &'static str,
    detail: String,
    exit_code: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<&'a str>,
}

#[derive(Debug, Serialize)]
struct ErrorEventEnvelope<'a> {
    api_version: &'static str,
    kind: &'static str,
    command: &'a str,
    sequence: u64,
    event_kind: &'static str,
    generated_at_unix_seconds: u64,
    error: ApiErrorBody<'a>,
}

#[derive(Debug)]
pub(crate) struct MachineErrorRendered;

impl fmt::Display for MachineErrorRendered {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("machine-readable error already rendered")
    }
}

impl std::error::Error for MachineErrorRendered {}

pub(crate) fn print_success<T: Serialize + ?Sized>(
    command: &str,
    payload_kind: &str,
    data: &T,
) -> Result<()> {
    let envelope = SuccessEnvelope {
        api_version: API_VERSION,
        kind: "success",
        command,
        payload_kind,
        generated_at_unix_seconds: unix_now(),
        data,
    };
    println!("{}", serde_json::to_string_pretty(&envelope)?);
    Ok(())
}

pub(crate) fn print_command_success<T, P, H>(
    command: &'static str,
    payload_kind: &'static str,
    mode: OutputMode,
    payload: P,
    print_human: H,
) -> Result<()>
where
    T: Serialize,
    P: FnOnce() -> T,
    H: FnOnce() -> Result<()>,
{
    match mode {
        OutputMode::Human => print_human(),
        OutputMode::Json | OutputMode::Ndjson => {
            let data = payload();
            print_machine_success_payload(command, payload_kind, mode, &data)
        }
    }
}

pub(crate) fn render_error(command: &str, mode: OutputMode, err: &anyhow::Error) {
    if mode.is_human() {
        eprintln!("{err:#}");
        return;
    }

    let error = classify_error(err);

    match mode {
        OutputMode::Human => unreachable!("human mode handled above"),
        OutputMode::Json => {
            let envelope = ErrorEnvelope {
                api_version: API_VERSION,
                kind: "error",
                command,
                generated_at_unix_seconds: unix_now(),
                error,
            };
            match serde_json::to_string_pretty(&envelope) {
                Ok(rendered) => eprintln!("{rendered}"),
                Err(render_err) => eprintln!("{render_err}"),
            }
        }
        OutputMode::Ndjson => {
            let envelope = ErrorEventEnvelope {
                api_version: API_VERSION,
                kind: "event",
                command,
                sequence: 0,
                event_kind: "error",
                generated_at_unix_seconds: unix_now(),
                error,
            };
            match serde_json::to_string(&envelope) {
                Ok(rendered) => println!("{rendered}"),
                Err(render_err) => eprintln!("{render_err}"),
            }
        }
    }
}

fn classify_error(err: &anyhow::Error) -> ApiErrorBody<'static> {
    let detail = err.to_string();

    if let Some(core_error) = err.downcast_ref::<rebecca::core::RebeccaError>() {
        let (code, title) = match core_error {
            rebecca::core::RebeccaError::InvalidRuleId(_) => {
                ("invalid-rule-id", "Invalid cleanup rule")
            }
            rebecca::core::RebeccaError::InvalidCategory(_) => {
                ("invalid-category", "Invalid cleanup category")
            }
            rebecca::core::RebeccaError::InvalidProjectArtifactSelector(_) => (
                "invalid-project-artifact-selector",
                "Invalid project artifact selector",
            ),
            rebecca::core::RebeccaError::ConfigRead { .. } => {
                ("config-read-failed", "Configuration read failed")
            }
            rebecca::core::RebeccaError::ConfigParse { .. } => {
                ("config-parse-failed", "Configuration parse failed")
            }
            rebecca::core::RebeccaError::HistoryCorrupted(_) => {
                ("history-corrupted", "History record corrupted")
            }
            rebecca::core::RebeccaError::HistoryUnavailable(_) => {
                ("history-unavailable", "History unavailable")
            }
            rebecca::core::RebeccaError::PlatformUnavailable(_) => {
                ("platform-unavailable", "Platform unavailable")
            }
            rebecca::core::RebeccaError::OperationCancelled(_) => {
                ("operation-cancelled", "Operation cancelled")
            }
            rebecca::core::RebeccaError::ScanFailed(_) => ("scan-failed", "Scan failed"),
            rebecca::core::RebeccaError::ScanCacheUnavailable(_) => {
                ("scan-cache-unavailable", "Scan cache unavailable")
            }
            rebecca::core::RebeccaError::SafetyBlocked(_) => {
                ("safety-blocked", "Safety policy blocked cleanup")
            }
            rebecca::core::RebeccaError::ExecutionFailed(_) => {
                ("execution-failed", "Cleanup execution failed")
            }
            rebecca::core::RebeccaError::PathExpansionFailed(_) => {
                ("path-expansion-failed", "Path expansion failed")
            }
            rebecca::core::RebeccaError::ApplicationDiscoveryFailed(_) => (
                "application-discovery-failed",
                "Application discovery failed",
            ),
            rebecca::core::RebeccaError::RuleCatalogInvalid(_) => {
                ("rule-catalog-invalid", "Rule catalog invalid")
            }
            rebecca::core::RebeccaError::Json(_) => ("json-error", "JSON processing failed"),
            rebecca::core::RebeccaError::Io(_) => ("io-error", "I/O failed"),
            rebecca::core::RebeccaError::UserDirsUnavailable => {
                ("user-dirs-unavailable", "User directories unavailable")
            }
        };

        return ApiErrorBody {
            code,
            title,
            detail,
            exit_code: 1,
            source: Some("rebecca-core"),
        };
    }

    let (code, title) = if detail.contains("invalid protected path") {
        ("validation-error", "Validation failed")
    } else if detail.contains("purge root") {
        ("invalid-purge-root", "Invalid purge root")
    } else {
        ("internal-error", "Command failed")
    };

    ApiErrorBody {
        code,
        title,
        detail,
        exit_code: 1,
        source: None,
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

pub fn print_rule_catalog(rules: &[&RuleDefinition]) {
    println!("Rebecca rules: {}", rules.len());

    if rules.is_empty() {
        println!("No built-in rules match the current selection.");
        return;
    }

    let mut grouped: std::collections::BTreeMap<String, Vec<&RuleDefinition>> =
        std::collections::BTreeMap::new();
    for rule in rules {
        grouped
            .entry(rule.category.clone())
            .or_default()
            .push(*rule);
    }

    for rules in grouped.values_mut() {
        rules.sort_by(|left, right| left.id.cmp(&right.id));
    }

    for (category, rules) in grouped {
        println!("- {} ({})", category, rules.len());
        for rule in rules {
            println!(
                "  - {} [{}] {}{}",
                rule.id,
                rule.safety_level.label(),
                rule.name,
                restore_hint_suffix(rule.restore_hint.as_deref())
            );
        }
    }
}

pub(crate) fn restore_hint_suffix<I, S>(restore_hints: I) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut unique_hints = Vec::new();

    for hint in restore_hints {
        let hint = hint.as_ref();
        if !unique_hints.iter().any(|existing| existing == hint) {
            unique_hints.push(hint.to_string());
        }
    }

    if unique_hints.is_empty() {
        String::new()
    } else {
        format!(" [restore: {}]", unique_hints.join("; "))
    }
}

pub(crate) fn print_plan_with_events(
    plan: &CleanupPlan,
    contract: WorkflowOutputContract,
    mode: OutputMode,
    human_renderer: HumanPlanRenderer,
    scan_cache_summary: Option<ScanCacheProgressSummary>,
    event_writer: Option<NdjsonEventWriter>,
) -> Result<()> {
    print_workflow_success_payload(
        plan,
        plan,
        contract,
        mode,
        human_renderer,
        scan_cache_summary,
        event_writer,
    )
}

pub(crate) fn print_workflow_success_payload<T: Serialize + ?Sized>(
    plan: &CleanupPlan,
    payload: &T,
    contract: WorkflowOutputContract,
    mode: OutputMode,
    human_renderer: HumanPlanRenderer,
    scan_cache_summary: Option<ScanCacheProgressSummary>,
    event_writer: Option<NdjsonEventWriter>,
) -> Result<()> {
    match mode {
        OutputMode::Human => human_renderer(plan, scan_cache_summary),
        OutputMode::Json | OutputMode::Ndjson => {
            print_machine_workflow_success_payload(contract, mode, payload, event_writer)
        }
    }
}

fn print_machine_success_payload<T: Serialize + ?Sized>(
    command: &'static str,
    payload_kind: &'static str,
    mode: OutputMode,
    payload: &T,
) -> Result<()> {
    match mode {
        OutputMode::Human => unreachable!("human mode is rendered by the caller"),
        OutputMode::Json => print_success(command, payload_kind, payload),
        OutputMode::Ndjson => print_success_event(command, payload_kind, payload),
    }
}

fn print_machine_workflow_success_payload<T: Serialize + ?Sized>(
    contract: WorkflowOutputContract,
    mode: OutputMode,
    payload: &T,
    event_writer: Option<NdjsonEventWriter>,
) -> Result<()> {
    match mode {
        OutputMode::Human => unreachable!("human mode is rendered by the caller"),
        OutputMode::Json => print_success(contract.command, contract.payload_kind, payload),
        OutputMode::Ndjson => {
            let mut writer =
                event_writer.unwrap_or_else(|| NdjsonEventWriter::new(contract.command));
            writer.emit_completed(contract.payload_kind, payload)
        }
    }
}

pub(crate) fn print_success_event<T: Serialize + ?Sized>(
    command: &'static str,
    payload_kind: &str,
    data: &T,
) -> Result<()> {
    let mut writer = NdjsonEventWriter::new(command);
    writer.emit_completed(payload_kind, data)
}

#[derive(Debug, Default)]
pub(crate) struct NdjsonEventWriter {
    command: &'static str,
    next_sequence: u64,
}

impl NdjsonEventWriter {
    pub(crate) fn new(command: &'static str) -> Self {
        Self {
            command,
            next_sequence: 0,
        }
    }

    pub(crate) fn emit_started(&mut self) -> Result<()> {
        self.emit_data("started", json!({}))
    }

    pub(crate) fn emit_plan_progress(&mut self, event: PlanProgressEvent<'_>) -> Result<()> {
        match event {
            PlanProgressEvent::TargetScanning { rule_id, path } => self.emit_data(
                "target-scanning",
                json!({
                    "rule_id": rule_id,
                    "path": path,
                }),
            ),
            PlanProgressEvent::TargetFinished {
                rule_id,
                path,
                status,
                estimated_bytes,
            } => self.emit_data(
                "target-finished",
                json!({
                    "rule_id": rule_id,
                    "path": path,
                    "status": status,
                    "estimated_bytes": estimated_bytes,
                }),
            ),
            PlanProgressEvent::FileMeasured {
                rule_id,
                target_path,
                path,
                file_size,
                files_scanned,
                bytes_scanned,
            } => self.emit_data(
                "file-measured",
                json!({
                    "rule_id": rule_id,
                    "target_path": target_path,
                    "path": path,
                    "file_size": file_size,
                    "files_scanned": files_scanned,
                    "bytes_scanned": bytes_scanned,
                }),
            ),
            PlanProgressEvent::ScanCacheHit {
                rule_id,
                path,
                estimated_bytes,
            } => self.emit_data(
                "scan-cache-hit",
                json!({
                    "rule_id": rule_id,
                    "path": path,
                    "estimated_bytes": estimated_bytes,
                }),
            ),
            PlanProgressEvent::ScanCacheMiss {
                rule_id,
                path,
                reason,
                pruned,
            } => self.emit_data(
                "scan-cache-miss",
                json!({
                    "rule_id": rule_id,
                    "path": path,
                    "reason": reason.label(),
                    "pruned": pruned,
                }),
            ),
            PlanProgressEvent::ScanCacheWriteSkipped { rule_id, path } => self.emit_data(
                "scan-cache-write-skipped",
                json!({
                    "rule_id": rule_id,
                    "path": path,
                }),
            ),
            PlanProgressEvent::ScanCachePruned { report } => self.emit_data(
                "scan-cache-pruned",
                json!({
                    "inspected": report.inspected,
                    "pruned": report.pruned,
                    "retained": report.retained,
                }),
            ),
        }
    }

    pub(crate) fn emit_completed<T: Serialize + ?Sized>(
        &mut self,
        payload_kind: &str,
        data: &T,
    ) -> Result<()> {
        #[derive(Serialize)]
        struct CompletionEvent<'a, T: Serialize + ?Sized> {
            api_version: &'static str,
            kind: &'static str,
            command: &'a str,
            sequence: u64,
            event_kind: &'static str,
            generated_at_unix_seconds: u64,
            payload_kind: &'a str,
            data: &'a T,
        }

        let event = CompletionEvent {
            api_version: API_VERSION,
            kind: "event",
            command: self.command,
            sequence: self.take_sequence(),
            event_kind: "completed",
            generated_at_unix_seconds: unix_now(),
            payload_kind,
            data,
        };
        println!("{}", serde_json::to_string(&event)?);
        Ok(())
    }

    pub(crate) fn emit_cancelled(&mut self, detail: &str) -> Result<()> {
        self.emit_data("cancelled", json!({ "detail": detail }))
    }

    pub(crate) fn emit_error(&mut self, err: &anyhow::Error) -> Result<()> {
        let event = ErrorEventEnvelope {
            api_version: API_VERSION,
            kind: "event",
            command: self.command,
            sequence: self.take_sequence(),
            event_kind: "error",
            generated_at_unix_seconds: unix_now(),
            error: classify_error(err),
        };
        println!("{}", serde_json::to_string(&event)?);
        Ok(())
    }

    fn emit_data(&mut self, event_kind: &'static str, data: serde_json::Value) -> Result<()> {
        #[derive(Serialize)]
        struct DataEvent<'a> {
            api_version: &'static str,
            kind: &'static str,
            command: &'a str,
            sequence: u64,
            event_kind: &'static str,
            generated_at_unix_seconds: u64,
            data: serde_json::Value,
        }

        let event = DataEvent {
            api_version: API_VERSION,
            kind: "event",
            command: self.command,
            sequence: self.take_sequence(),
            event_kind,
            generated_at_unix_seconds: unix_now(),
            data,
        };
        println!("{}", serde_json::to_string(&event)?);
        Ok(())
    }

    fn take_sequence(&mut self) -> u64 {
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.saturating_add(1);
        sequence
    }
}

pub(crate) fn format_issue_matrix_entry(issue: &CleanupIssueSummary) -> String {
    format!(
        "{} {}: {}, {} ({})",
        issue.status.label(),
        issue.reason_code.label(),
        format_count(issue.targets as u64, "target", "targets"),
        issue.estimated_bytes,
        format_bytes(issue.estimated_bytes)
    )
}

pub(crate) fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];

    if bytes < 1024 {
        return format!("{bytes} B");
    }

    let mut value = bytes as f64;
    let mut unit_index = 0;
    while value >= 1024.0 && unit_index < UNITS.len() - 1 {
        value /= 1024.0;
        unit_index += 1;
    }

    format!("{value:.2} {}", UNITS[unit_index])
}

#[cfg(test)]
mod tests {
    use super::restore_hint_suffix;

    #[test]
    fn restore_hint_suffix_deduplicates_and_formats_hints() {
        assert_eq!(
            restore_hint_suffix([
                "Steam web caches will be rebuilt on launch.",
                "Steam web caches will be rebuilt on launch.",
                "Steam download staging data will be recreated if needed.",
            ]),
            " [restore: Steam web caches will be rebuilt on launch.; Steam download staging data will be recreated if needed.]"
        );
    }
}

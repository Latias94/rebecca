use anyhow::Result;
use rebecca::core::RuleDefinition;
use rebecca::core::plan::{CleanupIssueSummary, CleanupPlan};
use rebecca::core::planner::PlanProgressEvent;
use rebecca::core::progress::InspectProgressEvent;
use serde::Serialize;
use serde_json::{Value, json};
use std::fmt;
use std::io::{self, Write};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::clean_view::ScanCacheProgressSummary;
use crate::cli::OutputMode;
use crate::text::format_count;

const API_VERSION_V1: &str = "rebecca.cli.v1";

pub(crate) type HumanPlanRenderer =
    fn(&CleanupPlan, Option<ScanCacheProgressSummary>) -> Result<()>;
pub(crate) type WorkflowSuccessRenderer = fn(
    &CleanupPlan,
    CliApiContract,
    OutputMode,
    HumanPlanRenderer,
    Option<ScanCacheProgressSummary>,
    Option<NdjsonEventWriter>,
) -> Result<()>;

#[derive(Debug, Clone, Copy)]
pub(crate) struct CliApiContract {
    pub(crate) api_version: &'static str,
    pub(crate) command: &'static str,
    pub(crate) payload_kind: &'static str,
}

impl CliApiContract {
    pub(crate) const fn v1(command: &'static str, payload_kind: &'static str) -> Self {
        Self {
            api_version: API_VERSION_V1,
            command,
            payload_kind,
        }
    }
}

pub(crate) type WorkflowOutputContract = CliApiContract;

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

pub(crate) fn is_broken_pipe_error(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        cause
            .downcast_ref::<io::Error>()
            .is_some_and(|err| err.kind() == io::ErrorKind::BrokenPipe)
            || cause
                .downcast_ref::<rebecca::core::RebeccaError>()
                .is_some_and(|err| {
                    matches!(err, rebecca::core::RebeccaError::Io(io_err) if io_err.kind() == io::ErrorKind::BrokenPipe)
                })
    })
}

pub(crate) fn preserve_io_error_kind(err: anyhow::Error) -> io::Error {
    if let Some(io_err) = err.downcast_ref::<io::Error>() {
        return io::Error::new(io_err.kind(), io_err.to_string());
    }

    io::Error::other(err.to_string())
}

fn write_stdout_line(line: &str) -> io::Result<()> {
    let stdout = io::stdout();
    let mut writer = stdout.lock();
    writer.write_all(line.as_bytes())?;
    writer.write_all(b"\n")
}

pub(crate) fn print_success<T: Serialize + ?Sized>(
    command: &str,
    payload_kind: &str,
    data: &T,
) -> Result<()> {
    let envelope = SuccessEnvelope {
        api_version: API_VERSION_V1,
        kind: "success",
        command,
        payload_kind,
        generated_at_unix_seconds: unix_now(),
        data,
    };
    write_stdout_line(&to_machine_json_pretty(&envelope)?)?;
    Ok(())
}

pub(crate) fn print_success_with_contract<T: Serialize + ?Sized>(
    contract: CliApiContract,
    data: &T,
) -> Result<()> {
    let envelope = SuccessEnvelope {
        api_version: contract.api_version,
        kind: "success",
        command: contract.command,
        payload_kind: contract.payload_kind,
        generated_at_unix_seconds: unix_now(),
        data,
    };
    write_stdout_line(&to_machine_json_pretty(&envelope)?)?;
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

pub(crate) fn print_command_success_with_contract<T, P, H>(
    contract: CliApiContract,
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
            print_machine_success_payload_with_contract(contract, mode, &data)
        }
    }
}

pub(crate) fn render_error(contract: CliApiContract, mode: OutputMode, err: &anyhow::Error) {
    if mode.is_human() {
        eprintln!("{err:#}");
        return;
    }

    let error = classify_error(err);

    match mode {
        OutputMode::Human => unreachable!("human mode handled above"),
        OutputMode::Json => {
            let envelope = ErrorEnvelope {
                api_version: contract.api_version,
                kind: "error",
                command: contract.command,
                generated_at_unix_seconds: unix_now(),
                error,
            };
            match to_machine_json_pretty(&envelope) {
                Ok(rendered) => eprintln!("{rendered}"),
                Err(render_err) => eprintln!("{render_err}"),
            }
        }
        OutputMode::Ndjson => {
            let envelope = ErrorEventEnvelope {
                api_version: contract.api_version,
                kind: "event",
                command: contract.command,
                sequence: 0,
                event_kind: "error",
                generated_at_unix_seconds: unix_now(),
                error,
            };
            match to_machine_json(&envelope) {
                Ok(rendered) => {
                    if let Err(err) = write_stdout_line(&rendered)
                        && err.kind() != io::ErrorKind::BrokenPipe
                    {
                        eprintln!("{err}");
                    }
                }
                Err(render_err) => eprintln!("{render_err}"),
            }
        }
    }
}

fn classify_error(err: &anyhow::Error) -> ApiErrorBody<'static> {
    let detail = err.to_string();

    if err.downcast_ref::<clap::Error>().is_some() {
        return ApiErrorBody {
            code: "invalid-arguments",
            title: "Invalid command arguments",
            detail,
            exit_code: 1,
            source: Some("clap"),
        };
    }

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
            rebecca::core::RebeccaError::InvalidCatalogSelector(_) => {
                ("invalid-catalog-selector", "Invalid catalog selector")
            }
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
            rebecca::core::RebeccaError::SafetyCatalogInvalid(_) => {
                ("safety-catalog-invalid", "Safety catalog invalid")
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

    let (code, title) = if detail.contains("invalid protected path")
        || detail.contains("--dry-run cannot be combined with --yes")
    {
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

pub(crate) fn print_machine_success_payload_with_contract<T: Serialize + ?Sized>(
    contract: CliApiContract,
    mode: OutputMode,
    payload: &T,
) -> Result<()> {
    match mode {
        OutputMode::Human => unreachable!("human mode is rendered by the caller"),
        OutputMode::Json => print_success_with_contract(contract, payload),
        OutputMode::Ndjson => print_success_event_with_contract(contract, payload),
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
        OutputMode::Json => print_success_with_contract(contract, payload),
        OutputMode::Ndjson => {
            let mut writer =
                event_writer.unwrap_or_else(|| NdjsonEventWriter::with_contract(contract));
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

pub(crate) fn print_success_event_with_contract<T: Serialize + ?Sized>(
    contract: CliApiContract,
    data: &T,
) -> Result<()> {
    let mut writer = NdjsonEventWriter::with_contract(contract);
    writer.emit_completed(contract.payload_kind, data)
}

#[derive(Debug, Default)]
pub(crate) struct NdjsonEventWriter {
    command: &'static str,
    api_version: &'static str,
    next_sequence: u64,
}

impl NdjsonEventWriter {
    pub(crate) fn new(command: &'static str) -> Self {
        Self::with_contract(CliApiContract::v1(command, "event"))
    }

    pub(crate) fn with_contract(contract: CliApiContract) -> Self {
        Self {
            command: contract.command,
            api_version: contract.api_version,
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

    pub(crate) fn emit_inspect_progress(&mut self, event: InspectProgressEvent<'_>) -> Result<()> {
        let data = match event {
            InspectProgressEvent::RootStarted {
                root_index,
                root_count,
                root,
                backend,
            } => json!({
                "progress_kind": "root-started",
                "root_index": root_index,
                "root_count": root_count,
                "root": root,
                "backend": backend.label(),
            }),
            InspectProgressEvent::RootFinished {
                root_index,
                root_count,
                root,
                status,
                logical_bytes,
                files,
                directories,
            } => json!({
                "progress_kind": "root-finished",
                "root_index": root_index,
                "root_count": root_count,
                "root": root,
                "status": status.label(),
                "logical_bytes": logical_bytes,
                "files": files,
                "directories": directories,
            }),
            InspectProgressEvent::EntryStarted {
                root,
                path,
                entry_index,
                backend,
            } => json!({
                "progress_kind": "entry-started",
                "root": root,
                "path": path,
                "entry_index": entry_index,
                "backend": backend.label(),
            }),
            InspectProgressEvent::EntryMeasured {
                root,
                path,
                entry_index,
                logical_bytes,
                files,
                directories,
            } => json!({
                "progress_kind": "entry-measured",
                "root": root,
                "path": path,
                "entry_index": entry_index,
                "logical_bytes": logical_bytes,
                "files": files,
                "directories": directories,
            }),
            InspectProgressEvent::FileMeasured {
                root,
                target_path,
                path,
                file_size,
                files_scanned,
                bytes_scanned,
            } => json!({
                "progress_kind": "file-measured",
                "root": root,
                "target_path": target_path,
                "path": path,
                "file_size": file_size,
                "files_scanned": files_scanned,
                "bytes_scanned": bytes_scanned,
            }),
            InspectProgressEvent::TraversalProgress {
                root,
                counter,
                value,
                logical_bytes,
                files,
                directories,
            } => json!({
                "progress_kind": "traversal-progress",
                "root": root,
                "counter": counter.label(),
                "value": value,
                "logical_bytes": logical_bytes,
                "files": files,
                "directories": directories,
            }),
            InspectProgressEvent::BackendFallback {
                root,
                backend,
                reason,
            } => json!({
                "progress_kind": "backend-fallback",
                "root": root,
                "backend": backend.label(),
                "reason": reason,
            }),
            InspectProgressEvent::BackendStageStarted {
                root,
                backend,
                stage,
            } => json!({
                "progress_kind": "backend-stage-started",
                "root": root,
                "backend": backend.label(),
                "stage": stage,
            }),
            InspectProgressEvent::BackendStageFinished {
                root,
                backend,
                stage,
            } => json!({
                "progress_kind": "backend-stage-finished",
                "root": root,
                "backend": backend.label(),
                "stage": stage,
            }),
            InspectProgressEvent::BackendMetric {
                root,
                backend,
                metric,
                value,
            } => json!({
                "progress_kind": "backend-metric",
                "root": root,
                "backend": backend.label(),
                "metric": metric,
                "value": value,
            }),
            InspectProgressEvent::CacheEvent {
                path,
                event,
                reason,
                estimated_bytes,
            } => json!({
                "progress_kind": "cache-event",
                "path": path,
                "event": event.label(),
                "reason": reason,
                "estimated_bytes": estimated_bytes,
            }),
            InspectProgressEvent::Finalizing {
                roots,
                logical_bytes,
                files,
                directories,
            } => json!({
                "progress_kind": "finalizing",
                "roots": roots,
                "logical_bytes": logical_bytes,
                "files": files,
                "directories": directories,
            }),
        };
        self.emit_payload("inspect-progress", "inspect-progress", &data)
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
            api_version: self.api_version,
            kind: "event",
            command: self.command,
            sequence: self.take_sequence(),
            event_kind: "completed",
            generated_at_unix_seconds: unix_now(),
            payload_kind,
            data,
        };
        self.write_event(&event)?;
        Ok(())
    }

    pub(crate) fn emit_payload<T: Serialize + ?Sized>(
        &mut self,
        event_kind: &'static str,
        payload_kind: &str,
        data: &T,
    ) -> Result<()> {
        #[derive(Serialize)]
        struct PayloadEvent<'a, T: Serialize + ?Sized> {
            api_version: &'static str,
            kind: &'static str,
            command: &'a str,
            sequence: u64,
            event_kind: &'static str,
            generated_at_unix_seconds: u64,
            payload_kind: &'a str,
            data: &'a T,
        }

        let event = PayloadEvent {
            api_version: self.api_version,
            kind: "event",
            command: self.command,
            sequence: self.take_sequence(),
            event_kind,
            generated_at_unix_seconds: unix_now(),
            payload_kind,
            data,
        };
        self.write_event(&event)?;
        Ok(())
    }

    pub(crate) fn emit_cancelled(&mut self, detail: &str) -> Result<()> {
        self.emit_data("cancelled", json!({ "detail": detail }))
    }

    pub(crate) fn emit_error(&mut self, err: &anyhow::Error) -> Result<()> {
        let event = ErrorEventEnvelope {
            api_version: self.api_version,
            kind: "event",
            command: self.command,
            sequence: self.take_sequence(),
            event_kind: "error",
            generated_at_unix_seconds: unix_now(),
            error: classify_error(err),
        };
        self.write_event(&event)?;
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
            api_version: self.api_version,
            kind: "event",
            command: self.command,
            sequence: self.take_sequence(),
            event_kind,
            generated_at_unix_seconds: unix_now(),
            data,
        };
        self.write_event(&event)?;
        Ok(())
    }

    fn take_sequence(&mut self) -> u64 {
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.saturating_add(1);
        sequence
    }

    fn write_event<T: Serialize + ?Sized>(&self, event: &T) -> Result<()> {
        let stdout = io::stdout();
        let mut writer = stdout.lock();
        let event = machine_json_value(event)?;
        serde_json::to_writer(&mut writer, &event)?;
        writer.write_all(b"\n")?;
        Ok(())
    }
}

fn to_machine_json_pretty<T: Serialize + ?Sized>(value: &T) -> Result<String> {
    let value = machine_json_value(value)?;
    Ok(serde_json::to_string_pretty(&value)?)
}

fn to_machine_json<T: Serialize + ?Sized>(value: &T) -> Result<String> {
    let value = machine_json_value(value)?;
    Ok(serde_json::to_string(&value)?)
}

fn machine_json_value<T: Serialize + ?Sized>(value: &T) -> Result<Value> {
    let mut value = serde_json::to_value(value)?;
    normalize_machine_path_fields(&mut value);
    Ok(value)
}

fn normalize_machine_path_fields(value: &mut Value) {
    match value {
        Value::Object(fields) => {
            for (key, child) in fields {
                if is_machine_path_field(key) {
                    normalize_machine_path_value(child);
                } else {
                    normalize_machine_path_fields(child);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                normalize_machine_path_fields(item);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

fn normalize_machine_path_value(value: &mut Value) {
    match value {
        Value::String(path) => {
            if path.contains('\\') {
                *path = path.replace('\\', "/");
            }
        }
        Value::Array(items) => {
            for item in items {
                normalize_machine_path_value(item);
            }
        }
        Value::Object(fields) => {
            for child in fields.values_mut() {
                normalize_machine_path_value(child);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn is_machine_path_field(key: &str) -> bool {
    matches!(
        key,
        "path" | "paths" | "root" | "roots" | "install_locations"
    ) || key.ends_with("_path")
        || key.ends_with("_paths")
        || key.ends_with("_dir")
        || key.ends_with("_dirs")
        || key.ends_with("_file")
        || key.ends_with("_files")
        || key.ends_with("_root")
        || key.ends_with("_roots")
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

pub(crate) fn format_shell_command(command: &str, args: &[String]) -> String {
    std::iter::once(command)
        .chain(args.iter().map(String::as_str))
        .map(format_shell_argument)
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn format_shell_argument(value: &str) -> String {
    if !value.is_empty()
        && value.chars().all(|ch| {
            ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '/' | ':' | '\\')
        })
    {
        return value.to_string();
    }

    format!("'{}'", value.replace('\'', "''"))
}

#[cfg(test)]
mod tests {
    use super::{format_shell_command, normalize_machine_path_fields, restore_hint_suffix};
    use serde_json::json;

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

    #[test]
    fn format_shell_command_quotes_powershell_arguments_when_needed() {
        assert_eq!(
            format_shell_command(
                "rebecca",
                &[
                    "clean".to_string(),
                    "--root".to_string(),
                    "C:\\Users\\Ada Lovelace\\Temp".to_string(),
                ],
            ),
            "rebecca clean --root 'C:\\Users\\Ada Lovelace\\Temp'"
        );
    }

    #[test]
    fn machine_json_normalizes_path_fields_without_rewriting_plain_text() {
        let mut value = json!({
            "path": "C:\\Users\\Ada\\Temp",
            "cache_dir": "C:\\Users\\Ada\\AppData\\Local\\Rebecca\\cache",
            "attempted_paths": [
                "C:\\Users\\Ada\\Temp\\a.bin",
                "D:\\Cache\\b.bin"
            ],
            "install_locations": [
                "C:\\Program Files\\Example"
            ],
            "project_artifact": {
                "project_root": "C:\\Users\\Ada\\workspace\\app"
            },
            "detail": "PowerShell showed C:\\Users\\Ada\\Temp in stderr",
            "suggested_command": {
                "command": "rebecca",
                "args": ["clean", "--root", "C:\\Users\\Ada\\Temp"]
            }
        });

        normalize_machine_path_fields(&mut value);

        assert_eq!(value["path"], "C:/Users/Ada/Temp");
        assert_eq!(
            value["cache_dir"],
            "C:/Users/Ada/AppData/Local/Rebecca/cache"
        );
        assert_eq!(value["attempted_paths"][0], "C:/Users/Ada/Temp/a.bin");
        assert_eq!(value["attempted_paths"][1], "D:/Cache/b.bin");
        assert_eq!(value["install_locations"][0], "C:/Program Files/Example");
        assert_eq!(
            value["project_artifact"]["project_root"],
            "C:/Users/Ada/workspace/app"
        );
        assert_eq!(
            value["detail"],
            "PowerShell showed C:\\Users\\Ada\\Temp in stderr"
        );
        assert_eq!(
            value["suggested_command"]["args"][2],
            "C:\\Users\\Ada\\Temp"
        );
    }
}

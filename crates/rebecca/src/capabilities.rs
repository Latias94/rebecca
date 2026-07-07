use anyhow::Result;
use rebecca::core::Platform;
use serde::Serialize;

use crate::cli::OutputMode;
use crate::output::CliApiContract;

#[derive(Debug, Serialize)]
struct CapabilitiesReport {
    api_version: &'static str,
    cli_version: &'static str,
    platform: PlatformReport,
    features: FeatureReport,
    output_formats: &'static [&'static str],
    schema_documents: &'static [&'static str],
    commands: Vec<CommandCapability>,
    long_running_commands: &'static [&'static str],
    safety_model: SafetyModelReport,
}

#[derive(Debug, Serialize)]
struct PlatformReport {
    current: &'static str,
    cleanup_execution_supported: bool,
}

#[derive(Debug, Serialize)]
struct FeatureReport {
    rules: bool,
    windows: bool,
    ntfs: bool,
}

#[derive(Debug, Serialize)]
struct CommandCapability {
    name: &'static str,
    payload_kind: &'static str,
    machine_readable: bool,
    ndjson: bool,
    mutates_files: bool,
}

#[derive(Debug, Serialize)]
struct SafetyModelReport {
    preview_default: bool,
    explicit_execution_flag: &'static str,
    recoverable_delete_default: bool,
    persistent_risky_opt_in: bool,
    external_rules_enabled_by_validation: bool,
}

const OUTPUT_FORMATS: &[&str] = &["human", "json", "ndjson"];
const SCHEMA_DOCUMENTS: &[&str] = &["envelope", "event", "error", "payloads"];
const LONG_RUNNING_COMMANDS: &[&str] = &[
    "clean",
    "apps scan",
    "apps clean",
    "purge",
    "inspect artifacts",
    "inspect map",
    "inspect space",
];

pub fn run(output_mode: OutputMode) -> Result<()> {
    let report = capabilities_report();
    crate::output::print_command_success_with_contract(
        CliApiContract::v1("capabilities", "capabilities"),
        output_mode,
        || &report,
        || {
            print_human(&report);
            Ok(())
        },
    )
}

fn capabilities_report() -> CapabilitiesReport {
    CapabilitiesReport {
        api_version: "rebecca.cli.v1",
        cli_version: env!("CARGO_PKG_VERSION"),
        platform: PlatformReport {
            current: Platform::current().label(),
            cleanup_execution_supported: matches!(
                Platform::current(),
                Platform::Windows | Platform::Linux | Platform::Macos
            ),
        },
        features: FeatureReport {
            rules: cfg!(feature = "rules"),
            windows: cfg!(feature = "windows"),
            ntfs: cfg!(feature = "ntfs"),
        },
        output_formats: OUTPUT_FORMATS,
        schema_documents: SCHEMA_DOCUMENTS,
        commands: command_capabilities(),
        long_running_commands: LONG_RUNNING_COMMANDS,
        safety_model: SafetyModelReport {
            preview_default: true,
            explicit_execution_flag: "--yes",
            recoverable_delete_default: true,
            persistent_risky_opt_in: false,
            external_rules_enabled_by_validation: false,
        },
    }
}

fn command_capabilities() -> Vec<CommandCapability> {
    vec![
        command("capabilities", "capabilities", false, false),
        command("schema export", "cli-schema", false, false),
        command("catalog", "catalog", false, false),
        command("catalog validate", "catalog-validation", false, false),
        command("rules validate", "rule-validation", false, false),
        command("scan", "rule-catalog", false, false),
        command("clean", "cleanup-plan", true, true),
        command("apps scan", "app-leftovers-cleanup-plan", false, true),
        command("apps clean", "app-leftovers-cleanup-plan", true, true),
        command("purge", "project-artifact-cleanup-plan", true, true),
        command("inspect space", "inspect-space", false, true),
        command("inspect map", "inspect-map", false, true),
        command("inspect artifacts", "inspect-artifacts", false, true),
        command("inspect lint", "inspect-lint", false, false),
        command("history", "history-list", false, false),
        command("cache inspect", "cache-inventory", false, false),
        command("cache doctor", "cache-doctor", false, false),
        command("cache prune", "cache-prune-report", true, false),
        command("cache purge", "cache-purge-report", true, false),
        command("config paths", "config-paths", false, false),
        command("config show", "config-view", false, false),
        command("config validate", "config-validation", false, false),
        command("doctor permissions", "permissions-diagnostic", false, false),
        command(
            "doctor active-processes",
            "active-process-diagnostic",
            false,
            false,
        ),
    ]
}

fn command(
    name: &'static str,
    payload_kind: &'static str,
    mutates_files: bool,
    ndjson: bool,
) -> CommandCapability {
    CommandCapability {
        name,
        payload_kind,
        machine_readable: true,
        ndjson,
        mutates_files,
    }
}

fn print_human(report: &CapabilitiesReport) {
    println!("Rebecca CLI capabilities");
    println!("API version: {}", report.api_version);
    println!("CLI version: {}", report.cli_version);
    println!("Platform: {}", report.platform.current);
    println!(
        "Features: rules={}, windows={}, ntfs={}",
        report.features.rules, report.features.windows, report.features.ntfs
    );
    println!("Machine formats: {}", report.output_formats.join(", "));
    println!("Schema documents: {}", report.schema_documents.join(", "));
    println!("Commands:");
    for command in &report.commands {
        println!(
            "  - {} -> {}{}{}",
            command.name,
            command.payload_kind,
            if command.ndjson { " [ndjson]" } else { "" },
            if command.mutates_files {
                " [mutating]"
            } else {
                ""
            }
        );
    }
}

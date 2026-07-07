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
    recommended_startup_commands: &'static [&'static str],
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
    availability: CommandAvailability,
    platforms: &'static [&'static str],
    schema_documents: &'static [&'static str],
    preflight_commands: &'static [&'static str],
    required_execution_flag: Option<&'static str>,
    required_confirmation_flags: &'static [&'static str],
    macos_privacy_relevant: bool,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "kebab-case")]
enum CommandAvailability {
    Available,
    PreviewOnly,
    UnsupportedPlatform,
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
const SCHEMA_DOCUMENTS: &[&str] = &[
    "envelope",
    "event",
    "error",
    "payloads",
    "config",
    "cleaner-manifest-v1",
];
const ALL_PLATFORMS: &[&str] = &["windows", "linux", "macos"];
const CURRENT_PLATFORM_ONLY: &[&str] = &["current"];
const CLEANUP_SCHEMA_DOCS: &[&str] = &["payloads", "config", "cleaner-manifest-v1"];
const PAYLOAD_SCHEMA_DOCS: &[&str] = &["payloads"];
const CONFIG_SCHEMA_DOCS: &[&str] = &["payloads", "config"];
const RULE_SCHEMA_DOCS: &[&str] = &["payloads", "cleaner-manifest-v1"];
const CLEAN_PREFLIGHT: &[&str] = &[
    "doctor permissions",
    "doctor active-processes",
    "config show",
];
const MACOS_CLEAN_PREFLIGHT: &[&str] = &[
    "doctor permissions",
    "doctor active-processes",
    "catalog --kind cleanup-rule --platform macos",
    "config show",
];
const STARTUP_COMMANDS: &[&str] = &[
    "capabilities",
    "schema export --document payloads",
    "schema export --document config",
    "schema export --document cleaner-manifest-v1",
    "doctor permissions",
    "doctor active-processes",
    "catalog --kind cleanup-rule --platform current",
    "config show",
    "config paths",
];
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
        recommended_startup_commands: STARTUP_COMMANDS,
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
        command("schema export", "cli-schema", false, false)
            .with_schema_documents(SCHEMA_DOCUMENTS),
        command("catalog", "catalog", false, false),
        command("catalog validate", "catalog-validation", false, false),
        command("rules validate", "rule-validation", false, false)
            .with_schema_documents(RULE_SCHEMA_DOCS),
        command("scan", "rule-catalog", false, false).with_schema_documents(CLEANUP_SCHEMA_DOCS),
        command("clean", "cleanup-plan", true, true)
            .with_schema_documents(CLEANUP_SCHEMA_DOCS)
            .with_preflight(clean_preflight_commands())
            .with_required_execution_flag("--yes")
            .with_required_confirmation_flags(&[
                "--allow-moderate",
                "--allow-risky",
                "--allow-warning",
            ])
            .with_macos_privacy_relevant(),
        command("apps scan", "app-leftovers-cleanup-plan", false, true)
            .with_schema_documents(PAYLOAD_SCHEMA_DOCS)
            .with_preflight(&["doctor active-processes"]),
        command("apps clean", "app-leftovers-cleanup-plan", true, true)
            .with_schema_documents(PAYLOAD_SCHEMA_DOCS)
            .with_preflight(&["doctor permissions", "doctor active-processes"])
            .with_required_execution_flag("--yes")
            .with_required_confirmation_flags(&["--allow-warning"])
            .with_macos_privacy_relevant(),
        command("purge", "project-artifact-cleanup-plan", true, true)
            .with_schema_documents(PAYLOAD_SCHEMA_DOCS)
            .with_preflight(&["config show"])
            .with_required_execution_flag("--yes")
            .with_macos_privacy_relevant(),
        command("inspect space", "inspect-space", false, true),
        command("inspect map", "inspect-map", false, true),
        command("inspect artifacts", "inspect-artifacts", false, true)
            .with_preflight(&["config show"]),
        command("inspect lint", "inspect-lint", false, false),
        command("history", "history-list", false, false),
        command("cache inspect", "cache-inventory", false, false),
        command("cache doctor", "cache-doctor", false, false),
        command("cache prune", "cache-prune-report", true, false)
            .with_required_execution_flag("--yes"),
        command("cache purge", "cache-purge-report", true, false)
            .with_required_execution_flag("--yes")
            .with_required_confirmation_flags(&["--permanent"]),
        command("config paths", "config-paths", false, false)
            .with_schema_documents(CONFIG_SCHEMA_DOCS),
        command("config show", "config-view", false, false)
            .with_schema_documents(CONFIG_SCHEMA_DOCS),
        command("config validate", "config-validation", false, false)
            .with_schema_documents(CONFIG_SCHEMA_DOCS),
        command("doctor permissions", "permissions-diagnostic", false, false)
            .with_macos_privacy_relevant(),
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
        availability: command_availability(mutates_files),
        platforms: command_platforms(),
        schema_documents: PAYLOAD_SCHEMA_DOCS,
        preflight_commands: &[],
        required_execution_flag: None,
        required_confirmation_flags: &[],
        macos_privacy_relevant: false,
    }
}

impl CommandCapability {
    fn with_schema_documents(mut self, schema_documents: &'static [&'static str]) -> Self {
        self.schema_documents = schema_documents;
        self
    }

    fn with_preflight(mut self, preflight_commands: &'static [&'static str]) -> Self {
        self.preflight_commands = preflight_commands;
        self
    }

    fn with_required_execution_flag(mut self, flag: &'static str) -> Self {
        self.required_execution_flag = Some(flag);
        self
    }

    fn with_required_confirmation_flags(mut self, flags: &'static [&'static str]) -> Self {
        self.required_confirmation_flags = flags;
        self
    }

    fn with_macos_privacy_relevant(mut self) -> Self {
        self.macos_privacy_relevant = true;
        self
    }
}

fn command_availability(mutates_files: bool) -> CommandAvailability {
    if cleanup_execution_supported() {
        CommandAvailability::Available
    } else if mutates_files {
        CommandAvailability::PreviewOnly
    } else {
        CommandAvailability::UnsupportedPlatform
    }
}

fn cleanup_execution_supported() -> bool {
    matches!(
        Platform::current(),
        Platform::Windows | Platform::Linux | Platform::Macos
    )
}

fn command_platforms() -> &'static [&'static str] {
    if cleanup_execution_supported() {
        ALL_PLATFORMS
    } else {
        CURRENT_PLATFORM_ONLY
    }
}

fn clean_preflight_commands() -> &'static [&'static str] {
    if Platform::current() == Platform::Macos {
        MACOS_CLEAN_PREFLIGHT
    } else {
        CLEAN_PREFLIGHT
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

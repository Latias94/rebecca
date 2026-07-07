use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use rebecca::core::config::load_app_paths;
use rebecca::core::external_rules::ExternalRuleStore;
use rebecca::core::{RebeccaError, RuleDefinition};
use serde::Serialize;

use crate::cli::OutputMode;
use crate::output::CliApiContract;

#[derive(Debug, Serialize)]
struct RuleValidationReport {
    valid: bool,
    enabled: bool,
    files: Vec<PathBuf>,
    discovery: RuleDiscoveryReport,
    summary: RuleValidationSummary,
    diagnostics: Vec<RuleValidationDiagnostic>,
    rule_count: usize,
    target_count: usize,
    platforms: Vec<&'static str>,
    categories: Vec<String>,
    rules: Vec<String>,
    rule_previews: Vec<RuleValidationPreview>,
    checks: &'static [&'static str],
}

#[derive(Debug, Serialize)]
struct RuleDiscoveryReport {
    directory_max_depth: usize,
    file_limit: usize,
    symlink_traversal: bool,
}

#[derive(Debug, Default, Serialize)]
struct RuleValidationSummary {
    checks_passed: usize,
    diagnostics: usize,
}

#[derive(Debug, Serialize)]
struct RuleValidationDiagnostic {
    code: &'static str,
    severity: &'static str,
    path: Option<PathBuf>,
    message: String,
}

#[derive(Debug, Serialize)]
struct RuleValidationPreview {
    rule_id: String,
    platform: &'static str,
    category: String,
    safety_level: &'static str,
    target_count: usize,
    warning_gates: Vec<String>,
    source: &'static str,
    enabled_by_validation: bool,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct RuleDiscoveryOptions {
    pub(crate) max_depth: usize,
    pub(crate) max_files: usize,
}

const EXTERNAL_RULE_VALIDATION_CHECKS: &[&str] = &[
    "Cleaner Manifest v1 TOML parses",
    "rule ids and target specs are unique",
    "rule ids use platform-prefixed canonical syntax",
    "warning gates exist in Rebecca safety catalog",
    "restore hints are present",
    "dangerous safety level is rejected",
    "protected target shapes are blocked",
    "browser rules stay inside regenerable cache boundaries",
    "target shapes have positive cleanup basis",
    "glob discovery is bounded",
    "directory discovery is bounded",
    "symbolic links are not traversed",
    "shape-derived warning gates are declared",
];

pub fn validate(
    output_mode: OutputMode,
    files: Vec<PathBuf>,
    dirs: Vec<PathBuf>,
    discovery: RuleDiscoveryOptions,
) -> Result<()> {
    let report = match build_validation_report(files, dirs, discovery) {
        Ok(report) => report,
        Err(err) if !output_mode.is_human() => {
            let report = failed_validation_report(discovery, &err);
            crate::output::print_machine_success_payload_with_contract(
                CliApiContract::v1("rules validate", "rule-validation"),
                output_mode,
                &report,
            )?;
            return Err(crate::output::MachineErrorRendered.into());
        }
        Err(err) => return Err(err),
    };

    crate::output::print_command_success_with_contract(
        CliApiContract::v1("rules validate", "rule-validation"),
        output_mode,
        || &report,
        || {
            println!("External rule validation: ok");
            println!("Files: {}", report.files.len());
            println!("Rules: {}", report.rule_count);
            println!("Targets: {}", report.target_count);
            println!("Enabled: {}", report.enabled);
            println!(
                "Discovery: max-depth={}, max-files={}, symlink-traversal={}",
                report.discovery.directory_max_depth,
                report.discovery.file_limit,
                report.discovery.symlink_traversal
            );
            println!("Checks:");
            for check in report.checks {
                println!("  - {check}");
            }
            Ok(())
        },
    )
}

pub fn import(output_mode: OutputMode, file: PathBuf) -> Result<()> {
    let store = external_rule_store()?;
    let report = store.import_manifest(&file)?;
    crate::output::print_command_success_with_contract(
        CliApiContract::v1("rules import", "rule-import"),
        output_mode,
        || &report,
        || {
            println!("External rule imported: {}", report.imported.import_id);
            println!("Enabled: {}", report.imported.enabled);
            println!("Rules: {}", report.imported.rule_ids.join(", "));
            Ok(())
        },
    )
}

pub fn list(output_mode: OutputMode) -> Result<()> {
    let store = external_rule_store()?;
    let report = store.list()?;
    crate::output::print_command_success_with_contract(
        CliApiContract::v1("rules list", "rule-import-list"),
        output_mode,
        || &report,
        || {
            println!("Imported external rules: {}", report.entries.len());
            for entry in &report.entries {
                println!(
                    "  - {} enabled={} rules={}",
                    entry.import_id,
                    entry.enabled,
                    entry.rule_ids.join(", ")
                );
            }
            Ok(())
        },
    )
}

pub fn enable(output_mode: OutputMode, import_id: String) -> Result<()> {
    mutate_enabled(output_mode, "rules enable", import_id, true)
}

pub fn disable(output_mode: OutputMode, import_id: String) -> Result<()> {
    mutate_enabled(output_mode, "rules disable", import_id, false)
}

pub fn remove(output_mode: OutputMode, import_id: String) -> Result<()> {
    let store = external_rule_store()?;
    let report = store.remove(&import_id)?;
    crate::output::print_command_success_with_contract(
        CliApiContract::v1("rules remove", "rule-import-mutation"),
        output_mode,
        || &report,
        || {
            println!("External rule removed: {}", report.import_id);
            Ok(())
        },
    )
}

fn mutate_enabled(
    output_mode: OutputMode,
    command: &'static str,
    import_id: String,
    enabled: bool,
) -> Result<()> {
    let store = external_rule_store()?;
    let report = store.set_enabled(&import_id, enabled)?;
    crate::output::print_command_success_with_contract(
        CliApiContract::v1(command, "rule-import-mutation"),
        output_mode,
        || &report,
        || {
            println!(
                "External rule {}: enabled={}",
                report.import_id, report.enabled
            );
            Ok(())
        },
    )
}

fn external_rule_store() -> Result<ExternalRuleStore> {
    Ok(ExternalRuleStore::default_for_state_dir(
        &load_app_paths()?.state_dir,
    ))
}

fn build_validation_report(
    files: Vec<PathBuf>,
    dirs: Vec<PathBuf>,
    discovery: RuleDiscoveryOptions,
) -> Result<RuleValidationReport> {
    validate_discovery_options(discovery)?;
    let files = collect_rule_files(files, dirs, discovery)?;
    let rules = rebecca::rules::validate_external_rule_files(&files)?;
    Ok(validation_report(files, discovery, &rules))
}

fn validation_report(
    files: Vec<PathBuf>,
    discovery: RuleDiscoveryOptions,
    rules: &[RuleDefinition],
) -> RuleValidationReport {
    let target_count = rules
        .iter()
        .map(|rule| rule.path_templates.len())
        .sum::<usize>();
    let platforms = rules
        .iter()
        .map(|rule| rule.platform.label())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let categories = rules
        .iter()
        .map(|rule| rule.category.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let rule_ids = rules
        .iter()
        .map(|rule| rule.id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let rule_previews = rules
        .iter()
        .map(rule_validation_preview)
        .collect::<Vec<_>>();

    RuleValidationReport {
        valid: true,
        enabled: false,
        discovery: RuleDiscoveryReport {
            directory_max_depth: discovery.max_depth,
            file_limit: discovery.max_files,
            symlink_traversal: false,
        },
        summary: RuleValidationSummary {
            checks_passed: EXTERNAL_RULE_VALIDATION_CHECKS.len(),
            diagnostics: 0,
        },
        diagnostics: Vec::new(),
        files,
        rule_count: rule_ids.len(),
        target_count,
        platforms,
        categories,
        rules: rule_ids,
        rule_previews,
        checks: EXTERNAL_RULE_VALIDATION_CHECKS,
    }
}

fn failed_validation_report(
    discovery: RuleDiscoveryOptions,
    err: &anyhow::Error,
) -> RuleValidationReport {
    RuleValidationReport {
        valid: false,
        enabled: false,
        files: Vec::new(),
        discovery: RuleDiscoveryReport {
            directory_max_depth: discovery.max_depth,
            file_limit: discovery.max_files,
            symlink_traversal: false,
        },
        summary: RuleValidationSummary {
            checks_passed: 0,
            diagnostics: 1,
        },
        diagnostics: vec![rule_validation_diagnostic(err)],
        rule_count: 0,
        target_count: 0,
        platforms: Vec::new(),
        categories: Vec::new(),
        rules: Vec::new(),
        rule_previews: Vec::new(),
        checks: EXTERNAL_RULE_VALIDATION_CHECKS,
    }
}

fn rule_validation_preview(rule: &RuleDefinition) -> RuleValidationPreview {
    RuleValidationPreview {
        rule_id: rule.id.clone(),
        platform: rule.platform.label(),
        category: rule.category.clone(),
        safety_level: rule.safety_level.label(),
        target_count: rule.path_templates.len(),
        warning_gates: rule.warnings.clone(),
        source: rule.provenance.source.label(),
        enabled_by_validation: false,
    }
}

fn rule_validation_diagnostic(err: &anyhow::Error) -> RuleValidationDiagnostic {
    if let Some(core_error) = err.downcast_ref::<RebeccaError>() {
        return match core_error {
            RebeccaError::RuleCatalogInvalid(message) => RuleValidationDiagnostic {
                code: "rule-catalog-invalid",
                severity: "error",
                path: None,
                message: message.clone(),
            },
            _ => RuleValidationDiagnostic {
                code: "rule-validation-failed",
                severity: "error",
                path: None,
                message: core_error.to_string(),
            },
        };
    }

    RuleValidationDiagnostic {
        code: "rule-validation-failed",
        severity: "error",
        path: None,
        message: err.to_string(),
    }
}

fn validate_discovery_options(discovery: RuleDiscoveryOptions) -> Result<()> {
    if discovery.max_files == 0 {
        return Err(rule_catalog_invalid(
            "rules validate --max-files must be at least 1",
        ));
    }

    Ok(())
}

fn collect_rule_files(
    files: Vec<PathBuf>,
    dirs: Vec<PathBuf>,
    discovery: RuleDiscoveryOptions,
) -> Result<Vec<PathBuf>> {
    if files.is_empty() && dirs.is_empty() {
        return Err(rule_catalog_invalid(
            "rules validate requires at least one --file or --dir input",
        ));
    }

    let mut collected = BTreeSet::new();
    for file in files {
        validate_file_input(&file)?;
        insert_rule_file(file, &mut collected, discovery.max_files)?;
    }
    for dir in dirs {
        collect_rule_files_from_dir(&dir, &mut collected, discovery, 0)?;
    }

    if collected.is_empty() {
        return Err(rule_catalog_invalid(
            "rules validate did not find any .toml rule manifests",
        ));
    }

    Ok(collected.into_iter().collect())
}

fn validate_file_input(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path).map_err(|err| {
        rule_catalog_invalid(format!(
            "rule file is not readable: {}: {err}",
            path.display()
        ))
    })?;
    if metadata.file_type().is_symlink() {
        return Err(rule_catalog_invalid(format!(
            "rule file must not be a symlink: {}",
            path.display()
        )));
    }
    if !metadata.is_file() {
        return Err(rule_catalog_invalid(format!(
            "rule file is not readable: {}",
            path.display()
        )));
    }
    if !is_toml_manifest_path(path) {
        return Err(rule_catalog_invalid(format!(
            "rule file must use .toml extension: {}",
            path.display()
        )));
    }
    Ok(())
}

fn collect_rule_files_from_dir(
    dir: &Path,
    collected: &mut BTreeSet<PathBuf>,
    discovery: RuleDiscoveryOptions,
    depth: usize,
) -> Result<()> {
    let metadata = fs::symlink_metadata(dir).map_err(|err| {
        rule_catalog_invalid(format!(
            "rule directory is not readable: {}: {err}",
            dir.display()
        ))
    })?;
    if metadata.file_type().is_symlink() {
        return Err(rule_catalog_invalid(format!(
            "rule directory must not be a symlink: {}",
            dir.display()
        )));
    }
    if !metadata.is_dir() {
        return Err(rule_catalog_invalid(format!(
            "rule directory is not readable: {}",
            dir.display()
        )));
    }

    for entry in fs::read_dir(dir).map_err(|err| {
        rule_catalog_invalid(format!(
            "rule directory is not readable: {}: {err}",
            dir.display()
        ))
    })? {
        let entry = entry.map_err(|err| {
            rule_catalog_invalid(format!(
                "rule directory entry is not readable under {}: {err}",
                dir.display()
            ))
        })?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|err| {
            rule_catalog_invalid(format!(
                "rule directory entry type is not readable: {}: {err}",
                path.display()
            ))
        })?;
        if file_type.is_symlink() {
            continue;
        }

        if file_type.is_dir() {
            if depth >= discovery.max_depth {
                return Err(rule_catalog_invalid(format!(
                    "rule directory exceeds --max-depth {} at {}",
                    discovery.max_depth,
                    path.display()
                )));
            }
            collect_rule_files_from_dir(&path, collected, discovery, depth + 1)?;
        } else if file_type.is_file() && is_toml_manifest_path(&path) {
            insert_rule_file(path, collected, discovery.max_files)?;
        }
    }

    Ok(())
}

fn insert_rule_file(
    path: PathBuf,
    collected: &mut BTreeSet<PathBuf>,
    max_files: usize,
) -> Result<()> {
    if collected.insert(path.clone()) && collected.len() > max_files {
        return Err(rule_catalog_invalid(format!(
            "rules validate discovered more than --max-files {} manifests; latest path was {}",
            max_files,
            path.display()
        )));
    }

    Ok(())
}

fn is_toml_manifest_path(path: &Path) -> bool {
    path.extension().and_then(|extension| extension.to_str()) == Some("toml")
}

fn rule_catalog_invalid(message: impl Into<String>) -> anyhow::Error {
    RebeccaError::RuleCatalogInvalid(message.into()).into()
}

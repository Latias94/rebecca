use std::path::{Path, PathBuf};

use anyhow::Result;
use rebecca::core::RebeccaError;
use rebecca::core::config::{
    AppPaths, CONFIG_SCHEMA_VERSION, LoadedRebeccaConfig, RebeccaConfig, load_app_paths,
    load_config_with_source, resolve_runtime_config_from_loaded,
};
use serde::Serialize;

use crate::cli::OutputMode;
use crate::output::CliApiContract;

#[derive(Debug, Serialize)]
struct ConfigValidationReport {
    valid: bool,
    schema_version: u32,
    config_file: PathBuf,
    loaded_from_file: bool,
    summary: ConfigValidationSummary,
    diagnostics: Vec<ConfigValidationDiagnostic>,
    checks: &'static [&'static str],
}

#[derive(Debug, Default, Serialize)]
struct ConfigValidationSummary {
    checks_passed: usize,
    diagnostics: usize,
}

#[derive(Debug, Serialize)]
struct ConfigValidationDiagnostic {
    code: &'static str,
    severity: &'static str,
    path: PathBuf,
    message: String,
}

#[derive(Debug, Serialize)]
struct ConfigView {
    schema_version: u32,
    config_file: PathBuf,
    loaded_from_file: bool,
    config: RebeccaConfig,
    runtime: RuntimeConfigView,
}

#[derive(Debug, Serialize)]
struct RuntimeConfigView {
    app_paths: AppPaths,
    scan_cache: RuntimeScanCacheView,
    protected_paths: Vec<PathBuf>,
    purge: RuntimePurgeView,
}

#[derive(Debug, Serialize)]
struct RuntimeScanCacheView {
    directory_record_max_age_seconds: u64,
}

#[derive(Debug, Serialize)]
struct RuntimePurgeView {
    roots: Vec<PathBuf>,
    max_depth: usize,
    min_age_days: u64,
}

const CONFIG_VALIDATION_CHECKS: &[&str] = &[
    "config file is readable when present",
    "TOML shape matches Rebecca config schema",
    "config schema version is supported",
    "scan cache policy is positive",
    "protected paths are absolute user paths",
    "purge roots are absolute user paths",
    "runtime app paths resolve",
];

pub fn show(output_mode: OutputMode, file: Option<PathBuf>) -> Result<()> {
    let (config_file, loaded) = load_selected_config(file.as_deref())?;
    let runtime = resolve_runtime_config_from_loaded(&config_file, &loaded.config)?;
    let view = ConfigView {
        schema_version: CONFIG_SCHEMA_VERSION,
        loaded_from_file: loaded.loaded_from_file,
        config_file,
        config: loaded.config,
        runtime: RuntimeConfigView {
            app_paths: runtime.app_paths,
            scan_cache: RuntimeScanCacheView {
                directory_record_max_age_seconds: runtime
                    .scan_cache_policy
                    .directory_record_max_age_seconds(),
            },
            protected_paths: runtime.protected_paths,
            purge: RuntimePurgeView {
                roots: runtime.purge.roots,
                max_depth: runtime.purge.max_depth,
                min_age_days: runtime.purge.min_age_days,
            },
        },
    };

    crate::output::print_command_success_with_contract(
        CliApiContract::v1("config show", "config-view"),
        output_mode,
        || &view,
        || {
            println!("Config file: {}", view.config_file.display());
            println!("Loaded from file: {}", view.loaded_from_file);
            println!("Schema version: {}", view.schema_version);
            println!(
                "Scan cache max age: {}s",
                view.runtime.scan_cache.directory_record_max_age_seconds
            );
            println!("Protected paths: {}", view.runtime.protected_paths.len());
            println!("Purge roots: {}", view.runtime.purge.roots.len());
            Ok(())
        },
    )
}

pub fn validate(output_mode: OutputMode, file: Option<PathBuf>) -> Result<()> {
    let config_file = selected_config_file(file.as_deref())?;
    let report = match validate_selected_config(&config_file, file.is_some()) {
        Ok(report) => report,
        Err(err) if !output_mode.is_human() => {
            let report = failed_validation_report(&config_file, &err);
            crate::output::print_machine_success_payload_with_contract(
                CliApiContract::v1("config validate", "config-validation"),
                output_mode,
                &report,
            )?;
            return Err(crate::output::MachineErrorRendered.into());
        }
        Err(err) => return Err(err),
    };

    crate::output::print_command_success_with_contract(
        CliApiContract::v1("config validate", "config-validation"),
        output_mode,
        || &report,
        || {
            println!("Rebecca config validation: ok");
            println!("Config file: {}", report.config_file.display());
            println!("Loaded from file: {}", report.loaded_from_file);
            println!("Checks:");
            for check in report.checks {
                println!("  - {check}");
            }
            Ok(())
        },
    )
}

fn validate_selected_config(
    config_file: &Path,
    explicit_file: bool,
) -> Result<ConfigValidationReport> {
    let loaded = load_selected_config_from_path(config_file, explicit_file)?;
    let _ = resolve_runtime_config_from_loaded(config_file, &loaded.config)?;
    let report = ConfigValidationReport {
        valid: true,
        schema_version: CONFIG_SCHEMA_VERSION,
        loaded_from_file: loaded.loaded_from_file,
        config_file: config_file.to_path_buf(),
        summary: ConfigValidationSummary {
            checks_passed: CONFIG_VALIDATION_CHECKS.len(),
            diagnostics: 0,
        },
        diagnostics: Vec::new(),
        checks: CONFIG_VALIDATION_CHECKS,
    };

    Ok(report)
}

fn load_selected_config(file: Option<&Path>) -> Result<(PathBuf, LoadedRebeccaConfig)> {
    let config_file = selected_config_file(file)?;
    let config = load_selected_config_from_path(&config_file, file.is_some())?;
    Ok((config_file, config))
}

fn selected_config_file(file: Option<&Path>) -> Result<PathBuf> {
    Ok(match file {
        Some(path) => path.to_path_buf(),
        None => load_app_paths()?.config_file,
    })
}

fn load_selected_config_from_path(
    config_file: &Path,
    explicit_file: bool,
) -> Result<LoadedRebeccaConfig> {
    if explicit_file && !config_file.exists() {
        return Err(RebeccaError::ConfigRead {
            path: config_file.to_path_buf(),
            message: "file does not exist".to_string(),
        }
        .into());
    }
    load_config_with_source(config_file).map_err(Into::into)
}

fn failed_validation_report(config_file: &Path, err: &anyhow::Error) -> ConfigValidationReport {
    let diagnostic = config_validation_diagnostic(config_file, err);
    ConfigValidationReport {
        valid: false,
        schema_version: CONFIG_SCHEMA_VERSION,
        config_file: config_file.to_path_buf(),
        loaded_from_file: false,
        summary: ConfigValidationSummary {
            checks_passed: 0,
            diagnostics: 1,
        },
        diagnostics: vec![diagnostic],
        checks: CONFIG_VALIDATION_CHECKS,
    }
}

fn config_validation_diagnostic(
    config_file: &Path,
    err: &anyhow::Error,
) -> ConfigValidationDiagnostic {
    if let Some(core_error) = err.downcast_ref::<RebeccaError>() {
        return match core_error {
            RebeccaError::ConfigRead { path, message } => ConfigValidationDiagnostic {
                code: "config-read-failed",
                severity: "error",
                path: path.clone(),
                message: message.clone(),
            },
            RebeccaError::ConfigParse { path, message } => ConfigValidationDiagnostic {
                code: "config-parse-failed",
                severity: "error",
                path: path.clone(),
                message: message.clone(),
            },
            _ => ConfigValidationDiagnostic {
                code: "config-validation-failed",
                severity: "error",
                path: config_file.to_path_buf(),
                message: core_error.to_string(),
            },
        };
    }

    ConfigValidationDiagnostic {
        code: "config-validation-failed",
        severity: "error",
        path: config_file.to_path_buf(),
        message: err.to_string(),
    }
}

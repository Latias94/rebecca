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
    checks: &'static [&'static str],
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
    let (config_file, loaded) = load_selected_config(file.as_deref())?;
    let _ = resolve_runtime_config_from_loaded(&config_file, &loaded.config)?;
    let report = ConfigValidationReport {
        valid: true,
        schema_version: CONFIG_SCHEMA_VERSION,
        loaded_from_file: loaded.loaded_from_file,
        config_file,
        checks: CONFIG_VALIDATION_CHECKS,
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

fn load_selected_config(file: Option<&Path>) -> Result<(PathBuf, LoadedRebeccaConfig)> {
    let config_file = match file {
        Some(path) => path.to_path_buf(),
        None => load_app_paths()?.config_file,
    };
    if file.is_some() && !config_file.exists() {
        return Err(RebeccaError::ConfigRead {
            path: config_file,
            message: "file does not exist".to_string(),
        }
        .into());
    }
    let config = load_config_with_source(&config_file)?;
    Ok((config_file, config))
}

use std::path::{Path, PathBuf};

use directories::BaseDirs;
use serde::{Deserialize, Serialize};

use crate::{RebeccaError, Result};

pub const CONFIG_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppPaths {
    pub config_dir: PathBuf,
    pub config_file: PathBuf,
    pub state_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub history_file: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RebeccaConfig {
    pub version: u32,
    pub app_paths: RebeccaAppPathsConfig,
}

impl Default for RebeccaConfig {
    fn default() -> Self {
        Self {
            version: CONFIG_SCHEMA_VERSION,
            app_paths: RebeccaAppPathsConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RebeccaAppPathsConfig {
    pub state_dir: Option<PathBuf>,
    pub cache_dir: Option<PathBuf>,
    pub history_file: Option<PathBuf>,
}

pub fn default_app_paths() -> Result<AppPaths> {
    let config_dir = default_config_dir()?;
    let config_file = config_dir.join("config.toml");
    let config = load_config(&config_file)?;
    resolve_app_paths_with_config_dir(config_dir, config_file, &config)
}

pub fn load_app_paths() -> Result<AppPaths> {
    default_app_paths()
}

pub fn load_app_paths_from(config_file: &Path) -> Result<AppPaths> {
    let config_dir = config_file
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let config = load_config(config_file)?;
    resolve_app_paths_with_config_dir(config_dir, config_file.to_path_buf(), &config)
}

pub fn load_config(config_file: &Path) -> Result<RebeccaConfig> {
    if !config_file.exists() {
        return Ok(RebeccaConfig::default());
    }

    let raw = std::fs::read_to_string(config_file).map_err(|err| RebeccaError::ConfigRead {
        path: config_file.to_path_buf(),
        message: err.to_string(),
    })?;
    if raw.trim().is_empty() {
        return Ok(RebeccaConfig::default());
    }

    let config = toml::from_str(&raw).map_err(|err| RebeccaError::ConfigParse {
        path: config_file.to_path_buf(),
        message: err.to_string(),
    })?;
    validate_config(config_file, &config)?;

    Ok(config)
}

pub fn resolve_app_paths(config: &RebeccaConfig) -> Result<AppPaths> {
    let config_dir = default_config_dir()?;
    let config_file = config_dir.join("config.toml");
    resolve_app_paths_with_config_dir(config_dir, config_file, config)
}

fn resolve_app_paths_with_config_dir(
    config_dir: PathBuf,
    config_file: PathBuf,
    config: &RebeccaConfig,
) -> Result<AppPaths> {
    let state_dir = resolve_or_default(
        env_path("REBECCA_STATE_DIR").or_else(|| config.app_paths.state_dir.clone()),
        default_state_dir,
    )?;
    let cache_dir = resolve_or_default(
        env_path("REBECCA_CACHE_DIR").or_else(|| config.app_paths.cache_dir.clone()),
        default_cache_dir,
    )?;
    let history_file = env_path("REBECCA_HISTORY_FILE")
        .or_else(|| config.app_paths.history_file.clone())
        .unwrap_or_else(|| state_dir.join("history.jsonl"));

    Ok(AppPaths {
        config_file,
        history_file,
        config_dir,
        state_dir,
        cache_dir,
    })
}

fn resolve_or_default(value: Option<PathBuf>, default: fn() -> Result<PathBuf>) -> Result<PathBuf> {
    match value {
        Some(path) => Ok(path),
        None => default(),
    }
}

fn validate_config(config_file: &Path, config: &RebeccaConfig) -> Result<()> {
    if config.version != CONFIG_SCHEMA_VERSION {
        return Err(RebeccaError::ConfigParse {
            path: config_file.to_path_buf(),
            message: format!(
                "unsupported config version {}; supported version is {}",
                config.version, CONFIG_SCHEMA_VERSION
            ),
        });
    }

    Ok(())
}

fn default_config_dir() -> Result<PathBuf> {
    if let Some(path) = env_path("REBECCA_CONFIG_DIR") {
        return Ok(path);
    }

    let base_dirs = BaseDirs::new().ok_or(RebeccaError::UserDirsUnavailable)?;
    Ok(base_dirs.config_dir().join("Rebecca"))
}

fn default_state_dir() -> Result<PathBuf> {
    if let Some(path) = env_path("REBECCA_STATE_DIR") {
        return Ok(path);
    }

    let base_dirs = BaseDirs::new().ok_or(RebeccaError::UserDirsUnavailable)?;
    Ok(base_dirs.data_local_dir().join("Rebecca").join("state"))
}

fn default_cache_dir() -> Result<PathBuf> {
    if let Some(path) = env_path("REBECCA_CACHE_DIR") {
        return Ok(path);
    }

    let base_dirs = BaseDirs::new().ok_or(RebeccaError::UserDirsUnavailable)?;
    Ok(base_dirs.cache_dir().join("Rebecca").join("cache"))
}

fn env_path(key: &str) -> Option<PathBuf> {
    let value = std::env::var_os(key)?;
    if value.is_empty() {
        return None;
    }

    Some(PathBuf::from(value))
}

#[cfg(test)]
mod tests {
    use super::{
        CONFIG_SCHEMA_VERSION, RebeccaAppPathsConfig, RebeccaConfig, default_app_paths,
        load_app_paths_from, load_config, resolve_app_paths,
    };

    #[test]
    fn load_config_missing_file_returns_default() {
        let temp = tempfile::tempdir().unwrap();
        let config_file = temp.path().join("config.toml");

        let config = load_config(&config_file).unwrap();

        assert_eq!(config, RebeccaConfig::default());
    }

    #[test]
    fn load_config_empty_file_returns_default() {
        let temp = tempfile::tempdir().unwrap();
        let config_file = temp.path().join("config.toml");
        std::fs::write(&config_file, "   \n").unwrap();

        let config = load_config(&config_file).unwrap();

        assert_eq!(config, RebeccaConfig::default());
    }

    #[test]
    fn load_config_comment_only_file_returns_default() {
        let temp = tempfile::tempdir().unwrap();
        let config_file = temp.path().join("config.toml");
        std::fs::write(&config_file, "# local Rebecca config\n").unwrap();

        let config = load_config(&config_file).unwrap();

        assert_eq!(config, RebeccaConfig::default());
    }

    #[test]
    fn load_config_defaults_to_current_schema_version() {
        let temp = tempfile::tempdir().unwrap();
        let config_file = temp.path().join("config.toml");
        std::fs::write(
            &config_file,
            r#"
[app_paths]
state_dir = "C:\\Rebecca\\State"
"#,
        )
        .unwrap();

        let config = load_config(&config_file).unwrap();

        assert_eq!(config.version, CONFIG_SCHEMA_VERSION);
    }

    #[test]
    fn load_config_accepts_current_schema_version() {
        let temp = tempfile::tempdir().unwrap();
        let config_file = temp.path().join("config.toml");
        std::fs::write(
            &config_file,
            r#"
version = 1

[app_paths]
state_dir = "C:\\Rebecca\\State"
"#,
        )
        .unwrap();

        let config = load_config(&config_file).unwrap();

        assert_eq!(config.version, CONFIG_SCHEMA_VERSION);
        assert_eq!(
            config.app_paths.state_dir,
            Some(std::path::PathBuf::from(r"C:\Rebecca\State"))
        );
    }

    #[test]
    fn load_config_rejects_unsupported_schema_version() {
        let temp = tempfile::tempdir().unwrap();
        let config_file = temp.path().join("config.toml");
        std::fs::write(&config_file, "version = 2\n").unwrap();

        let err = load_config(&config_file).unwrap_err();

        let message = err.to_string();
        assert!(message.contains("unsupported config version 2"));
        assert!(message.contains("supported version is 1"));
        assert!(message.contains("config.toml"));
    }

    #[test]
    fn load_config_rejects_unknown_fields() {
        let temp = tempfile::tempdir().unwrap();
        let config_file = temp.path().join("config.toml");
        std::fs::write(
            &config_file,
            r#"
[app_paths]
unknown = "value"
"#,
        )
        .unwrap();

        let err = load_config(&config_file).unwrap_err();

        assert!(err.to_string().contains("unknown field"));
        assert!(err.to_string().contains("config.toml"));
    }

    #[test]
    fn load_config_reports_read_errors() {
        let temp = tempfile::tempdir().unwrap();
        let config_file = temp.path().join("config.toml");
        std::fs::create_dir_all(&config_file).unwrap();

        let err = load_config(&config_file).unwrap_err();

        let message = err.to_string();
        assert!(message.contains("config read failed"));
        assert!(message.contains("config.toml"));
    }

    #[test]
    fn resolve_app_paths_honors_config_overrides() {
        let config = RebeccaConfig {
            version: CONFIG_SCHEMA_VERSION,
            app_paths: RebeccaAppPathsConfig {
                state_dir: Some(std::path::PathBuf::from(r"C:\Rebecca\State")),
                cache_dir: Some(std::path::PathBuf::from(r"C:\Rebecca\Cache")),
                history_file: Some(std::path::PathBuf::from(r"C:\Rebecca\State\audit.jsonl")),
            },
        };

        let paths = resolve_app_paths(&config).unwrap();

        assert!(paths.config_dir.ends_with("Rebecca"));
        assert_eq!(
            paths.state_dir,
            std::path::PathBuf::from(r"C:\Rebecca\State")
        );
        assert_eq!(
            paths.cache_dir,
            std::path::PathBuf::from(r"C:\Rebecca\Cache")
        );
        assert_eq!(
            paths.history_file,
            std::path::PathBuf::from(r"C:\Rebecca\State\audit.jsonl")
        );
    }

    #[test]
    fn load_app_paths_from_uses_config_file_when_present() {
        let temp = tempfile::tempdir().unwrap();
        let config_dir = temp.path().join("Rebecca");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::write(
            config_dir.join("config.toml"),
            r#"
[app_paths]
state_dir = "C:\\Rebecca\\State"
cache_dir = "C:\\Rebecca\\Cache"
history_file = "C:\\Rebecca\\State\\audit.jsonl"
"#,
        )
        .unwrap();

        let paths = load_app_paths_from(&config_dir.join("config.toml")).unwrap();

        assert_eq!(paths.config_file, config_dir.join("config.toml"));
        assert_eq!(paths.config_dir, config_dir);
        assert_eq!(
            paths.state_dir,
            std::path::PathBuf::from(r"C:\Rebecca\State")
        );
        assert_eq!(
            paths.cache_dir,
            std::path::PathBuf::from(r"C:\Rebecca\Cache")
        );
        assert_eq!(
            paths.history_file,
            std::path::PathBuf::from(r"C:\Rebecca\State\audit.jsonl")
        );
    }

    #[test]
    fn default_app_paths_remain_available() {
        assert!(default_app_paths().is_ok());
    }
}

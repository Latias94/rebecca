use std::path::PathBuf;

use directories::BaseDirs;
use serde::{Deserialize, Serialize};

use crate::{RebeccaError, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppPaths {
    pub config_dir: PathBuf,
    pub config_file: PathBuf,
    pub state_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub history_file: PathBuf,
}

pub fn default_app_paths() -> Result<AppPaths> {
    let base_dirs = BaseDirs::new();
    let config_dir = env_path("REBECCA_CONFIG_DIR")
        .or_else(|| {
            base_dirs
                .as_ref()
                .map(|dirs| dirs.config_dir().join("Rebecca"))
        })
        .ok_or(RebeccaError::UserDirsUnavailable)?;
    let state_dir = env_path("REBECCA_STATE_DIR")
        .or_else(|| {
            base_dirs
                .as_ref()
                .map(|dirs| dirs.data_local_dir().join("Rebecca").join("state"))
        })
        .ok_or(RebeccaError::UserDirsUnavailable)?;
    let cache_dir = env_path("REBECCA_CACHE_DIR")
        .or_else(|| {
            base_dirs
                .as_ref()
                .map(|dirs| dirs.cache_dir().join("Rebecca").join("cache"))
        })
        .ok_or(RebeccaError::UserDirsUnavailable)?;
    let history_file =
        env_path("REBECCA_HISTORY_FILE").unwrap_or_else(|| state_dir.join("history.jsonl"));

    Ok(AppPaths {
        config_file: config_dir.join("config.toml"),
        history_file,
        config_dir,
        state_dir,
        cache_dir,
    })
}

fn env_path(key: &str) -> Option<PathBuf> {
    let value = std::env::var_os(key)?;
    if value.is_empty() {
        return None;
    }
    Some(PathBuf::from(value))
}

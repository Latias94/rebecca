use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rebecca::core::config::AppRuntimeConfig;
use rebecca::core::disk_map::DiskMapSortField;
use rebecca::core::scan::ScanBackendKind;
use serde::{Deserialize, Serialize};

use crate::tui::app::TuiApp;
use crate::tui::model::TuiScreen;
use crate::tui::view::ViewOptions;

const PREFERENCES_VERSION: u32 = 1;
const PREFERENCES_FILE_NAME: &str = "tui-preferences.json";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TuiPreferenceLoad {
    pub(crate) preferences: TuiPreferences,
    pub(crate) warning: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TuiPreferences {
    pub(crate) version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) last_screen: Option<TuiScreen>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) sort: Option<DiskMapSortField>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) entry_limit: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) scan_backend: Option<ScanBackendKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) screen_reader: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) no_color: Option<bool>,
}

impl Default for TuiPreferences {
    fn default() -> Self {
        Self {
            version: PREFERENCES_VERSION,
            last_screen: None,
            sort: None,
            entry_limit: None,
            scan_backend: None,
            screen_reader: None,
            no_color: None,
        }
    }
}

impl TuiPreferences {
    pub(crate) fn load(path: &Path) -> TuiPreferenceLoad {
        let Ok(raw) = fs::read_to_string(path) else {
            return TuiPreferenceLoad {
                preferences: Self::default(),
                warning: None,
            };
        };
        match serde_json::from_str::<Self>(&raw) {
            Ok(mut preferences) if preferences.version == PREFERENCES_VERSION => {
                preferences.last_screen = preferences.last_screen.and_then(persistable_screen);
                TuiPreferenceLoad {
                    preferences,
                    warning: None,
                }
            }
            Ok(_) => TuiPreferenceLoad {
                preferences: Self::default(),
                warning: Some("Ignored unsupported TUI preferences version.".to_string()),
            },
            Err(err) => TuiPreferenceLoad {
                preferences: Self::default(),
                warning: Some(format!("Ignored corrupt TUI preferences: {err}")),
            },
        }
    }

    pub(crate) fn from_app(app: &TuiApp, view_options: ViewOptions) -> Self {
        Self {
            version: PREFERENCES_VERSION,
            last_screen: persistable_screen(app.screen),
            sort: Some(app.sort),
            entry_limit: Some(app.entry_limit),
            scan_backend: Some(app.scan_backend),
            screen_reader: Some(!view_options.visual_bars),
            no_color: Some(!view_options.color),
        }
    }

    pub(crate) fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "failed to create TUI preference directory {}",
                    parent.display()
                )
            })?;
        }
        let mut encoded = serde_json::to_string_pretty(self)
            .context("failed to encode TUI preferences as JSON")?;
        encoded.push('\n');
        fs::write(path, encoded)
            .with_context(|| format!("failed to write TUI preferences {}", path.display()))
    }
}

pub(crate) fn preferences_path(runtime_config: &AppRuntimeConfig) -> PathBuf {
    runtime_config
        .app_paths
        .state_dir
        .join(PREFERENCES_FILE_NAME)
}

pub(crate) fn persistable_screen(screen: TuiScreen) -> Option<TuiScreen> {
    match screen {
        TuiScreen::Map | TuiScreen::Treemap | TuiScreen::Types | TuiScreen::Extensions => {
            Some(screen)
        }
        TuiScreen::RootPicker
        | TuiScreen::Busy
        | TuiScreen::Preview
        | TuiScreen::Confirm
        | TuiScreen::Executed
        | TuiScreen::History
        | TuiScreen::Help
        | TuiScreen::Error => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_preferences_load_defaults() {
        let temp = tempfile::tempdir().unwrap();

        let loaded = TuiPreferences::load(&temp.path().join("missing.json"));

        assert_eq!(loaded.preferences, TuiPreferences::default());
        assert!(loaded.warning.is_none());
    }

    #[test]
    fn corrupt_preferences_load_defaults_with_warning() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("tui-preferences.json");
        fs::write(&path, "{not-json").unwrap();

        let loaded = TuiPreferences::load(&path);

        assert_eq!(loaded.preferences, TuiPreferences::default());
        assert!(loaded.warning.unwrap().contains("Ignored corrupt"));
    }

    #[test]
    fn save_creates_parent_directory_and_omits_private_state() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("state").join("tui-preferences.json");
        let preferences = TuiPreferences {
            version: PREFERENCES_VERSION,
            last_screen: Some(TuiScreen::Treemap),
            sort: Some(DiskMapSortField::Files),
            entry_limit: Some(500),
            scan_backend: Some(ScanBackendKind::PortableRecursive),
            screen_reader: Some(true),
            no_color: Some(true),
        };

        preferences.save(&path).unwrap();

        let raw = fs::read_to_string(&path).unwrap();
        assert!(raw.contains("treemap"));
        assert!(!raw.contains("root"));
        assert!(!raw.contains("filter"));
        assert!(!raw.contains("basket"));
        assert_eq!(TuiPreferences::load(&path).preferences, preferences);
    }
}

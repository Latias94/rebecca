use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

use crate::applications::InstalledApplication;
use crate::environment::Environment;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppLeftoverCandidate {
    pub app: InstalledApplication,
    pub path: PathBuf,
    pub source: AppLeftoverSource,
    pub modified_at_unix_seconds: Option<u64>,
}

impl AppLeftoverCandidate {
    pub fn rule_id(&self) -> &'static str {
        match self.source {
            AppLeftoverSource::LocalAppData => "windows.app-leftover-local-cache",
            AppLeftoverSource::RoamingAppData => "windows.app-leftover-roaming-cache",
            AppLeftoverSource::LocalLowAppData => "windows.app-leftover-local-low-cache",
        }
    }

    pub fn restore_hint(&self) -> String {
        format!(
            "{} {} cache data is rebuildable.",
            self.app.display_name(),
            self.source.label()
        )
    }

    pub fn deletion_style(&self) -> AppLeftoverDeletionStyle {
        AppLeftoverDeletionStyle::PreserveRootContents
    }

    pub fn advice_context(&self) -> AppLeftoverAdviceContext {
        AppLeftoverAdviceContext {
            app: AppLeftoverAppContext {
                stable_id: self.app.stable_id().to_string(),
                display_name: self.app.display_name().to_string(),
                publisher: self.app.publisher.clone(),
            },
            source: self.source,
            target_leaf: self
                .path
                .file_name()
                .map(|leaf| leaf.to_string_lossy().into_owned())
                .unwrap_or_default(),
            deletion_style: self.deletion_style(),
            modified_at_unix_seconds: self.modified_at_unix_seconds,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppLeftoverAdviceContext {
    pub app: AppLeftoverAppContext,
    pub source: AppLeftoverSource,
    pub target_leaf: String,
    pub deletion_style: AppLeftoverDeletionStyle,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified_at_unix_seconds: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppLeftoverAppContext {
    pub stable_id: String,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AppLeftoverDeletionStyle {
    PreserveRootContents,
}

impl AppLeftoverDeletionStyle {
    pub fn label(self) -> &'static str {
        match self {
            Self::PreserveRootContents => "preserve-root-contents",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AppLeftoverSource {
    LocalAppData,
    RoamingAppData,
    LocalLowAppData,
}

impl AppLeftoverSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::LocalAppData => "local-app-data",
            Self::RoamingAppData => "roaming-app-data",
            Self::LocalLowAppData => "local-low-app-data",
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct LeftoverRoot<'a> {
    env_key: &'static str,
    source: AppLeftoverSource,
    children: &'a [&'a str],
}

const LOCAL_APP_DATA_CHILDREN: &[&str] = &["Cache", "Code Cache", "GPUCache", "CachedData"];
const ROAMING_APP_DATA_CHILDREN: &[&str] = &["Cache", "Code Cache", "GPUCache", "CachedData"];
const LOCAL_LOW_APP_DATA_CHILDREN: &[&str] = &["Cache"];

const LEFTOVER_ROOTS: &[LeftoverRoot<'_>] = &[
    LeftoverRoot {
        env_key: "LOCALAPPDATA",
        source: AppLeftoverSource::LocalAppData,
        children: LOCAL_APP_DATA_CHILDREN,
    },
    LeftoverRoot {
        env_key: "APPDATA",
        source: AppLeftoverSource::RoamingAppData,
        children: ROAMING_APP_DATA_CHILDREN,
    },
    LeftoverRoot {
        env_key: "USERPROFILE",
        source: AppLeftoverSource::LocalLowAppData,
        children: LOCAL_LOW_APP_DATA_CHILDREN,
    },
];

pub fn derive_app_leftover_candidates(
    applications: &[InstalledApplication],
    env: &impl Environment,
) -> Vec<AppLeftoverCandidate> {
    let mut seen = BTreeSet::new();
    let mut candidates = Vec::new();

    for app in applications {
        for name in app_name_variants(app) {
            for root in LEFTOVER_ROOTS {
                let Some(root_path) = root_path(root, env) else {
                    continue;
                };
                let app_root = root_path.join(&name);
                for child in root.children {
                    let path = app_root.join(child);
                    if !path.exists() {
                        continue;
                    }
                    let key = format!("{}|{}", app.stable_id(), path_key(&path));
                    if seen.insert(key) {
                        let modified_at_unix_seconds = modified_at_unix_seconds(&path);
                        candidates.push(AppLeftoverCandidate {
                            app: app.clone(),
                            path,
                            source: root.source,
                            modified_at_unix_seconds,
                        });
                    }
                }
            }
        }
    }

    candidates.sort_by(|left, right| {
        left.app
            .stable_id()
            .cmp(right.app.stable_id())
            .then_with(|| left.path.cmp(&right.path))
    });
    candidates
}

fn root_path(root: &LeftoverRoot<'_>, env: &impl Environment) -> Option<PathBuf> {
    let value = env.get(root.env_key)?;
    if value.is_empty() {
        return None;
    }

    let base = PathBuf::from(value);
    match root.source {
        AppLeftoverSource::LocalLowAppData => Some(base.join("AppData").join("LocalLow")),
        AppLeftoverSource::LocalAppData | AppLeftoverSource::RoamingAppData => Some(base),
    }
}

fn app_name_variants(app: &InstalledApplication) -> Vec<String> {
    let normalized = normalize_app_name(app.display_name());
    if !is_specific_app_name(&normalized) {
        return Vec::new();
    }

    let mut variants = Vec::new();
    push_unique(&mut variants, normalized);
    variants
}

fn normalize_app_name(value: &str) -> String {
    value
        .trim()
        .trim_end_matches(".app")
        .trim()
        .chars()
        .filter(|ch| !matches!(*ch, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_specific_app_name(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    value.chars().filter(|ch| ch.is_alphanumeric()).count() >= 3
        && !matches!(
            lower.as_str(),
            "app"
                | "apps"
                | "application"
                | "applications"
                | "cache"
                | "data"
                | "local"
                | "program"
                | "programs"
                | "setup"
                | "update"
                | "windows"
        )
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if value.trim().is_empty() {
        return;
    }

    if values
        .iter()
        .all(|existing| !existing.eq_ignore_ascii_case(&value))
    {
        values.push(value);
    }
}

fn path_key(path: &Path) -> String {
    let mut normalized = path.as_os_str().to_string_lossy().replace('\\', "/");
    while normalized.ends_with('/') && normalized.len() > 3 {
        normalized.pop();
    }
    normalized.to_ascii_lowercase()
}

fn modified_at_unix_seconds(path: &Path) -> Option<u64> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
}

#[cfg(test)]
mod tests {
    use crate::applications::InstalledApplication;
    use crate::environment::MapEnvironment;

    use super::*;

    #[test]
    fn derives_cache_children_from_appdata_roots() {
        let temp = tempfile::tempdir().unwrap();
        let app = InstalledApplication::new("hklm/example", "Example App", Vec::new());
        let local = temp.path().join("AppData").join("Local");
        let roaming = temp.path().join("AppData").join("Roaming");
        let local_low = temp.path().join("AppData").join("LocalLow");
        let env = MapEnvironment::new()
            .with_var("LOCALAPPDATA", local.as_os_str().to_os_string())
            .with_var("APPDATA", roaming.as_os_str().to_os_string())
            .with_var("USERPROFILE", temp.path().as_os_str().to_os_string());
        let local_cache = local.join("Example App").join("Cache");
        let local_code_cache = local.join("Example App").join("Code Cache");
        let roaming_cache = roaming.join("Example App").join("Cache");
        let local_low_cache = local_low.join("Example App").join("Cache");
        std::fs::create_dir_all(&local_cache).unwrap();
        std::fs::write(local_cache.join("state.bin"), b"keep").unwrap();
        std::fs::create_dir_all(&local_code_cache).unwrap();
        std::fs::write(local_code_cache.join("state.bin"), b"keep").unwrap();
        std::fs::create_dir_all(&roaming_cache).unwrap();
        std::fs::write(roaming_cache.join("state.bin"), b"keep").unwrap();
        std::fs::create_dir_all(&local_low_cache).unwrap();
        std::fs::write(local_low_cache.join("state.bin"), b"keep").unwrap();

        let candidates = derive_app_leftover_candidates(&[app], &env);
        let paths = candidates
            .iter()
            .map(|candidate| candidate.path.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        assert_eq!(paths.len(), 4);
        assert!(
            paths
                .iter()
                .any(|path| path.ends_with(r"AppData\Local\Example App\Cache"))
        );
        assert!(
            paths
                .iter()
                .any(|path| path.ends_with(r"AppData\Local\Example App\Code Cache"))
        );
        assert!(
            paths
                .iter()
                .any(|path| path.ends_with(r"AppData\Roaming\Example App\Cache"))
        );
        assert!(
            paths
                .iter()
                .any(|path| path.ends_with(r"AppData\LocalLow\Example App\Cache"))
        );
    }

    #[test]
    fn skips_generic_or_empty_application_names() {
        let app = InstalledApplication::new("hklm/app", "App", Vec::new());
        let temp = tempfile::tempdir().unwrap();
        let env = MapEnvironment::new().with_var(
            "LOCALAPPDATA",
            temp.path().join("AppData").join("Local").into_os_string(),
        );

        let candidates = derive_app_leftover_candidates(&[app], &env);

        assert!(candidates.is_empty());
    }
}

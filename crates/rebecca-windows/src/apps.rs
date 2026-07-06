#[cfg(windows)]
use std::collections::BTreeSet;
#[cfg(windows)]
use std::path::{Path, PathBuf};

use rebecca_core::applications::InstalledApplication;
#[cfg(windows)]
use rebecca_core::error::RebeccaError;
use rebecca_core::error::Result;

#[cfg(windows)]
use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ};
#[cfg(windows)]
use winreg::{HKEY, RegKey};

#[cfg(windows)]
#[derive(Clone, Copy)]
struct UninstallRegistryRoot {
    root: HKEY,
    key_path: &'static str,
    root_label: &'static str,
}

#[cfg(windows)]
const UNINSTALL_REGISTRY_ROOTS: [UninstallRegistryRoot; 4] = [
    UninstallRegistryRoot {
        root: HKEY_CURRENT_USER,
        key_path: "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
        root_label: "hkcu",
    },
    UninstallRegistryRoot {
        root: HKEY_LOCAL_MACHINE,
        key_path: "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
        root_label: "hklm",
    },
    UninstallRegistryRoot {
        root: HKEY_LOCAL_MACHINE,
        key_path: "SOFTWARE\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
        root_label: "hklm-wow6432",
    },
    UninstallRegistryRoot {
        root: HKEY_CURRENT_USER,
        key_path: "Software\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
        root_label: "hkcu-wow6432",
    },
];

#[cfg(windows)]
#[derive(Debug, Clone, Default)]
struct RawUninstallEntry {
    key_name: String,
    display_name: Option<String>,
    publisher: Option<String>,
    install_location: Option<String>,
    display_icon: Option<String>,
    uninstall_string: Option<String>,
    system_component: Option<u32>,
}

pub fn discover_installed_applications() -> Result<Vec<InstalledApplication>> {
    platform::discover_installed_applications()
}

#[cfg(windows)]
mod platform {
    use super::*;

    pub fn discover_installed_applications() -> Result<Vec<InstalledApplication>> {
        discover_installed_applications_with(read_uninstall_entries)
    }

    fn read_uninstall_entries(
        source: UninstallRegistryRoot,
    ) -> Result<Vec<(String, RawUninstallEntry)>> {
        let root =
            match RegKey::predef(source.root).open_subkey_with_flags(source.key_path, KEY_READ) {
                Ok(root) => root,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
                Err(err) => {
                    return Err(RebeccaError::ApplicationDiscoveryFailed(format!(
                        "could not open uninstall registry root {}: {err}",
                        source.key_path
                    )));
                }
            };

        let mut entries = Vec::new();
        for key_name in root.enum_keys().filter_map(std::result::Result::ok) {
            let Ok(key) = root.open_subkey_with_flags(&key_name, KEY_READ) else {
                continue;
            };

            entries.push((
                source.root_label.to_string(),
                RawUninstallEntry {
                    display_name: registry_string(&key, "DisplayName"),
                    publisher: registry_string(&key, "Publisher"),
                    install_location: registry_string(&key, "InstallLocation"),
                    display_icon: registry_string(&key, "DisplayIcon"),
                    uninstall_string: registry_string(&key, "UninstallString"),
                    system_component: registry_u32(&key, "SystemComponent"),
                    key_name,
                },
            ));
        }

        Ok(entries)
    }

    fn registry_string(key: &RegKey, value_name: &str) -> Option<String> {
        key.get_value::<String, _>(value_name)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    }

    fn registry_u32(key: &RegKey, value_name: &str) -> Option<u32> {
        key.get_value::<u32, _>(value_name).ok()
    }
}

#[cfg(not(windows))]
mod platform {
    use super::*;

    pub fn discover_installed_applications() -> Result<Vec<InstalledApplication>> {
        Ok(Vec::new())
    }
}

#[cfg(windows)]
fn discover_installed_applications_with<F>(mut read_entries: F) -> Result<Vec<InstalledApplication>>
where
    F: FnMut(UninstallRegistryRoot) -> Result<Vec<(String, RawUninstallEntry)>>,
{
    let mut apps = Vec::new();

    for source in UNINSTALL_REGISTRY_ROOTS {
        let Ok(entries) = read_entries(source) else {
            continue;
        };
        apps.extend(entries.into_iter().filter_map(|(root_label, entry)| {
            application_from_uninstall_entry(&root_label, source.key_path, entry)
        }));
    }

    Ok(dedupe_applications(apps))
}

#[cfg(windows)]
fn application_from_uninstall_entry(
    root_label: &str,
    source_path: &str,
    entry: RawUninstallEntry,
) -> Option<InstalledApplication> {
    if entry.system_component == Some(1) {
        return None;
    }

    let display_name = entry.display_name?.trim().to_string();
    if display_name.is_empty() {
        return None;
    }

    let mut install_locations = Vec::new();
    if let Some(path) = entry
        .install_location
        .as_deref()
        .and_then(install_location_from_value)
    {
        install_locations.push(path);
    }
    if let Some(path) = entry
        .display_icon
        .as_deref()
        .and_then(install_root_from_command_like_value)
    {
        install_locations.push(path);
    }
    if let Some(path) = entry
        .uninstall_string
        .as_deref()
        .and_then(install_root_from_command_like_value)
    {
        install_locations.push(path);
    }

    let stable_id = format!(
        "{root_label}:{}:{}",
        source_path.to_ascii_lowercase(),
        entry.key_name.to_ascii_lowercase()
    );

    let mut app = InstalledApplication::new(stable_id, display_name, install_locations);
    if let Some(publisher) = entry.publisher {
        app = app.with_publisher(publisher);
    }

    Some(app)
}

#[cfg(windows)]
fn install_location_from_value(value: &str) -> Option<PathBuf> {
    let trimmed = value.trim().trim_matches('"');
    if trimmed.is_empty() {
        return None;
    }

    Some(PathBuf::from(trimmed))
}

#[cfg(windows)]
fn install_root_from_command_like_value(value: &str) -> Option<PathBuf> {
    let executable = extract_executable_path(value)?;
    Path::new(&executable).parent().map(PathBuf::from)
}

#[cfg(windows)]
fn extract_executable_path(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }

    let candidate = if let Some(rest) = trimmed.strip_prefix('"') {
        rest.split_once('"').map(|(path, _)| path.to_string())?
    } else {
        let lower = trimmed.to_ascii_lowercase();
        let mut search_start = 0;
        loop {
            let relative_end = lower[search_start..].find(".exe")?;
            let exe_end = search_start + relative_end + 4;
            let next_char = trimmed[exe_end..].chars().next();
            if next_char.is_none()
                || next_char.is_some_and(|ch| ch.is_whitespace() || ch == '"' || ch == ',')
            {
                break trimmed[..exe_end].trim().trim_matches(',').to_string();
            }
            search_start = exe_end;
        }
    };

    if candidate.trim().is_empty() {
        None
    } else {
        Some(candidate)
    }
}

#[cfg(windows)]
fn dedupe_applications(
    applications: impl IntoIterator<Item = InstalledApplication>,
) -> Vec<InstalledApplication> {
    let mut seen = BTreeSet::new();
    let mut deduped = Vec::new();

    for app in applications {
        let key = application_key(&app);
        if seen.insert(key) {
            deduped.push(app);
        }
    }

    deduped.sort_by(|left, right| {
        left.display_name()
            .to_ascii_lowercase()
            .cmp(&right.display_name().to_ascii_lowercase())
            .then_with(|| left.stable_id().cmp(right.stable_id()))
    });
    deduped
}

#[cfg(windows)]
fn application_key(app: &InstalledApplication) -> String {
    if let Some(first_location) = app.install_locations().first() {
        return format!(
            "{}|{}",
            app.display_name().trim().to_ascii_lowercase(),
            path_key(first_location)
        );
    }

    app.stable_id().trim().to_ascii_lowercase()
}

#[cfg(windows)]
fn path_key(path: &Path) -> String {
    let mut normalized = path.as_os_str().to_string_lossy().replace('\\', "/");
    while normalized.ends_with('/') && normalized.len() > 3 {
        normalized.pop();
    }
    normalized.to_ascii_lowercase()
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn command_like_value_extracts_quoted_executable_root() {
        let path =
            install_root_from_command_like_value(r#""C:\Program Files\Example\uninstall.exe" /S"#);

        assert_eq!(path, Some(PathBuf::from(r"C:\Program Files\Example")));
    }

    #[test]
    fn command_like_value_extracts_unquoted_executable_root() {
        let path =
            install_root_from_command_like_value(r"C:\Program Files\Example\uninstall.exe /S");

        assert_eq!(path, Some(PathBuf::from(r"C:\Program Files\Example")));
    }

    #[test]
    fn uninstall_entry_with_display_name_and_install_location_becomes_application() {
        let app = application_from_uninstall_entry(
            "hklm",
            r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
            RawUninstallEntry {
                key_name: "Example".to_string(),
                display_name: Some("Example App".to_string()),
                publisher: Some("Example Inc".to_string()),
                install_location: Some(r"C:\Program Files\Example".to_string()),
                ..RawUninstallEntry::default()
            },
        )
        .expect("entry should be discovered");

        assert_eq!(app.display_name(), "Example App");
        assert_eq!(app.publisher.as_deref(), Some("Example Inc"));
        assert_eq!(
            app.install_locations(),
            &[PathBuf::from(r"C:\Program Files\Example")]
        );
    }

    #[test]
    fn uninstall_entry_skips_system_component_and_missing_display_name() {
        assert!(
            application_from_uninstall_entry(
                "hklm",
                "path",
                RawUninstallEntry {
                    key_name: "System".to_string(),
                    display_name: Some("System".to_string()),
                    system_component: Some(1),
                    ..RawUninstallEntry::default()
                },
            )
            .is_none()
        );
        assert!(
            application_from_uninstall_entry(
                "hklm",
                "path",
                RawUninstallEntry {
                    key_name: "Missing".to_string(),
                    ..RawUninstallEntry::default()
                },
            )
            .is_none()
        );
    }

    #[test]
    fn uninstall_inventory_skips_failed_sources_without_failing_run() {
        let mut calls = 0usize;
        let apps = discover_installed_applications_with(|source| {
            calls += 1;
            if calls == 1 {
                return Err(RebeccaError::ApplicationDiscoveryFailed(
                    "source unavailable".to_string(),
                ));
            }

            Ok(vec![(
                source.root_label.to_string(),
                RawUninstallEntry {
                    key_name: "Example".to_string(),
                    display_name: Some("Example App".to_string()),
                    install_location: Some(r"C:\Program Files\Example".to_string()),
                    ..RawUninstallEntry::default()
                },
            )])
        })
        .unwrap();

        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].display_name(), "Example App");
    }
}

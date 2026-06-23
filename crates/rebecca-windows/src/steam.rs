use std::path::PathBuf;

use rebecca_core::applications::{ApplicationDiscovery, SteamInstallation};
use rebecca_core::error::{RebeccaError, Result};

#[cfg(windows)]
use winreg::enums::{HKEY_CLASSES_ROOT, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ};
#[cfg(windows)]
use winreg::{HKEY, RegKey};

#[derive(Debug, Default, Clone, Copy)]
pub struct WindowsApplicationDiscovery;

#[cfg(windows)]
#[derive(Clone, Copy)]
struct SteamRegistrySource {
    root: HKEY,
    key_path: &'static str,
    value_name: &'static str,
    parser: fn(&str) -> Option<PathBuf>,
}

impl WindowsApplicationDiscovery {
    pub fn new() -> Self {
        Self
    }
}

#[cfg(windows)]
impl ApplicationDiscovery for WindowsApplicationDiscovery {
    fn steam_installation(&self) -> Result<Option<SteamInstallation>> {
        discover_steam_installation()
    }
}

#[cfg(not(windows))]
impl ApplicationDiscovery for WindowsApplicationDiscovery {
    fn steam_installation(&self) -> Result<Option<SteamInstallation>> {
        Ok(None)
    }
}

#[cfg(windows)]
fn discover_steam_installation() -> Result<Option<SteamInstallation>> {
    discover_steam_installation_with(registry_string_value)
}

#[cfg(windows)]
fn discover_steam_installation_with<F>(mut lookup: F) -> Result<Option<SteamInstallation>>
where
    F: FnMut(HKEY, &str, &str) -> Result<Option<String>>,
{
    for source in steam_registry_sources() {
        if let Some(path) = resolve_steam_registry_source(source, &mut lookup)? {
            return Ok(Some(steam_installation_from_path(path)));
        }
    }

    Ok(None)
}

#[cfg(windows)]
fn steam_registry_sources() -> [SteamRegistrySource; 5] {
    [
        SteamRegistrySource {
            root: HKEY_CURRENT_USER,
            key_path: "Software\\Valve\\Steam",
            value_name: "SteamPath",
            parser: install_path_from_value,
        },
        SteamRegistrySource {
            root: HKEY_CURRENT_USER,
            key_path: "Software\\Valve\\Steam",
            value_name: "SteamExe",
            parser: install_root_from_executable_path,
        },
        SteamRegistrySource {
            root: HKEY_LOCAL_MACHINE,
            key_path: "SOFTWARE\\Valve\\Steam",
            value_name: "InstallPath",
            parser: install_path_from_value,
        },
        SteamRegistrySource {
            root: HKEY_LOCAL_MACHINE,
            key_path: "SOFTWARE\\WOW6432Node\\Valve\\Steam",
            value_name: "InstallPath",
            parser: install_path_from_value,
        },
        SteamRegistrySource {
            root: HKEY_CLASSES_ROOT,
            key_path: "steam\\Shell\\Open\\Command",
            value_name: "",
            parser: command_install_path_from_command,
        },
    ]
}

#[cfg(windows)]
fn resolve_steam_registry_source<F>(
    source: SteamRegistrySource,
    lookup: &mut F,
) -> Result<Option<PathBuf>>
where
    F: FnMut(HKEY, &str, &str) -> Result<Option<String>>,
{
    Ok(lookup(source.root, source.key_path, source.value_name)?
        .as_deref()
        .and_then(source.parser))
}

#[cfg(windows)]
fn registry_string_value(root: HKEY, key_path: &str, value_name: &str) -> Result<Option<String>> {
    let key = match RegKey::predef(root).open_subkey_with_flags(key_path, KEY_READ) {
        Ok(key) => key,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(RebeccaError::ApplicationDiscoveryFailed(format!(
                "could not open Steam registry key {key_path}: {err}"
            )));
        }
    };

    let value: String = match key.get_value::<String, _>(value_name) {
        Ok(value) if !value.trim().is_empty() => value,
        Ok(_) => return Ok(None),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(RebeccaError::ApplicationDiscoveryFailed(format!(
                "could not read {value_name} from {key_path}: {err}"
            )));
        }
    };

    Ok(Some(value))
}

#[cfg(windows)]
fn install_root_from_executable_path(executable: &str) -> Option<PathBuf> {
    PathBuf::from(executable).parent().map(PathBuf::from)
}

#[cfg(windows)]
fn install_path_from_value(value: &str) -> Option<PathBuf> {
    Some(PathBuf::from(value))
}

#[cfg(windows)]
fn command_install_path_from_command(command: &str) -> Option<PathBuf> {
    let executable = extract_command_executable(command)?;
    install_root_from_executable_path(&executable)
}

#[cfg(windows)]
fn extract_command_executable(command: &str) -> Option<String> {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Some(rest) = trimmed.strip_prefix('"') {
        rest.split_once('"').map(|(path, _)| path.to_string())
    } else {
        let lower = trimmed.to_ascii_lowercase();
        let mut search_start = 0;

        while let Some(relative_end) = lower[search_start..].find(".exe") {
            let exe_end = search_start + relative_end + 4;
            let next_char = trimmed[exe_end..].chars().next();

            if next_char.is_none() || next_char.is_some_and(|ch| ch.is_whitespace() || ch == '"') {
                return Some(trimmed[..exe_end].trim().to_string());
            }

            search_start = exe_end;
        }

        None
    }
}

#[cfg(not(windows))]
fn discover_steam_installation() -> Result<Option<SteamInstallation>> {
    Ok(None)
}

#[cfg(windows)]
pub fn steam_installation_from_path(steam_path: impl Into<PathBuf>) -> SteamInstallation {
    let steam_path = steam_path.into();

    SteamInstallation::from_install_path(&steam_path)
        .unwrap_or_else(|_| SteamInstallation::new(steam_path, Vec::new()))
}

#[cfg(all(test, windows))]
mod tests {
    use std::fs;

    use super::{
        command_install_path_from_command, discover_steam_installation_with,
        extract_command_executable, install_root_from_executable_path,
        steam_installation_from_path,
    };
    use winreg::HKEY;
    use winreg::enums::HKEY_CURRENT_USER;

    #[test]
    fn steam_installation_falls_back_to_install_root_when_libraryfolders_is_unreadable() {
        let temp = tempfile::tempdir().unwrap();
        let install_path = temp.path().join("Steam");
        let library_file = install_path.join("steamapps").join("libraryfolders.vdf");
        fs::create_dir_all(&library_file).unwrap();

        let installation = steam_installation_from_path(&install_path);

        assert_eq!(installation.install_path(), install_path.as_path());
        assert!(installation.library_paths().is_empty());
    }

    #[test]
    fn steam_installation_reads_libraryfolders_when_file_is_present() {
        let temp = tempfile::tempdir().unwrap();
        let install_path = temp.path().join("Steam");
        let steamapps = install_path.join("steamapps");
        fs::create_dir_all(&steamapps).unwrap();
        fs::write(
            steamapps.join("libraryfolders.vdf"),
            r#"
"libraryfolders"
{
    "0"
    {
        "path"      "D:\\SteamLibrary"
    }
}
"#,
        )
        .unwrap();

        let installation = steam_installation_from_path(&install_path);

        assert_eq!(installation.install_path(), install_path.as_path());
        assert_eq!(
            installation.library_paths(),
            &[std::path::PathBuf::from(r"D:\SteamLibrary")]
        );
    }

    #[test]
    fn command_install_path_from_command_handles_quoted_paths() {
        let command = r#""C:\Program Files (x86)\Steam\steam.exe" -silent"#;

        let install_path = command_install_path_from_command(command);

        assert_eq!(
            install_path,
            Some(std::path::PathBuf::from(r"C:\Program Files (x86)\Steam"))
        );
    }

    #[test]
    fn command_install_path_from_command_handles_unquoted_paths_with_spaces() {
        let command = r"C:\Program Files (x86)\Steam\steam.exe -silent";

        let install_path = command_install_path_from_command(command);

        assert_eq!(
            install_path,
            Some(std::path::PathBuf::from(r"C:\Program Files (x86)\Steam"))
        );
    }

    #[test]
    fn command_install_path_from_command_returns_none_without_executable_extension() {
        let command = r#"steam://uninstall/123"#;

        let install_path = command_install_path_from_command(command);

        assert_eq!(install_path, None);
    }

    #[test]
    fn install_root_from_executable_path_uses_parent_directory() {
        let install_path =
            install_root_from_executable_path(r"C:\Program Files (x86)\Steam\steam.exe");

        assert_eq!(
            install_path,
            Some(std::path::PathBuf::from(r"C:\Program Files (x86)\Steam"))
        );
    }

    #[test]
    fn discover_steam_installation_prefers_steam_path_over_legacy_values() {
        let mut queries = Vec::new();
        let mut lookup = |root: HKEY, key_path: &str, value_name: &str| {
            queries.push((root, key_path.to_string(), value_name.to_string()));
            Ok(Some(match value_name {
                "SteamPath" => r"C:\Steam".to_string(),
                "SteamExe" => r"C:\Steam\steam.exe".to_string(),
                "InstallPath" => r"C:\LegacySteam".to_string(),
                _ => r#""C:\Steam\steam.exe" -- "%1""#.to_string(),
            }))
        };

        let installation = discover_steam_installation_with(&mut lookup)
            .unwrap()
            .expect("Steam should be discovered");

        assert_eq!(
            installation.install_path(),
            std::path::Path::new(r"C:\Steam")
        );
        assert_eq!(
            queries,
            vec![(
                HKEY_CURRENT_USER,
                "Software\\Valve\\Steam".to_string(),
                "SteamPath".to_string()
            )]
        );
    }

    #[test]
    fn discover_steam_installation_uses_steamexe_before_lm_install_path() {
        let mut queries = Vec::new();
        let mut lookup = |root: HKEY, key_path: &str, value_name: &str| {
            queries.push((root, key_path.to_string(), value_name.to_string()));
            Ok(match value_name {
                "SteamPath" => None,
                "SteamExe" => Some(r"C:\Steam\steam.exe".to_string()),
                "InstallPath" => Some(r"C:\LegacySteam".to_string()),
                _ => Some(r#""C:\Steam\steam.exe" -- "%1""#.to_string()),
            })
        };

        let installation = discover_steam_installation_with(&mut lookup)
            .unwrap()
            .expect("Steam should be discovered");

        assert_eq!(
            installation.install_path(),
            std::path::Path::new(r"C:\Steam")
        );
        assert_eq!(
            queries,
            vec![
                (
                    HKEY_CURRENT_USER,
                    "Software\\Valve\\Steam".to_string(),
                    "SteamPath".to_string()
                ),
                (
                    HKEY_CURRENT_USER,
                    "Software\\Valve\\Steam".to_string(),
                    "SteamExe".to_string()
                )
            ]
        );
    }

    #[test]
    fn extract_command_executable_handles_quoted_paths() {
        let command = r#""C:\Program Files (x86)\Steam\steam.exe" -silent"#;

        let executable = extract_command_executable(command);

        assert_eq!(
            executable,
            Some(r"C:\Program Files (x86)\Steam\steam.exe".to_string())
        );
    }

    #[test]
    fn extract_command_executable_handles_unquoted_paths_with_spaces() {
        let command = r"C:\Program Files (x86)\Steam\steam.exe -silent";

        let executable = extract_command_executable(command);

        assert_eq!(
            executable,
            Some(r"C:\Program Files (x86)\Steam\steam.exe".to_string())
        );
    }

    #[test]
    fn extract_command_executable_trims_whitespace() {
        let command = r#"  "C:\Steam\steam.exe"  "#;

        let executable = extract_command_executable(command);

        assert_eq!(executable, Some(r"C:\Steam\steam.exe".to_string()));
    }
}

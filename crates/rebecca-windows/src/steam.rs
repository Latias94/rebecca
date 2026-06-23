use std::path::PathBuf;

use rebecca_core::applications::{ApplicationDiscovery, SteamInstallation};
use rebecca_core::error::{RebeccaError, Result};

#[cfg(windows)]
use winreg::enums::{HKEY_CLASSES_ROOT, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ};
#[cfg(windows)]
use winreg::{HKEY, RegKey};

#[derive(Debug, Default, Clone, Copy)]
pub struct WindowsApplicationDiscovery;

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
    match registry_install_path(HKEY_CURRENT_USER, "Software\\Valve\\Steam", "SteamPath")? {
        Some(path) => Ok(Some(steam_installation_from_path(path))),
        None => discover_steam_installation_from_legacy_registry(),
    }
}

#[cfg(windows)]
fn discover_steam_installation_from_legacy_registry() -> Result<Option<SteamInstallation>> {
    for key_path in [
        "SOFTWARE\\Valve\\Steam",
        "SOFTWARE\\WOW6432Node\\Valve\\Steam",
    ] {
        match registry_install_path(HKEY_LOCAL_MACHINE, key_path, "InstallPath")? {
            Some(path) => return Ok(Some(steam_installation_from_path(path))),
            None => {}
        }
    }

    match registry_command_install_path(HKEY_CLASSES_ROOT, "steam\\Shell\\Open\\Command")? {
        Some(path) => Ok(Some(steam_installation_from_path(path))),
        None => Ok(None),
    }
}

#[cfg(windows)]
fn registry_install_path(root: HKEY, key_path: &str, value_name: &str) -> Result<Option<PathBuf>> {
    let key = match RegKey::predef(root).open_subkey_with_flags(key_path, KEY_READ) {
        Ok(key) => key,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(RebeccaError::ApplicationDiscoveryFailed(format!(
                "could not open Steam registry key {key_path}: {err}"
            )));
        }
    };

    let path: String = match key.get_value::<String, _>(value_name) {
        Ok(path) if !path.trim().is_empty() => path,
        Ok(_) => return Ok(None),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(RebeccaError::ApplicationDiscoveryFailed(format!(
                "could not read {value_name} from {key_path}: {err}"
            )));
        }
    };

    Ok(Some(PathBuf::from(path)))
}

#[cfg(windows)]
fn registry_command_install_path(root: HKEY, key_path: &str) -> Result<Option<PathBuf>> {
    let key = match RegKey::predef(root).open_subkey_with_flags(key_path, KEY_READ) {
        Ok(key) => key,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(RebeccaError::ApplicationDiscoveryFailed(format!(
                "could not open Steam registry key {key_path}: {err}"
            )));
        }
    };

    let command: String = match key.get_value::<String, _>("") {
        Ok(command) if !command.trim().is_empty() => command,
        Ok(_) => return Ok(None),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(RebeccaError::ApplicationDiscoveryFailed(format!(
                "could not read Steam command from {key_path}: {err}"
            )));
        }
    };

    Ok(command_install_path_from_command(&command))
}

#[cfg(windows)]
fn command_install_path_from_command(command: &str) -> Option<PathBuf> {
    let executable = extract_command_executable(command)?;
    PathBuf::from(executable).parent().map(PathBuf::from)
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
        command_install_path_from_command, extract_command_executable, steam_installation_from_path,
    };

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

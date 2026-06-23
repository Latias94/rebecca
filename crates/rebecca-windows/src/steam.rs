use std::path::PathBuf;

use rebecca_core::applications::{ApplicationDiscovery, SteamInstallation};
use rebecca_core::error::{RebeccaError, Result};

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
    use std::io::ErrorKind;
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let steam_key = match hkcu.open_subkey("Software\\Valve\\Steam") {
        Ok(key) => key,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(RebeccaError::ApplicationDiscoveryFailed(format!(
                "could not open Steam registry key: {err}"
            )));
        }
    };

    let steam_path: String = match steam_key.get_value::<String, _>("SteamPath") {
        Ok(path) if !path.trim().is_empty() => path,
        Ok(_) => return Ok(None),
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(RebeccaError::ApplicationDiscoveryFailed(format!(
                "could not read SteamPath from registry: {err}"
            )));
        }
    };

    Ok(Some(steam_installation_from_path(steam_path)))
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

    use super::steam_installation_from_path;

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
}

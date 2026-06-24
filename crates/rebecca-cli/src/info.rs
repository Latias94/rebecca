use anyhow::Result;
use rebecca_core::applications::ApplicationDiscovery;
#[cfg(debug_assertions)]
use rebecca_core::applications::{
    NoopApplicationDiscovery, StaticApplicationDiscovery, SteamInstallation,
};
use rebecca_core::config::default_app_paths;
use rebecca_core::history::HistoryStore;

use crate::output::restore_hint_suffix;

pub fn print_history(json: bool) -> Result<()> {
    let paths = default_app_paths()?;
    let store = HistoryStore::new(paths.history_file);
    let entries = store.load()?;

    if json {
        println!("{}", serde_json::to_string_pretty(&entries)?);
        return Ok(());
    }

    if entries.is_empty() {
        println!("No cleanup history found.");
        return Ok(());
    }

    println!("Cleanup history: {} run(s)", entries.len());
    for entry in entries {
        println!(
            "- {}: {} completed, {} failed, {} pending bytes{}",
            entry.recorded_at_unix_seconds,
            entry.summary.completed_targets,
            entry.summary.failed_targets,
            entry.summary.pending_reclaim_bytes,
            restore_hint_suffix(
                entry
                    .targets
                    .iter()
                    .filter_map(|target| target.restore_hint.as_deref())
            )
        );
    }

    Ok(())
}

pub fn print_config_paths(json: bool) -> Result<()> {
    let paths = default_app_paths()?;

    if json {
        println!("{}", serde_json::to_string_pretty(&paths)?);
        return Ok(());
    }

    println!("Config file: {}", paths.config_file.display());
    println!("Config dir:  {}", paths.config_dir.display());
    println!("State dir:   {}", paths.state_dir.display());
    println!("Cache dir:   {}", paths.cache_dir.display());
    println!("History:     {}", paths.history_file.display());

    Ok(())
}

pub fn print_privilege_level() -> Result<()> {
    println!("Privilege level: {}", current_privilege_label());
    Ok(())
}

pub fn print_steam_discovery(discovery: &dyn ApplicationDiscovery) -> Result<()> {
    match discovery.steam_installation()? {
        Some(installation) => {
            println!("Steam install: {}", installation.install_path().display());
            if installation.library_paths().is_empty() {
                println!("Steam libraries: none discovered");
            } else {
                println!("Steam libraries:");
                for path in installation.library_paths() {
                    println!("- {}", path.display());
                }
            }
        }
        None => {
            println!("Steam install: not discovered");
        }
    }

    Ok(())
}

pub fn steam_application_discovery() -> Box<dyn ApplicationDiscovery> {
    if let Some(applications) = steam_application_discovery_override() {
        return applications;
    }

    #[cfg(windows)]
    {
        Box::new(rebecca_windows::steam::WindowsApplicationDiscovery::new())
    }

    #[cfg(not(windows))]
    {
        Box::new(rebecca_core::applications::NoopApplicationDiscovery::new())
    }
}

#[cfg(debug_assertions)]
fn steam_application_discovery_override() -> Option<Box<dyn ApplicationDiscovery>> {
    let discovery = std::env::var("REBECCA_STEAM_DISCOVERY").ok();
    if discovery.as_deref().is_some_and(|value| {
        value.eq_ignore_ascii_case("none") || value.eq_ignore_ascii_case("disabled")
    }) {
        return Some(Box::new(NoopApplicationDiscovery::new()));
    }

    let path = std::env::var("REBECCA_STEAM_DISCOVERY_PATH").ok()?;
    let path = path.trim();
    if path.is_empty() {
        return Some(Box::new(NoopApplicationDiscovery::new()));
    }

    Some(Box::new(
        StaticApplicationDiscovery::new()
            .with_steam_installation(steam_installation_from_debug_path(path)),
    ))
}

#[cfg(debug_assertions)]
fn steam_installation_from_debug_path(path: &str) -> SteamInstallation {
    #[cfg(windows)]
    {
        return rebecca_windows::steam::steam_installation_from_path(path);
    }

    #[cfg(not(windows))]
    {
        return SteamInstallation::from_install_path(path)
            .unwrap_or_else(|_| SteamInstallation::new(path, Vec::new()));
    }
}

#[cfg(not(debug_assertions))]
fn steam_application_discovery_override() -> Option<Box<dyn ApplicationDiscovery>> {
    None
}

#[cfg(windows)]
fn current_privilege_label() -> &'static str {
    match rebecca_windows::current_privilege_level() {
        rebecca_windows::PrivilegeLevel::StandardUser => "standard-user",
        rebecca_windows::PrivilegeLevel::Elevated => "elevated",
        rebecca_windows::PrivilegeLevel::Unknown => "unknown",
    }
}

#[cfg(not(windows))]
fn current_privilege_label() -> &'static str {
    "unsupported-platform"
}

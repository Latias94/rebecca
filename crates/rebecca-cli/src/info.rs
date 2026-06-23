use anyhow::Result;
use rebecca_core::applications::ApplicationDiscovery;
use rebecca_core::config::default_app_paths;
use rebecca_core::history::HistoryStore;

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
            "- {}: {} completed, {} failed, {} pending bytes",
            entry.recorded_at_unix_seconds,
            entry.summary.completed_targets,
            entry.summary.failed_targets,
            entry.summary.pending_reclaim_bytes
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

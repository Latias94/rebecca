use std::num::NonZeroUsize;

use anyhow::Result;
use rebecca_core::applications::ApplicationDiscovery;
#[cfg(debug_assertions)]
use rebecca_core::applications::{
    InstalledApplication, NoopApplicationDiscovery, StaticApplicationDiscovery, SteamInstallation,
};
use rebecca_core::config::{AppPaths, load_app_paths};
use rebecca_core::history::HistoryStore;
use rebecca_core::plan::{CleanupIssueSummary, CleanupTarget, CleanupTargetIssueReason};

use crate::history_view::{HistoryAggregateSummary, HistoryProjection, HistoryRunHighlight};
use crate::output::{format_bytes, format_issue_matrix_entry, restore_hint_suffix};

fn config_paths_json(paths: &AppPaths) -> serde_json::Value {
    serde_json::json!({
        "config_dir": &paths.config_dir,
        "config_file": &paths.config_file,
        "state_dir": &paths.state_dir,
        "cache_dir": &paths.cache_dir,
        "history_file": &paths.history_file,
        "storage": paths.storage_entries(),
    })
}

pub fn print_history(json: bool, limit: Option<NonZeroUsize>) -> Result<()> {
    let paths = load_app_paths()?;
    let store = HistoryStore::new(paths.history_file);
    let history = HistoryProjection::new(store.load()?, limit);

    if json {
        println!("{}", serde_json::to_string_pretty(history.entries())?);
        return Ok(());
    }

    if history.is_empty() {
        println!("No cleanup history found.");
        return Ok(());
    }

    println!("Cleanup history: {} run(s)", history.entries().len());
    print_history_summary(history.summary());
    print_largest_history_runs(history.largest_runs());

    for entry in history.entries() {
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
        print_history_issue_matrix(&entry.summary.issue_matrix);
        print_history_issue_targets(&entry.targets);
    }

    Ok(())
}

fn print_history_summary(summary: &HistoryAggregateSummary) {
    println!();
    println!("History summary:");
    println!("  Runs: {}", summary.runs);
    println!("  Completed targets: {}", summary.completed_targets);
    println!("  Skipped targets: {}", summary.skipped_targets);
    println!("  Blocked targets: {}", summary.blocked_targets);
    println!("  Failed targets: {}", summary.failed_targets);
    println!(
        "  Freed bytes: {} ({})",
        summary.freed_bytes,
        format_bytes(summary.freed_bytes)
    );
    println!(
        "  Pending reclaim bytes: {} ({})",
        summary.pending_reclaim_bytes,
        format_bytes(summary.pending_reclaim_bytes)
    );
    println!();
}

fn print_largest_history_runs(runs: &[HistoryRunHighlight]) {
    if runs.is_empty() {
        return;
    }

    println!("Largest cleanup runs:");
    for run in runs {
        println!(
            "  - {}: {} ({}) total, {} ({}) freed, {} ({}) pending reclaim",
            run.recorded_at_unix_seconds,
            run.total_bytes,
            format_bytes(run.total_bytes),
            run.freed_bytes,
            format_bytes(run.freed_bytes),
            run.pending_reclaim_bytes,
            format_bytes(run.pending_reclaim_bytes)
        );
    }
    println!();
}

fn print_history_issue_matrix(issue_matrix: &[CleanupIssueSummary]) {
    if issue_matrix.is_empty() {
        return;
    }

    println!("  Issue matrix:");
    for issue in issue_matrix {
        println!("  - {}", format_issue_matrix_entry(issue));
    }
}

fn print_history_issue_targets(targets: &[CleanupTarget]) {
    let issue_targets = targets
        .iter()
        .filter(|target| target.status.is_issue())
        .collect::<Vec<_>>();

    if issue_targets.is_empty() {
        return;
    }

    println!("  Issue targets:");
    for target in issue_targets {
        println!(
            "  - {} {}: {} [{}]{}{}",
            target.status.label(),
            target
                .reason_code
                .unwrap_or(CleanupTargetIssueReason::Unclassified)
                .label(),
            target.rule_id,
            target.path.display(),
            target
                .reason
                .as_deref()
                .map(|reason| format!(" ({reason})"))
                .unwrap_or_default(),
            restore_hint_suffix(target.restore_hint.as_deref())
        );
    }
}

pub fn print_config_paths(json: bool) -> Result<()> {
    let paths = load_app_paths()?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&config_paths_json(&paths))?
        );
        return Ok(());
    }

    for entry in paths.storage_entries() {
        println!(
            "{}: {} [{}; {}]",
            entry.id.label(),
            entry.path.display(),
            entry.lifecycle.label(),
            entry.retention.label()
        );
    }

    Ok(())
}

pub fn print_privilege_level() -> Result<()> {
    println!("Privilege level: {}", current_privilege_label());
    Ok(())
}

pub fn application_discovery() -> Box<dyn ApplicationDiscovery> {
    if let Some(applications) = application_discovery_override() {
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
fn application_discovery_override() -> Option<Box<dyn ApplicationDiscovery>> {
    if app_discovery_is_disabled() {
        return Some(Box::new(NoopApplicationDiscovery::new()));
    }

    let mut static_discovery = StaticApplicationDiscovery::new();
    let mut has_override = false;

    let steam_discovery = std::env::var("REBECCA_STEAM_DISCOVERY").ok();
    if steam_discovery.as_deref().is_some_and(|value| {
        value.eq_ignore_ascii_case("none") || value.eq_ignore_ascii_case("disabled")
    }) {
        has_override = true;
    } else if let Some(path) = std::env::var("REBECCA_STEAM_DISCOVERY_PATH").ok() {
        let path = path.trim();
        has_override = true;
        if !path.is_empty() {
            static_discovery = static_discovery
                .with_steam_installation(SteamInstallation::from_install_path_best_effort(path));
        }
    }

    let installed_applications = installed_applications_override();
    if !installed_applications.is_empty() {
        has_override = true;
        static_discovery = static_discovery.with_installed_applications(installed_applications);
    }

    has_override.then(|| Box::new(static_discovery) as Box<dyn ApplicationDiscovery>)
}

#[cfg(not(debug_assertions))]
fn application_discovery_override() -> Option<Box<dyn ApplicationDiscovery>> {
    None
}

#[cfg(debug_assertions)]
fn app_discovery_is_disabled() -> bool {
    std::env::var("REBECCA_APP_DISCOVERY")
        .ok()
        .is_some_and(|value| {
            value.eq_ignore_ascii_case("none") || value.eq_ignore_ascii_case("disabled")
        })
}

#[cfg(debug_assertions)]
fn installed_applications_override() -> Vec<InstalledApplication> {
    let Ok(raw) = std::env::var("REBECCA_INSTALLED_APPLICATIONS") else {
        return Vec::new();
    };

    raw.split(';')
        .enumerate()
        .filter_map(|(index, name)| {
            let name = name.trim();
            (!name.is_empty()).then(|| {
                InstalledApplication::new(
                    format!("debug/installed-application/{index}"),
                    name,
                    Vec::new(),
                )
            })
        })
        .collect()
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

use std::num::NonZeroUsize;

use anyhow::Result;
use rebecca::core::applications::ApplicationDiscovery;
#[cfg(debug_assertions)]
use rebecca::core::applications::{
    InstalledApplication, NoopApplicationDiscovery, StaticApplicationDiscovery, SteamInstallation,
};
use rebecca::core::config::{AppPaths, load_app_paths};
use rebecca::core::history::HistoryStore;
use rebecca::core::plan::{CleanupIssueSummary, CleanupTarget, CleanupTargetIssueReason};
use serde::Serialize;

use crate::cli::OutputMode;
use crate::history_view::{HistoryAggregateSummary, HistoryProjection, HistoryRunHighlight};
use crate::output::{format_bytes, format_issue_matrix_entry, restore_hint_suffix};

#[derive(Debug, Serialize)]
struct PermissionDiagnostic<'a> {
    platform: &'a str,
    platform_supported: bool,
    cleanup_execution_supported: bool,
    privilege_level: &'a str,
    suggested_action: &'a str,
}

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

pub fn print_history(output_mode: OutputMode, limit: Option<NonZeroUsize>) -> Result<()> {
    let paths = load_app_paths()?;
    let store = HistoryStore::new(paths.history_file);
    let history = HistoryProjection::new(store.load()?, limit);

    if output_mode.is_json() {
        return crate::output::print_success("history", "history-list", history.entries());
    }

    if output_mode.is_ndjson() {
        return crate::output::print_success_event("history", "history-list", history.entries());
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

pub fn print_config_paths(output_mode: OutputMode) -> Result<()> {
    let paths = load_app_paths()?;

    if output_mode.is_json() {
        let value = config_paths_json(&paths);
        return crate::output::print_success("config paths", "config-paths", &value);
    }

    if output_mode.is_ndjson() {
        let value = config_paths_json(&paths);
        return crate::output::print_success_event("config paths", "config-paths", &value);
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

pub fn print_privilege_level(output_mode: OutputMode) -> Result<()> {
    if output_mode.is_json() {
        let diagnostic = permission_diagnostic();
        return crate::output::print_success(
            "doctor permissions",
            "permissions-diagnostic",
            &diagnostic,
        );
    }

    if output_mode.is_ndjson() {
        let diagnostic = permission_diagnostic();
        return crate::output::print_success_event(
            "doctor permissions",
            "permissions-diagnostic",
            &diagnostic,
        );
    }

    println!("Privilege level: {}", current_privilege_label());
    Ok(())
}

fn permission_diagnostic() -> PermissionDiagnostic<'static> {
    let privilege_level = current_privilege_label();
    PermissionDiagnostic {
        platform: current_platform_label(),
        platform_supported: cfg!(windows),
        cleanup_execution_supported: cfg!(windows),
        privilege_level,
        suggested_action: permission_suggested_action(privilege_level),
    }
}

fn permission_suggested_action(privilege_level: &str) -> &'static str {
    match privilege_level {
        "elevated" => "cleanup execution can use elevated Windows permissions",
        "standard-user" => "run from an elevated shell if protected cleanup targets are blocked",
        "unsupported-platform" => {
            "cleanup execution is currently Windows-only; use preview commands on this platform"
        }
        _ => {
            "permission level could not be determined; use dry-run preview before executing cleanup"
        }
    }
}

fn current_platform_label() -> &'static str {
    if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        "unsupported"
    }
}

pub fn application_discovery() -> Box<dyn ApplicationDiscovery> {
    if let Some(applications) = application_discovery_override() {
        return applications;
    }

    #[cfg(windows)]
    {
        Box::new(rebecca::windows::steam::WindowsApplicationDiscovery::new())
    }

    #[cfg(not(windows))]
    {
        Box::new(rebecca::core::applications::NoopApplicationDiscovery::new())
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
    } else if let Ok(path) = std::env::var("REBECCA_STEAM_DISCOVERY_PATH") {
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
    match rebecca::windows::current_privilege_level() {
        rebecca::windows::PrivilegeLevel::StandardUser => "standard-user",
        rebecca::windows::PrivilegeLevel::Elevated => "elevated",
        rebecca::windows::PrivilegeLevel::Unknown => "unknown",
    }
}

#[cfg(not(windows))]
fn current_privilege_label() -> &'static str {
    "unsupported-platform"
}

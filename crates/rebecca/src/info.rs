use std::num::NonZeroUsize;
#[cfg(target_os = "linux")]
use std::path::Path;

use anyhow::Result;
use rebecca::core::RuleDefinition;
use rebecca::core::applications::ApplicationDiscovery;
#[cfg(debug_assertions)]
use rebecca::core::applications::{
    InstalledApplication, NoopApplicationDiscovery, StaticApplicationDiscovery, SteamInstallation,
};
use rebecca::core::config::{AppPaths, load_app_paths};
use rebecca::core::history::HistoryStore;
use rebecca::core::plan::{CleanupIssueSummary, CleanupTarget, CleanupTargetIssueReason};
use rebecca::core::warnings::ACTIVE_PROCESS_WARNING;
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

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ActiveProcessDiagnostic {
    platform: String,
    platform_supported: bool,
    process_inspection_available: bool,
    matches: Vec<ActiveProcessMatch>,
    unavailable_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct ActiveProcessMatch {
    process_id: u32,
    executable_name: String,
    warning: String,
    rule_ids: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ProcessSnapshot {
    pub(crate) process_id: u32,
    pub(crate) executable_name: String,
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
    let report = match limit {
        Some(limit) => store.load_tail_report(limit)?,
        None => store.load_report()?,
    };
    for diagnostic in &report.diagnostics {
        eprintln!(
            "History warning: skipped corrupted history line {} ({})",
            diagnostic.line_number, diagnostic.message
        );
    }
    let history = HistoryProjection::new(report.entries, limit);

    crate::output::print_command_success(
        "history",
        "history-list",
        output_mode,
        || history.entries(),
        || print_history_human(&history),
    )
}

fn print_history_human(history: &HistoryProjection) -> Result<()> {
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

    crate::output::print_command_success(
        "config paths",
        "config-paths",
        output_mode,
        || config_paths_json(&paths),
        || {
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
        },
    )
}

pub fn print_privilege_level(output_mode: OutputMode) -> Result<()> {
    crate::output::print_command_success(
        "doctor permissions",
        "permissions-diagnostic",
        output_mode,
        permission_diagnostic,
        || {
            println!("Privilege level: {}", current_privilege_label());
            Ok(())
        },
    )
}

pub fn print_active_processes(output_mode: OutputMode) -> Result<()> {
    crate::output::print_command_success(
        "doctor active-processes",
        "active-process-diagnostic",
        output_mode,
        active_process_diagnostic,
        || {
            let diagnostic = active_process_diagnostic();
            if !diagnostic.process_inspection_available {
                println!(
                    "Active process diagnostics unavailable: {}",
                    diagnostic
                        .unavailable_reason
                        .as_deref()
                        .unwrap_or("process inspection is unavailable")
                );
                return Ok(());
            }

            if diagnostic.matches.is_empty() {
                println!("Active process diagnostics: no warning-bearing cleanup rules matched.");
                return Ok(());
            }

            println!("Active process diagnostics:");
            for matched in &diagnostic.matches {
                println!(
                    "- {} (pid {}): {} [{}]",
                    matched.executable_name,
                    matched.process_id,
                    matched.warning,
                    matched.rule_ids.join(", ")
                );
            }
            Ok(())
        },
    )
}

fn active_process_diagnostic() -> ActiveProcessDiagnostic {
    let rules = match rebecca::rules::builtin_rules() {
        Ok(rules) => rules,
        Err(err) => {
            return ActiveProcessDiagnostic {
                platform: current_platform_label().to_string(),
                platform_supported: active_process_platform_supported(),
                process_inspection_available: false,
                matches: Vec::new(),
                unavailable_reason: Some(err.to_string()),
            };
        }
    };

    match active_process_snapshots() {
        Ok(processes) => active_process_diagnostic_from_processes(&rules, processes),
        Err(err) => ActiveProcessDiagnostic {
            platform: current_platform_label().to_string(),
            platform_supported: active_process_platform_supported(),
            process_inspection_available: false,
            matches: Vec::new(),
            unavailable_reason: Some(err.to_string()),
        },
    }
}

pub(crate) fn active_process_diagnostic_from_processes(
    rules: &[RuleDefinition],
    processes: Vec<ProcessSnapshot>,
) -> ActiveProcessDiagnostic {
    let active_rules = rules
        .iter()
        .filter(|rule| rule.platform == rebecca::core::Platform::current())
        .filter(|rule| {
            rule.warnings
                .iter()
                .any(|warning| warning.eq_ignore_ascii_case(ACTIVE_PROCESS_WARNING))
        })
        .collect::<Vec<_>>();
    let mut matches = Vec::new();

    for process in processes {
        let process_key = normalized_process_token(&process.executable_name);
        if process_key.is_empty() {
            continue;
        }

        let rule_ids = active_rules
            .iter()
            .filter(|rule| cleanup_rule_process_tokens(rule).contains(&process_key))
            .map(|rule| rule.id.clone())
            .collect::<Vec<_>>();
        if rule_ids.is_empty() {
            continue;
        }

        matches.push(ActiveProcessMatch {
            process_id: process.process_id,
            executable_name: process.executable_name,
            warning: ACTIVE_PROCESS_WARNING.to_string(),
            rule_ids,
        });
    }

    matches.sort_by(|left, right| {
        left.executable_name
            .to_ascii_lowercase()
            .cmp(&right.executable_name.to_ascii_lowercase())
            .then_with(|| left.process_id.cmp(&right.process_id))
    });

    ActiveProcessDiagnostic {
        platform: current_platform_label().to_string(),
        platform_supported: active_process_platform_supported(),
        process_inspection_available: true,
        matches,
        unavailable_reason: None,
    }
}

fn permission_diagnostic() -> PermissionDiagnostic<'static> {
    let privilege_level = current_privilege_label();
    PermissionDiagnostic {
        platform: current_platform_label(),
        platform_supported: cleanup_platform_supported(),
        cleanup_execution_supported: cleanup_platform_supported(),
        privilege_level,
        suggested_action: permission_suggested_action(privilege_level),
    }
}

fn permission_suggested_action(privilege_level: &str) -> &'static str {
    match privilege_level {
        "elevated" if cfg!(target_os = "linux") => {
            "cleanup execution can use elevated Linux permissions"
        }
        "elevated" if cfg!(target_os = "macos") => {
            "cleanup execution can use elevated macOS permissions"
        }
        "elevated" => "cleanup execution can use elevated Windows permissions",
        "standard-user" if cfg!(target_os = "linux") => {
            "use sudo only for reviewed permission-sensitive system cache rules"
        }
        "standard-user" if cfg!(target_os = "macos") => {
            "use standard-user preview and recoverable cleanup for user-owned macOS cache rules"
        }
        "standard-user" => "run from an elevated shell if protected cleanup targets are blocked",
        "unsupported-platform" => {
            "cleanup execution is not supported on this platform; use preview commands only"
        }
        _ => {
            "permission level could not be determined; use dry-run preview before executing cleanup"
        }
    }
}

fn cleanup_platform_supported() -> bool {
    cfg!(windows) || cfg!(target_os = "linux") || cfg!(target_os = "macos")
}

fn active_process_platform_supported() -> bool {
    cfg!(windows) || cfg!(target_os = "linux") || cfg!(target_os = "macos")
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

fn active_process_snapshots() -> Result<Vec<ProcessSnapshot>> {
    if let Some(processes) = active_process_snapshots_override() {
        return Ok(processes);
    }

    #[cfg(windows)]
    {
        rebecca::windows::process::active_processes()
            .map(|processes| {
                processes
                    .into_iter()
                    .map(|process| ProcessSnapshot {
                        process_id: process.process_id,
                        executable_name: process.executable_name,
                    })
                    .collect()
            })
            .map_err(Into::into)
    }

    #[cfg(target_os = "linux")]
    {
        linux_active_processes()
    }

    #[cfg(target_os = "macos")]
    {
        macos_active_processes()
    }

    #[cfg(all(not(windows), not(target_os = "linux"), not(target_os = "macos")))]
    {
        Err(rebecca::core::RebeccaError::PlatformUnavailable(
            "process diagnostics are not available on this platform".to_string(),
        )
        .into())
    }
}

#[cfg(target_os = "linux")]
fn linux_active_processes() -> Result<Vec<ProcessSnapshot>> {
    linux_active_processes_from_proc_root(Path::new("/proc"))
}

#[cfg(target_os = "linux")]
fn linux_active_processes_from_proc_root(proc_root: &Path) -> Result<Vec<ProcessSnapshot>> {
    let entries = std::fs::read_dir(proc_root).map_err(|err| {
        rebecca::core::RebeccaError::PlatformUnavailable(format!(
            "Linux /proc process diagnostics are unavailable: {err}"
        ))
    })?;

    let mut processes = Vec::new();
    for entry in entries {
        let Ok(entry) = entry else {
            continue;
        };
        let file_name = entry.file_name();
        let Some(pid_text) = file_name.to_str() else {
            continue;
        };
        let Ok(process_id) = pid_text.parse::<u32>() else {
            continue;
        };
        let process_dir = entry.path();
        let Some(executable_name) = linux_process_name(&process_dir) else {
            continue;
        };

        processes.push(ProcessSnapshot {
            process_id,
            executable_name,
        });
    }

    Ok(processes)
}

#[cfg(target_os = "linux")]
fn linux_process_name(process_dir: &Path) -> Option<String> {
    let comm = std::fs::read_to_string(process_dir.join("comm"))
        .ok()
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty());
    if comm.is_some() {
        return comm;
    }

    std::fs::read_link(process_dir.join("exe"))
        .ok()
        .and_then(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .filter(|name| !name.is_empty())
}

#[cfg(target_os = "macos")]
fn macos_active_processes() -> Result<Vec<ProcessSnapshot>> {
    let output = std::process::Command::new("ps")
        .args(["-axo", "pid=,comm="])
        .output()
        .map_err(|err| {
            rebecca::core::RebeccaError::PlatformUnavailable(format!(
                "macOS ps process diagnostics are unavailable: {err}"
            ))
        })?;

    if !output.status.success() {
        return Err(rebecca::core::RebeccaError::PlatformUnavailable(
            "macOS ps process diagnostics exited unsuccessfully".to_string(),
        )
        .into());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout
        .lines()
        .filter_map(macos_process_snapshot_from_ps_line)
        .collect())
}

#[cfg(target_os = "macos")]
fn macos_process_snapshot_from_ps_line(line: &str) -> Option<ProcessSnapshot> {
    let line = line.trim();
    let (pid, command) = line.split_once(char::is_whitespace)?;
    let process_id = pid.trim().parse::<u32>().ok()?;
    let executable_name = std::path::Path::new(command.trim())
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())?;

    Some(ProcessSnapshot {
        process_id,
        executable_name,
    })
}

#[cfg(debug_assertions)]
fn active_process_snapshots_override() -> Option<Vec<ProcessSnapshot>> {
    let raw = std::env::var("REBECCA_ACTIVE_PROCESSES").ok()?;
    Some(
        raw.split(';')
            .enumerate()
            .filter_map(|(index, process)| {
                let process = process.trim();
                if process.is_empty() {
                    return None;
                }

                let (name, pid) = process
                    .split_once(':')
                    .map(|(name, pid)| {
                        (
                            name.trim(),
                            pid.trim().parse::<u32>().unwrap_or(index as u32 + 1),
                        )
                    })
                    .unwrap_or((process, index as u32 + 1));

                (!name.is_empty()).then(|| ProcessSnapshot {
                    process_id: pid,
                    executable_name: name.to_string(),
                })
            })
            .collect(),
    )
}

#[cfg(not(debug_assertions))]
fn active_process_snapshots_override() -> Option<Vec<ProcessSnapshot>> {
    None
}

fn cleanup_rule_process_tokens(rule: &RuleDefinition) -> Vec<String> {
    let mut tokens = Vec::new();
    for token in explicit_cleanup_rule_process_tokens(&rule.id) {
        tokens.push(token.to_string());
    }

    for source in [&rule.id, &rule.name] {
        for token in source.split(|character: char| {
            !character.is_ascii_alphanumeric() || character == '_' || character == '-'
        }) {
            let token = normalized_process_token(token);
            if token.is_empty()
                || token == "windows"
                || token == "linux"
                || token == "macos"
                || token == "cache"
                || tokens.iter().any(|existing| existing == &token)
            {
                continue;
            }
            tokens.push(token);
        }
    }
    tokens
}

fn explicit_cleanup_rule_process_tokens(rule_id: &str) -> &'static [&'static str] {
    match rule_id {
        "linux.brave-cache" => &["brave", "brave-browser"],
        "linux.chrome-cache" => &["chrome", "google-chrome"],
        "linux.chromium-cache" => &["chromium"],
        "linux.edge-cache" => &["microsoft-edge", "msedge"],
        "linux.firefox-profile-cache" => &["firefox"],
        "linux.slack-cache" => &["slack"],
        "linux.thunderbird-cache" => &["thunderbird"],
        "linux.vlc-cache" => &["vlc"],
        "linux.zoom-logs" => &["zoom"],
        "linux.zen-browser-cache" => &["zen", "zen-browser", "zenbrowser"],
        "macos.brave-cache" => &["brave", "brave browser"],
        "macos.chrome-cache" => &["chrome", "google chrome"],
        "macos.chromium-cache" => &["chromium"],
        "macos.discord-cache" => &["discord", "discord ptb", "discord canary"],
        "macos.edge-cache" => &["microsoft edge", "msedge"],
        "macos.figma-cache" => &["figma"],
        "macos.firefox-profile-cache" => &["firefox"],
        "macos.notion-cache" => &["notion"],
        "macos.postman-cache" => &["postman"],
        "macos.slack-cache" => &["slack"],
        "macos.thunderbird-cache" => &["thunderbird"],
        "macos.vlc-cache" => &["vlc"],
        "macos.vscode-cache" => &["code", "visual studio code"],
        "macos.waterfox-cache" => &["waterfox"],
        "macos.zoom-logs" => &["zoom", "zoom.us"],
        "macos.zen-browser-cache" => &["zen", "zen browser", "zenbrowser"],
        "windows.adobe-reader-cache" => &["acrobat", "acroread"],
        "windows.teamviewer-logs" => &["teamviewer"],
        "windows.thunderbird-cache" => &["thunderbird"],
        "windows.vlc-cache" => &["vlc"],
        "windows.zoom-logs" => &["zoom"],
        "windows.zen-browser-cache" => &["zen", "zen-browser", "zenbrowser"],
        _ => &[],
    }
}

fn normalized_process_token(value: &str) -> String {
    let value = value.trim().to_ascii_lowercase();
    value.strip_suffix(".exe").unwrap_or(&value).to_string()
}

pub fn application_discovery() -> Box<dyn ApplicationDiscovery> {
    if let Some(applications) = application_discovery_override() {
        return applications;
    }

    #[cfg(windows)]
    {
        Box::new(rebecca::windows::steam::WindowsApplicationDiscovery::new())
    }

    #[cfg(target_os = "linux")]
    {
        Box::new(rebecca::core::applications::LinuxApplicationDiscovery::new())
    }

    #[cfg(target_os = "macos")]
    {
        Box::new(rebecca::core::applications::MacosApplicationDiscovery::new())
    }

    #[cfg(all(not(windows), not(target_os = "linux"), not(target_os = "macos")))]
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

#[cfg(target_os = "linux")]
fn current_privilege_label() -> &'static str {
    match linux_effective_uid() {
        Some(0) => "elevated",
        Some(_) => "standard-user",
        None => "unknown",
    }
}

#[cfg(target_os = "linux")]
fn linux_effective_uid() -> Option<u32> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status.lines().find_map(|line| {
        let values = line.strip_prefix("Uid:")?;
        values
            .split_whitespace()
            .nth(1)
            .and_then(|effective_uid| effective_uid.parse::<u32>().ok())
    })
}

#[cfg(target_os = "macos")]
fn current_privilege_label() -> &'static str {
    match macos_effective_uid() {
        Some(0) => "elevated",
        Some(_) => "standard-user",
        None => "unknown",
    }
}

#[cfg(target_os = "macos")]
fn macos_effective_uid() -> Option<u32> {
    let output = std::process::Command::new("id").arg("-u").output().ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<u32>()
        .ok()
}

#[cfg(all(not(windows), not(target_os = "linux"), not(target_os = "macos")))]
fn current_privilege_label() -> &'static str {
    "unsupported-platform"
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn linux_active_processes_reads_comm_names_from_proc_root() {
        let temp = tempfile::tempdir().unwrap();
        let process_dir = temp.path().join("4242");
        std::fs::create_dir_all(&process_dir).unwrap();
        std::fs::write(process_dir.join("comm"), "firefox\n").unwrap();
        std::fs::create_dir_all(temp.path().join("not-a-pid")).unwrap();

        let processes = linux_active_processes_from_proc_root(temp.path()).unwrap();

        assert_eq!(processes.len(), 1);
        assert_eq!(processes[0].process_id, 4242);
        assert_eq!(processes[0].executable_name, "firefox");
    }

    #[test]
    fn linux_active_processes_reports_unavailable_proc_root() {
        let temp = tempfile::tempdir().unwrap();
        let missing = temp.path().join("missing-proc");

        let error = linux_active_processes_from_proc_root(&missing)
            .unwrap_err()
            .to_string();

        assert!(error.contains("Linux /proc process diagnostics are unavailable"));
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn macos_process_snapshot_parses_ps_command_path() {
        let process = macos_process_snapshot_from_ps_line(
            " 4242 /Applications/Slack.app/Contents/MacOS/Slack",
        )
        .unwrap();

        assert_eq!(process.process_id, 4242);
        assert_eq!(process.executable_name, "Slack");
    }

    #[test]
    fn macos_process_snapshot_ignores_invalid_ps_rows() {
        assert!(macos_process_snapshot_from_ps_line("not-a-pid /Applications/Slack").is_none());
        assert!(macos_process_snapshot_from_ps_line("4242").is_none());
    }
}

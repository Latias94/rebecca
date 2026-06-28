use std::fs;

mod common;
use rebecca_core::history::HistoryEntry;
use rebecca_core::plan::{CleanupPlan, CleanupSummary, CleanupTarget, CleanupTargetIssueReason};
use rebecca_core::{DeleteMode, PlanRequest, Platform, TargetStatus};
#[path = "common/isolated.rs"]
mod isolated;

fn completed_history_entry(
    recorded_at_unix_seconds: u64,
    pending_reclaim_bytes: u64,
) -> HistoryEntry {
    let mut plan = CleanupPlan {
        request: PlanRequest::for_platform(Platform::Windows, DeleteMode::RecycleBin),
        summary: CleanupSummary {
            completed_targets: 1,
            failed_targets: 0,
            pending_reclaim_bytes,
            ..CleanupSummary::default()
        },
        targets: vec![CleanupTarget::allowed(
            "windows.user-temp",
            std::path::PathBuf::from(format!(r"C:\Temp\cache-{recorded_at_unix_seconds}.tmp")),
            pending_reclaim_bytes,
            DeleteMode::RecycleBin,
        )],
    };
    plan.targets[0].status = TargetStatus::Completed;
    plan.targets[0].pending_reclaim_bytes = pending_reclaim_bytes;

    let mut entry = HistoryEntry::from_plan(&plan);
    entry.recorded_at_unix_seconds = recorded_at_unix_seconds;
    entry
}

fn write_history_entries(history_path: &std::path::Path, entries: &[HistoryEntry]) {
    if let Some(parent) = history_path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }

    let mut lines = entries
        .iter()
        .map(|entry| serde_json::to_string(entry).unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    lines.push('\n');
    std::fs::write(history_path, lines).unwrap();
}

fn history_entry_with_summary(
    recorded_at_unix_seconds: u64,
    summary: CleanupSummary,
) -> HistoryEntry {
    HistoryEntry {
        recorded_at_unix_seconds,
        request: PlanRequest::for_platform(Platform::Windows, DeleteMode::RecycleBin),
        summary,
        targets: Vec::new(),
    }
}

fn protected_history_entry(recorded_at_unix_seconds: u64) -> HistoryEntry {
    let mut plan = CleanupPlan {
        request: PlanRequest::for_platform(Platform::Windows, DeleteMode::DryRun),
        summary: CleanupSummary::default(),
        targets: vec![CleanupTarget::blocked_with_reason_code(
            "windows.custom-browser-history",
            std::path::PathBuf::from(
                r"C:\Users\Alice\AppData\Local\Google\Chrome\User Data\Default\History",
            ),
            DeleteMode::DryRun,
            CleanupTargetIssueReason::SafetyPolicyBlocked,
            "browser private data is protected",
        )],
    };
    plan.recompute_summary();

    let mut entry = HistoryEntry::from_plan(&plan);
    entry.recorded_at_unix_seconds = recorded_at_unix_seconds;
    entry
}

fn missing_target_history_entry(recorded_at_unix_seconds: u64) -> HistoryEntry {
    let mut plan = CleanupPlan {
        request: PlanRequest::for_platform(Platform::Windows, DeleteMode::RecycleBin),
        summary: CleanupSummary::default(),
        targets: vec![CleanupTarget::skipped_with_reason_code(
            "windows.user-temp",
            std::path::PathBuf::from(r"C:\Users\Alice\AppData\Local\Temp\gone.tmp"),
            DeleteMode::RecycleBin,
            CleanupTargetIssueReason::ExecutionTargetMissing,
            "path does not exist",
        )],
    };
    plan.recompute_summary();

    let mut entry = HistoryEntry::from_plan(&plan);
    entry.recorded_at_unix_seconds = recorded_at_unix_seconds;
    entry
}

fn app_leftovers_history_entry(recorded_at_unix_seconds: u64) -> HistoryEntry {
    let mut plan = CleanupPlan {
        request: PlanRequest::for_platform(Platform::Windows, DeleteMode::DryRun)
            .with_workflow(rebecca_core::CleanupWorkflow::AppLeftovers),
        summary: CleanupSummary {
            completed_targets: 1,
            failed_targets: 0,
            pending_reclaim_bytes: 12,
            ..CleanupSummary::default()
        },
        targets: vec![
            rebecca_core::CleanupTarget::allowed(
                "windows.app-leftover-local-cache",
                std::path::PathBuf::from(r"C:\Users\Alice\AppData\Local\WeChat\Cache"),
                12,
                DeleteMode::DryRun,
            )
            .with_restore_hint(Some(
                "App leftovers will be rebuilt when the app runs again.".to_string(),
            )),
        ],
    };
    plan.targets[0].status = TargetStatus::Completed;
    plan.targets[0].pending_reclaim_bytes = 12;

    let mut entry = HistoryEntry::from_plan(&plan);
    entry.recorded_at_unix_seconds = recorded_at_unix_seconds;
    entry
}

#[test]
fn history_json_is_empty_when_no_history_file_exists() {
    let temp = tempfile::tempdir().unwrap();
    let output = isolated::isolated_rebecca(&temp)
        .args(["history", "--format", "json"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );
    let value: serde_json::Value = common::support::api_data(&output.stdout);

    assert_eq!(value.as_array().unwrap().len(), 0);
}

#[test]
fn history_human_output_is_empty_when_no_history_file_exists() {
    let temp = tempfile::tempdir().unwrap();
    let output = isolated::isolated_rebecca(&temp)
        .args(["history"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("No cleanup history found."));
}

#[test]
fn history_reports_corrupted_history_file_with_line_number() {
    let temp = tempfile::tempdir().unwrap();
    let history_path = temp.path().join("rebecca-state").join("history.jsonl");
    if let Some(parent) = history_path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&history_path, "{not json}\n").unwrap();

    let output = isolated::isolated_rebecca(&temp)
        .args(["history"])
        .output()
        .unwrap();

    assert!(!output.status.success());

    let stderr = common::support::stderr(&output);
    assert!(stderr.contains("history record was corrupted"));
    assert!(stderr.contains("line 1"));
    assert!(stderr.contains("history.jsonl"));
}

#[test]
fn history_json_preserves_restore_hints() {
    let temp = tempfile::tempdir().unwrap();
    let history_path = temp.path().join("rebecca-state").join("history.jsonl");
    if let Some(parent) = history_path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }

    let mut plan = CleanupPlan {
        request: PlanRequest::for_platform(Platform::Windows, DeleteMode::RecycleBin),
        summary: CleanupSummary {
            completed_targets: 1,
            failed_targets: 0,
            pending_reclaim_bytes: 11,
            ..CleanupSummary::default()
        },
        targets: vec![
            CleanupTarget::allowed(
                "windows.user-temp",
                std::path::PathBuf::from(r"C:\Temp\cache.tmp"),
                11,
                DeleteMode::RecycleBin,
            )
            .with_restore_hint(Some("Temporary files can be recreated.".to_string())),
        ],
    };
    plan.targets[0].status = TargetStatus::Completed;
    plan.targets[0].pending_reclaim_bytes = 11;

    let entry = HistoryEntry::from_plan(&plan);
    std::fs::write(&history_path, serde_json::to_string(&entry).unwrap() + "\n").unwrap();

    let output = isolated::isolated_rebecca(&temp)
        .args(["history", "--format", "json"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let value: serde_json::Value = common::support::api_data(&output.stdout);
    let entries = value.as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0]["targets"][0]["restore_hint"].as_str().unwrap(),
        "Temporary files can be recreated."
    );
}

#[test]
fn history_json_preserves_execution_missing_issue_details() {
    let temp = tempfile::tempdir().unwrap();
    let history_path = temp.path().join("rebecca-state").join("history.jsonl");
    write_history_entries(&history_path, &[missing_target_history_entry(100)]);

    let output = isolated::isolated_rebecca(&temp)
        .args(["history", "--format", "json"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let value: serde_json::Value = common::support::api_data(&output.stdout);
    let entry = &value.as_array().unwrap()[0];

    assert_eq!(entry["summary"]["skipped_targets"], 1);
    assert!(
        entry["summary"]["issue_matrix"]
            .as_array()
            .unwrap()
            .iter()
            .any(|issue| issue["status"] == "skipped"
                && issue["reason_code"] == "execution-target-missing"
                && issue["targets"] == 1)
    );
    assert_eq!(entry["targets"][0]["status"], "skipped");
    assert_eq!(
        entry["targets"][0]["reason_code"],
        "execution-target-missing"
    );
    assert_eq!(entry["targets"][0]["reason"], "path does not exist");
}

#[test]
fn history_json_preserves_app_leftovers_workflow() {
    let temp = tempfile::tempdir().unwrap();
    let history_path = temp.path().join("rebecca-state").join("history.jsonl");
    write_history_entries(&history_path, &[app_leftovers_history_entry(42)]);

    let output = isolated::isolated_rebecca(&temp)
        .args(["history", "--format", "json"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let value: serde_json::Value = common::support::api_data(&output.stdout);
    let entry = &value.as_array().unwrap()[0];

    assert_eq!(entry["request"]["workflow"], "app-leftovers");
    assert_eq!(entry["summary"]["completed_targets"], 1);
    assert_eq!(
        entry["targets"][0]["rule_id"],
        "windows.app-leftover-local-cache"
    );
    assert!(
        entry["targets"][0]["path"]
            .as_str()
            .unwrap()
            .contains("WeChat\\Cache")
    );
}

#[test]
fn history_json_preserves_protected_issue_details() {
    let temp = tempfile::tempdir().unwrap();
    let history_path = temp.path().join("rebecca-state").join("history.jsonl");
    write_history_entries(&history_path, &[protected_history_entry(99)]);

    let output = isolated::isolated_rebecca(&temp)
        .args(["history", "--format", "json"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let value: serde_json::Value = common::support::api_data(&output.stdout);
    let entry = &value.as_array().unwrap()[0];

    assert_eq!(entry["summary"]["blocked_targets"], 1);
    assert!(
        entry["summary"]["issue_matrix"]
            .as_array()
            .unwrap()
            .iter()
            .any(|issue| issue["status"] == "blocked"
                && issue["reason_code"] == "safety-policy-blocked"
                && issue["targets"] == 1)
    );
    assert_eq!(entry["targets"][0]["status"], "blocked");
    assert_eq!(entry["targets"][0]["reason_code"], "safety-policy-blocked");
    assert_eq!(
        entry["targets"][0]["reason"],
        "browser private data is protected"
    );
    assert!(entry["targets"][0].get("children").is_none());
    assert!(entry["targets"][0].get("contents").is_none());
}

#[test]
fn history_human_output_lists_restore_hints() {
    let temp = tempfile::tempdir().unwrap();
    let history_path = temp.path().join("rebecca-state").join("history.jsonl");
    if let Some(parent) = history_path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }

    let mut plan = CleanupPlan {
        request: PlanRequest::for_platform(Platform::Windows, DeleteMode::RecycleBin),
        summary: CleanupSummary {
            completed_targets: 2,
            failed_targets: 0,
            pending_reclaim_bytes: 42,
            ..CleanupSummary::default()
        },
        targets: vec![
            CleanupTarget::allowed(
                "windows.user-temp",
                std::path::PathBuf::from(r"C:\Temp\cache.tmp"),
                11,
                DeleteMode::RecycleBin,
            )
            .with_restore_hint(Some("Temporary files can be recreated.".to_string())),
            CleanupTarget::allowed(
                "windows.steam-cache",
                std::path::PathBuf::from(r"C:\Steam\htmlcache\Default\Cache"),
                31,
                DeleteMode::RecycleBin,
            )
            .with_restore_hint(Some(
                "Steam web caches will be rebuilt on launch.".to_string(),
            )),
        ],
    };
    plan.targets[0].status = TargetStatus::Completed;
    plan.targets[0].pending_reclaim_bytes = 11;
    plan.targets[1].status = TargetStatus::Completed;
    plan.targets[1].pending_reclaim_bytes = 31;

    let entry = HistoryEntry::from_plan(&plan);
    std::fs::write(&history_path, serde_json::to_string(&entry).unwrap() + "\n").unwrap();

    let output = isolated::isolated_rebecca(&temp)
        .args(["history"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Cleanup history: 1 run(s)"));
    assert!(stdout.contains("- "));
    assert!(stdout.contains(
        "[restore: Temporary files can be recreated.; Steam web caches will be rebuilt on launch.]"
    ));
}

#[test]
fn history_human_output_lists_saved_issue_matrix() {
    let temp = tempfile::tempdir().unwrap();
    let history_path = temp.path().join("rebecca-state").join("history.jsonl");
    if let Some(parent) = history_path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }

    let mut plan = CleanupPlan {
        request: PlanRequest::for_platform(Platform::Windows, DeleteMode::DryRun),
        summary: CleanupSummary::default(),
        targets: vec![CleanupTarget::skipped_with_reason_code(
            "windows.user-temp",
            std::path::PathBuf::from(r"C:\Temp\cache.tmp"),
            DeleteMode::DryRun,
            CleanupTargetIssueReason::DuplicateTargetPath,
            "duplicate target path",
        )],
    };
    plan.recompute_summary();

    let entry = HistoryEntry::from_plan(&plan);
    std::fs::write(&history_path, serde_json::to_string(&entry).unwrap() + "\n").unwrap();

    let output = isolated::isolated_rebecca(&temp)
        .args(["history"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Cleanup history: 1 run(s)"));
    assert!(stdout.contains("Issue matrix:"));
    assert!(stdout.contains("- skipped duplicate-target-path: 1 target, 0 (0 B)"));
}

#[test]
fn history_human_output_lists_protected_issue_targets() {
    let temp = tempfile::tempdir().unwrap();
    let history_path = temp.path().join("rebecca-state").join("history.jsonl");
    write_history_entries(&history_path, &[protected_history_entry(99)]);

    let output = isolated::isolated_rebecca(&temp)
        .args(["history"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Cleanup history: 1 run(s)"));
    assert!(stdout.contains("Issue matrix:"));
    assert!(stdout.contains("- blocked safety-policy-blocked: 1 target, 0 (0 B)"));
    assert!(stdout.contains("Issue targets:"));
    assert!(stdout.contains("blocked safety-policy-blocked: windows.custom-browser-history"));
    assert!(stdout.contains("Default\\History"));
    assert!(stdout.contains("browser private data is protected"));
}

#[test]
fn history_human_output_lists_execution_missing_issue_targets() {
    let temp = tempfile::tempdir().unwrap();
    let history_path = temp.path().join("rebecca-state").join("history.jsonl");
    write_history_entries(&history_path, &[missing_target_history_entry(100)]);

    let output = isolated::isolated_rebecca(&temp)
        .args(["history"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Issue matrix:"));
    assert!(stdout.contains("- skipped execution-target-missing: 1 target, 0 (0 B)"));
    assert!(stdout.contains("skipped execution-target-missing: windows.user-temp"));
    assert!(stdout.contains("gone.tmp"));
    assert!(stdout.contains("path does not exist"));
}

#[test]
fn history_human_output_includes_aggregate_summary() {
    let temp = tempfile::tempdir().unwrap();
    let history_path = temp.path().join("rebecca-state").join("history.jsonl");
    write_history_entries(
        &history_path,
        &[
            history_entry_with_summary(
                10,
                CleanupSummary {
                    completed_targets: 1,
                    skipped_targets: 1,
                    blocked_targets: 0,
                    failed_targets: 0,
                    freed_bytes: 1024,
                    pending_reclaim_bytes: 512,
                    ..CleanupSummary::default()
                },
            ),
            history_entry_with_summary(
                20,
                CleanupSummary {
                    completed_targets: 2,
                    skipped_targets: 0,
                    blocked_targets: 1,
                    failed_targets: 1,
                    freed_bytes: 2048,
                    pending_reclaim_bytes: 1024,
                    ..CleanupSummary::default()
                },
            ),
        ],
    );

    let output = isolated::isolated_rebecca(&temp)
        .args(["history"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("History summary:"));
    assert!(stdout.contains("Runs: 2"));
    assert!(stdout.contains("Completed targets: 3"));
    assert!(stdout.contains("Skipped targets: 1"));
    assert!(stdout.contains("Blocked targets: 1"));
    assert!(stdout.contains("Failed targets: 1"));
    assert!(stdout.contains("Freed bytes: 3072 (3.00 KiB)"));
    assert!(stdout.contains("Pending reclaim bytes: 1536 (1.50 KiB)"));
}

#[test]
fn history_human_output_highlights_largest_cleanup_runs() {
    let temp = tempfile::tempdir().unwrap();
    let history_path = temp.path().join("rebecca-state").join("history.jsonl");
    write_history_entries(
        &history_path,
        &[
            history_entry_with_summary(
                10,
                CleanupSummary {
                    freed_bytes: 100,
                    pending_reclaim_bytes: 0,
                    ..CleanupSummary::default()
                },
            ),
            history_entry_with_summary(
                20,
                CleanupSummary {
                    freed_bytes: 0,
                    pending_reclaim_bytes: 400,
                    ..CleanupSummary::default()
                },
            ),
            history_entry_with_summary(
                30,
                CleanupSummary {
                    freed_bytes: 200,
                    pending_reclaim_bytes: 100,
                    ..CleanupSummary::default()
                },
            ),
            history_entry_with_summary(
                40,
                CleanupSummary {
                    freed_bytes: 0,
                    pending_reclaim_bytes: 200,
                    ..CleanupSummary::default()
                },
            ),
            history_entry_with_summary(50, CleanupSummary::default()),
        ],
    );

    let output = isolated::isolated_rebecca(&temp)
        .args(["history"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Largest cleanup runs:"));
    assert!(
        stdout.contains("  - 20: 400 (400 B) total, 0 (0 B) freed, 400 (400 B) pending reclaim")
    );
    assert!(
        stdout
            .contains("  - 30: 300 (300 B) total, 200 (200 B) freed, 100 (100 B) pending reclaim")
    );
    assert!(
        stdout.contains("  - 40: 200 (200 B) total, 0 (0 B) freed, 200 (200 B) pending reclaim")
    );
    assert!(!stdout.contains("  - 10:"));
    assert!(!stdout.contains("  - 50:"));
    assert!(stdout.find("  - 20:").unwrap() < stdout.find("  - 30:").unwrap());
    assert!(stdout.find("  - 30:").unwrap() < stdout.find("  - 40:").unwrap());
}

#[test]
fn history_human_output_omits_largest_runs_when_cleanup_bytes_are_zero() {
    let temp = tempfile::tempdir().unwrap();
    let history_path = temp.path().join("rebecca-state").join("history.jsonl");
    write_history_entries(
        &history_path,
        &[
            history_entry_with_summary(10, CleanupSummary::default()),
            history_entry_with_summary(20, CleanupSummary::default()),
        ],
    );

    let output = isolated::isolated_rebecca(&temp)
        .args(["history"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("History summary:"));
    assert!(!stdout.contains("Largest cleanup runs:"));
}

#[test]
fn history_human_limit_shows_most_recent_entries_in_chronological_order() {
    let temp = tempfile::tempdir().unwrap();
    let history_path = temp.path().join("rebecca-state").join("history.jsonl");
    write_history_entries(
        &history_path,
        &[
            completed_history_entry(10, 10),
            completed_history_entry(20, 20),
            completed_history_entry(30, 30),
        ],
    );

    let output = isolated::isolated_rebecca(&temp)
        .args(["history", "--limit", "2"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Cleanup history: 2 run(s)"));
    assert!(stdout.contains("Runs: 2"));
    assert!(stdout.contains("Completed targets: 2"));
    assert!(stdout.contains("Pending reclaim bytes: 50 (50 B)"));
    assert!(stdout.contains("Largest cleanup runs:"));
    assert!(stdout.contains("  - 30:"));
    assert!(stdout.contains("  - 20:"));
    assert!(!stdout.contains("  - 10:"));
    assert!(!stdout.contains("- 10:"));
    assert!(stdout.contains("- 20:"));
    assert!(stdout.contains("- 30:"));
    assert!(stdout.find("\n- 20:").unwrap() < stdout.find("\n- 30:").unwrap());
}

#[test]
fn history_json_limit_shows_most_recent_entries() {
    let temp = tempfile::tempdir().unwrap();
    let history_path = temp.path().join("rebecca-state").join("history.jsonl");
    write_history_entries(
        &history_path,
        &[
            completed_history_entry(10, 10),
            completed_history_entry(20, 20),
            completed_history_entry(30, 30),
        ],
    );

    let output = isolated::isolated_rebecca(&temp)
        .args(["history", "--format", "json", "--limit", "2"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let value: serde_json::Value = common::support::api_data(&output.stdout);
    let entries = value.as_array().unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0]["recorded_at_unix_seconds"].as_u64().unwrap(), 20);
    assert_eq!(entries[1]["recorded_at_unix_seconds"].as_u64().unwrap(), 30);
}

#[test]
fn history_limit_rejects_zero() {
    let temp = tempfile::tempdir().unwrap();
    let output = isolated::isolated_rebecca(&temp)
        .args(["history", "--limit", "0"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = common::support::stderr(&output);
    assert!(stderr.contains("--limit"));
    assert!(stderr.contains('0'));
}

mod common;
use rebecca_core::history::HistoryEntry;
use rebecca_core::plan::{CleanupPlan, CleanupSummary, CleanupTarget};
use rebecca_core::{DeleteMode, PlanRequest, Platform, TargetStatus};
#[path = "common/isolated.rs"]
mod isolated;

#[test]
fn history_json_is_empty_when_no_history_file_exists() {
    let temp = tempfile::tempdir().unwrap();
    let output = isolated::isolated_rebecca(&temp)
        .args(["history", "--json"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    assert_eq!(value.as_array().unwrap().len(), 0);
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

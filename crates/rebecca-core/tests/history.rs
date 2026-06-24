use std::fs;
use std::path::PathBuf;

use rebecca_core::history::HistoryStore;
use rebecca_core::plan::{CleanupPlan, CleanupTarget};
use rebecca_core::{DeleteMode, PlanRequest, Platform};

#[test]
fn append_and_load_history_entries_in_order() {
    let temp = tempfile::tempdir().unwrap();
    let store = HistoryStore::new(temp.path().join("history.jsonl"));
    let plan = sample_plan();

    store.append_plan(&plan).unwrap();
    store.append_plan(&plan).unwrap();

    let entries = store.load().unwrap();

    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].summary.completed_targets, 1);
    assert_eq!(entries[1].summary.completed_targets, 1);
    assert_eq!(
        entries[0].targets[0].restore_hint.as_deref(),
        Some("Temporary files can be recreated.")
    );
}

#[test]
fn append_and_load_history_entries_preserve_protected_issue_details() {
    let temp = tempfile::tempdir().unwrap();
    let store = HistoryStore::new(temp.path().join("history.jsonl"));
    let mut plan = CleanupPlan::empty(PlanRequest::for_platform(
        Platform::Windows,
        DeleteMode::RecycleBin,
    ));
    plan.targets.push(CleanupTarget::blocked_with_reason_code(
        "windows.custom-browser-history",
        PathBuf::from("C:/Users/Alice/AppData/Local/Google/Chrome/User Data/Default/History"),
        DeleteMode::RecycleBin,
        rebecca_core::CleanupTargetIssueReason::SafetyPolicyBlocked,
        "browser private data is protected",
    ));
    plan.recompute_summary();

    store.append_plan(&plan).unwrap();

    let entries = store.load().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].summary.blocked_targets, 1);
    assert_eq!(
        entries[0].summary.issue_matrix[0].reason_code,
        rebecca_core::CleanupTargetIssueReason::SafetyPolicyBlocked
    );
    assert_eq!(
        entries[0].targets[0].reason_code,
        Some(rebecca_core::CleanupTargetIssueReason::SafetyPolicyBlocked)
    );
    assert_eq!(
        entries[0].targets[0].reason.as_deref(),
        Some("browser private data is protected")
    );
}

#[test]
fn missing_history_file_loads_as_empty() {
    let temp = tempfile::tempdir().unwrap();
    let store = HistoryStore::new(temp.path().join("missing.jsonl"));

    assert!(store.load().unwrap().is_empty());
}

#[test]
fn malformed_history_line_is_reported() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("history.jsonl");
    fs::write(&path, "{not json}\n").unwrap();

    let store = HistoryStore::new(path);
    let err = store.load().unwrap_err();

    assert!(err.to_string().contains("history record was corrupted"));
}

#[test]
fn history_error_mentions_bad_line_number() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("history.jsonl");
    fs::write(&path, "{not json}\n{}\n").unwrap();

    let store = HistoryStore::new(path);
    let err = store.load().unwrap_err();

    let message = err.to_string();
    assert!(message.contains("history record was corrupted"));
    assert!(message.contains("line 1"));
}

fn sample_plan() -> CleanupPlan {
    let mut plan = CleanupPlan::empty(PlanRequest::for_platform(
        Platform::Windows,
        DeleteMode::RecycleBin,
    ));
    let mut target = CleanupTarget::allowed(
        "windows.user-temp",
        PathBuf::from("C:/Temp/file.tmp"),
        10,
        DeleteMode::RecycleBin,
    )
    .with_restore_hint(Some("Temporary files can be recreated.".to_string()));
    target.status = rebecca_core::TargetStatus::Completed;
    target.pending_reclaim_bytes = 10;
    plan.targets.push(target);
    plan.recompute_summary();
    plan
}

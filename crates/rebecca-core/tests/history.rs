use std::fs;
use std::num::NonZeroUsize;
use std::path::PathBuf;

use rebecca_core::history::{HistoryEntry, HistoryStore};
use rebecca_core::plan::{CleanupPlan, CleanupTarget, CleanupTargetIssueReason};
use rebecca_core::{CleanupWorkflow, DeleteMode, PlanRequest, Platform};

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
fn append_and_load_history_entries_preserve_execution_missing_issue_details() {
    let temp = tempfile::tempdir().unwrap();
    let store = HistoryStore::new(temp.path().join("history.jsonl"));
    let mut plan = CleanupPlan::empty(PlanRequest::for_platform(
        Platform::Windows,
        DeleteMode::RecycleBin,
    ));
    plan.targets.push(CleanupTarget::skipped_with_reason_code(
        "windows.user-temp",
        PathBuf::from("C:/Users/Alice/AppData/Local/Temp/gone.tmp"),
        DeleteMode::RecycleBin,
        CleanupTargetIssueReason::ExecutionTargetMissing,
        "path does not exist",
    ));
    plan.recompute_summary();

    store.append_plan(&plan).unwrap();

    let entries = store.load().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].summary.skipped_targets, 1);
    assert_eq!(
        entries[0].summary.issue_matrix[0].reason_code,
        CleanupTargetIssueReason::ExecutionTargetMissing
    );
    assert_eq!(
        entries[0].targets[0].reason_code,
        Some(CleanupTargetIssueReason::ExecutionTargetMissing)
    );
    assert_eq!(
        entries[0].targets[0].reason.as_deref(),
        Some("path does not exist")
    );
}

#[test]
fn append_and_load_history_entries_preserve_app_leftovers_workflow() {
    let temp = tempfile::tempdir().unwrap();
    let store = HistoryStore::new(temp.path().join("history.jsonl"));
    let mut plan = CleanupPlan::empty(
        PlanRequest::for_platform(Platform::Windows, DeleteMode::DryRun)
            .with_workflow(CleanupWorkflow::AppLeftovers),
    );
    plan.targets.push(CleanupTarget::allowed(
        "windows.app-leftover-local-cache",
        PathBuf::from("C:/Users/Alice/AppData/Local/Example App/Cache"),
        10,
        DeleteMode::DryRun,
    ));
    plan.recompute_summary();

    store.append_plan(&plan).unwrap();

    let entries = store.load().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].request.workflow, CleanupWorkflow::AppLeftovers);
    assert_eq!(
        entries[0].targets[0].rule_id,
        "windows.app-leftover-local-cache"
    );
}

#[test]
fn missing_history_file_loads_as_empty() {
    let temp = tempfile::tempdir().unwrap();
    let store = HistoryStore::new(temp.path().join("missing.jsonl"));

    assert!(store.load().unwrap().is_empty());
    assert!(
        store
            .load_tail(NonZeroUsize::new(2).unwrap())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn malformed_history_line_is_skipped_with_diagnostic() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("history.jsonl");
    fs::write(&path, "{not json}\n").unwrap();

    let store = HistoryStore::new(path);
    let report = store.load_report().unwrap();

    assert!(report.entries.is_empty());
    assert_eq!(report.diagnostics.len(), 1);
    assert!(
        report.diagnostics[0]
            .message
            .contains("history record was corrupted")
    );
}

#[test]
fn history_diagnostic_mentions_bad_line_number() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("history.jsonl");
    fs::write(&path, "{not json}\n{}\n").unwrap();

    let store = HistoryStore::new(path);
    let report = store.load_report().unwrap();

    assert!(report.entries.is_empty());
    assert_eq!(report.diagnostics.len(), 2);
    let message = &report.diagnostics[0].message;
    assert!(message.contains("history record was corrupted"));
    assert!(message.contains("line 1"));
}

#[test]
fn append_plan_report_converts_write_failure_to_warning() {
    let temp = tempfile::tempdir().unwrap();
    let history_path = temp.path().join("history.jsonl");
    fs::create_dir_all(&history_path).unwrap();
    let store = HistoryStore::new(history_path);

    let report = store.append_plan_report(&sample_plan());

    assert!(!report.written);
    let warning = report.warning.expect("expected history warning");
    assert_eq!(
        warning.kind,
        rebecca_core::ExecutionWarningKind::HistoryWriteFailed
    );
    assert!(warning.message.contains("cleanup history was not written"));
}

#[test]
fn load_tail_returns_newest_entries_in_chronological_order() {
    let temp = tempfile::tempdir().unwrap();
    let store = HistoryStore::new(temp.path().join("history.jsonl"));
    let mut first = HistoryEntry::from_plan(&sample_plan());
    first.recorded_at_unix_seconds = 10;
    let mut second = first.clone();
    second.recorded_at_unix_seconds = 20;
    let mut third = first.clone();
    third.recorded_at_unix_seconds = 30;
    store.append_entry(&first).unwrap();
    store.append_entry(&second).unwrap();
    store.append_entry(&third).unwrap();

    let entries = store.load_tail(NonZeroUsize::new(2).unwrap()).unwrap();

    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.recorded_at_unix_seconds)
            .collect::<Vec<_>>(),
        vec![20, 30]
    );
}

#[test]
fn load_tail_reports_original_line_number_for_tail_corruption() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("history.jsonl");
    fs::write(&path, "{}\n{not json}\n").unwrap();

    let store = HistoryStore::new(path);
    let report = store
        .load_tail_report(NonZeroUsize::new(1).unwrap())
        .unwrap();

    assert!(report.entries.is_empty());
    assert_eq!(report.diagnostics.len(), 1);
    let message = &report.diagnostics[0].message;
    assert!(message.contains("history record was corrupted"));
    assert!(message.contains("line 2"));
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

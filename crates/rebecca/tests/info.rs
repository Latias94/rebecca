mod common;
#[path = "common/isolated.rs"]
mod isolated;

#[test]
fn config_paths_json_is_parseable() {
    let temp = tempfile::tempdir().unwrap();
    let output = isolated::isolated_rebecca(&temp)
        .args(["config", "paths", "--format", "json"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );
    let value: serde_json::Value = common::support::api_data(&output.stdout);

    assert!(
        value["config_file"]
            .as_str()
            .unwrap()
            .contains("rebecca-config")
    );
    assert!(
        value["history_file"]
            .as_str()
            .unwrap()
            .contains("history.jsonl")
    );
    let storage = value["storage"].as_array().unwrap();
    assert_eq!(storage.len(), 5);
    assert_eq!(
        storage
            .iter()
            .find(|entry| entry["id"].as_str().unwrap() == "cache-dir")
            .unwrap()["retention"]
            .as_str()
            .unwrap(),
        "rebuildable"
    );
    assert_eq!(
        storage
            .iter()
            .find(|entry| entry["id"].as_str().unwrap() == "history-file")
            .unwrap()["lifecycle"]
            .as_str()
            .unwrap(),
        "append-only-history"
    );
}

#[test]
fn doctor_permissions_prints_permission_label() {
    let output = common::command::rebecca()
        .args(["doctor", "permissions"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Privilege level:"));
}

#[test]
fn doctor_active_processes_json_reports_fake_matching_process() {
    let output = common::command::rebecca()
        .env("REBECCA_ACTIVE_PROCESSES", "slack.exe:4242;unrelated.exe:9")
        .args(["doctor", "active-processes", "--format", "json"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let envelope = common::support::api_envelope(&output.stdout);
    assert_eq!(envelope["command"], "doctor active-processes");
    assert_eq!(envelope["payload_kind"], "active-process-diagnostic");
    let data = &envelope["data"];
    assert_eq!(data["process_inspection_available"], true);
    assert_eq!(data["matches"][0]["process_id"], 4242);
    assert_eq!(data["matches"][0]["executable_name"], "slack.exe");
    assert_eq!(data["matches"][0]["warning"], "active-process");
    assert_eq!(data["matches"][0]["rule_ids"][0], "windows.slack-cache");
}

#[test]
fn doctor_active_processes_json_matches_new_diagnostic_rules() {
    let output = common::command::rebecca()
        .env(
            "REBECCA_ACTIVE_PROCESSES",
            "Zoom.exe:10;TeamViewer.exe:11;vlc.exe:12",
        )
        .args(["doctor", "active-processes", "--format", "json"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let data = common::support::api_data(&output.stdout);
    let matches = data["matches"].as_array().unwrap();
    let rule_ids = matches
        .iter()
        .flat_map(|matched| matched["rule_ids"].as_array().unwrap())
        .map(|rule_id| rule_id.as_str().unwrap())
        .collect::<std::collections::BTreeSet<_>>();

    assert!(rule_ids.contains("windows.zoom-logs"));
    assert!(rule_ids.contains("windows.teamviewer-logs"));
    assert!(rule_ids.contains("windows.vlc-cache"));
}

#[test]
fn doctor_active_processes_json_matches_new_cache_rules() {
    let output = common::command::rebecca()
        .env(
            "REBECCA_ACTIVE_PROCESSES",
            "Acrobat.exe:20;thunderbird.exe:21",
        )
        .args(["doctor", "active-processes", "--format", "json"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let data = common::support::api_data(&output.stdout);
    let matches = data["matches"].as_array().unwrap();
    let rule_ids = matches
        .iter()
        .flat_map(|matched| matched["rule_ids"].as_array().unwrap())
        .map(|rule_id| rule_id.as_str().unwrap())
        .collect::<std::collections::BTreeSet<_>>();

    assert!(rule_ids.contains("windows.adobe-reader-cache"));
    assert!(rule_ids.contains("windows.thunderbird-cache"));
}

#[test]
fn doctor_active_processes_json_degrades_without_process_adapter() {
    let output = common::command::rebecca()
        .args(["doctor", "active-processes", "--format", "json"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let data = common::support::api_data(&output.stdout);
    assert!(data["platform"].as_str().is_some());
    assert!(data["platform_supported"].as_bool().is_some());
    assert!(data["process_inspection_available"].as_bool().is_some());
    assert!(data["matches"].as_array().is_some());
}

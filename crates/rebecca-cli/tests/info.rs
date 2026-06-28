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

use std::fs;

mod common;
use rebecca::core::history::HistoryStore;
use rebecca::core::plan::{CleanupPlan, CleanupTarget};
use rebecca::core::{DeleteMode, PlanRequest, Platform, TargetStatus};

#[test]
fn config_paths_json_is_parseable() {
    let temp = tempfile::tempdir().unwrap();
    let output = common::isolated::isolated_rebecca(&temp)
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
            .unwrap()["lifecycle"]
            .as_str()
            .unwrap(),
        "rebuildable-cache"
    );
    assert_eq!(
        storage
            .iter()
            .find(|entry| entry["id"].as_str().unwrap() == "history-file")
            .unwrap()["retention"]
            .as_str()
            .unwrap(),
        "preserve"
    );
}

#[test]
fn config_paths_json_respects_config_file_overrides() {
    let temp = tempfile::tempdir().unwrap();
    let config_dir = temp.path().join("rebecca-config");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(
        config_dir.join("config.toml"),
        r#"
version = 1

[app_paths]
state_dir = "C:\\Rebecca\\State"
cache_dir = "C:\\Rebecca\\Cache"
history_file = "C:\\Rebecca\\State\\audit.jsonl"
"#,
    )
    .unwrap();

    let output = common::isolated::isolated_rebecca(&temp)
        .env_remove("REBECCA_STATE_DIR")
        .env_remove("REBECCA_CACHE_DIR")
        .env_remove("REBECCA_HISTORY_FILE")
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
        value["state_dir"]
            .as_str()
            .unwrap()
            .ends_with("Rebecca/State")
    );
    assert!(
        value["cache_dir"]
            .as_str()
            .unwrap()
            .ends_with("Rebecca/Cache")
    );
    assert!(
        value["history_file"]
            .as_str()
            .unwrap()
            .ends_with("Rebecca/State/audit.jsonl")
    );
    assert!(!value["state_dir"].as_str().unwrap().contains('\\'));
    assert!(!value["cache_dir"].as_str().unwrap().contains('\\'));
    assert!(!value["history_file"].as_str().unwrap().contains('\\'));
}

#[test]
fn config_paths_reports_malformed_config_file() {
    let temp = tempfile::tempdir().unwrap();
    let config_dir = temp.path().join("rebecca-config");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(config_dir.join("config.toml"), "[app_paths\n").unwrap();

    let output = common::isolated::isolated_rebecca(&temp)
        .args(["config", "paths"])
        .output()
        .unwrap();

    assert!(!output.status.success());

    let stderr = common::support::stderr(&output);
    assert!(stderr.contains("config parse failed"));
    assert!(stderr.contains("config.toml"));
}

#[test]
fn config_paths_reports_unsupported_config_version() {
    let temp = tempfile::tempdir().unwrap();
    let config_dir = temp.path().join("rebecca-config");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(config_dir.join("config.toml"), "version = 2\n").unwrap();

    let output = common::isolated::isolated_rebecca(&temp)
        .args(["config", "paths"])
        .output()
        .unwrap();

    assert!(!output.status.success());

    let stderr = common::support::stderr(&output);
    assert!(stderr.contains("config parse failed"));
    assert!(stderr.contains("unsupported config version 2"));
    assert!(stderr.contains("config.toml"));
}

#[test]
fn history_uses_config_file_overrides() {
    let temp = tempfile::tempdir().unwrap();
    let config_dir = temp.path().join("rebecca-config");
    let history_path = temp.path().join("custom-state").join("audit.jsonl");
    fs::create_dir_all(&config_dir).unwrap();
    if let Some(parent) = history_path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(
        config_dir.join("config.toml"),
        format!(
            r#"
version = 1

[app_paths]
history_file = '{}'
"#,
            history_path.display()
        ),
    )
    .unwrap();

    let mut plan = CleanupPlan::empty(PlanRequest::for_platform(
        Platform::Windows,
        DeleteMode::RecoverableDelete,
    ));
    let mut target = CleanupTarget::allowed(
        "windows.user-temp",
        std::path::PathBuf::from(r"C:\Temp\file.tmp"),
        10,
        DeleteMode::RecoverableDelete,
    );
    target.status = TargetStatus::Completed;
    target.pending_reclaim_bytes = 10;
    plan.targets.push(target);
    plan.recompute_summary();
    HistoryStore::new(history_path.clone())
        .append_plan(&plan)
        .unwrap();

    let output = common::isolated::isolated_rebecca(&temp)
        .env_remove("REBECCA_HISTORY_FILE")
        .args(["history", "--format", "json"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let value: serde_json::Value = common::support::api_data(&output.stdout);
    assert_eq!(value.as_array().unwrap().len(), 1);
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
    assert!(stdout.contains("Suggested action:"));
    if cfg!(target_os = "macos") {
        assert!(stdout.contains("macOS privacy:"));
        assert!(stdout.contains("macOS privacy action:"));
    }
}

#[test]
fn doctor_help_omits_steam_command() {
    let output = common::command::rebecca()
        .args(["doctor", "--help"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("permissions"));
    assert!(!stdout.contains("steam"));
}

use std::fs;
use std::path::{Path, PathBuf};

mod common;
#[path = "common/isolated.rs"]
mod isolated;

fn write_fixture_file(path: impl AsRef<Path>, bytes: &[u8]) {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, bytes).unwrap();
}

fn appdata_roots(temp: &tempfile::TempDir) -> (PathBuf, PathBuf) {
    (
        temp.path().join("AppData").join("Local"),
        temp.path().join("AppData").join("Roaming"),
    )
}

fn write_app_leftover_fixture(temp: &tempfile::TempDir, app_name: &str) {
    let app = temp.path().join("AppData").join("Local").join(app_name);
    write_fixture_file(app.join("Cache").join("cache.bin"), b"abc");
    write_fixture_file(app.join("Local Storage").join("state.bin"), b"keep");
}

#[test]
fn apps_help_shows_scan_and_clean_subcommands() {
    let output = common::command::rebecca()
        .args(["apps", "--help"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("scan"));
    assert!(stdout.contains("clean"));
}

#[test]
fn apps_clean_help_lists_scan_cache_opt_out() {
    let output = common::command::rebecca()
        .args(["apps", "clean", "--help"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--scan-cache"));
    assert!(stdout.contains("--no-scan-cache"));
}

#[test]
fn apps_scan_json_builds_app_leftovers_plan() {
    let temp = tempfile::tempdir().unwrap();
    let (local, roaming) = appdata_roots(&temp);
    let cache = local.join("Example App").join("Cache");
    let durable = local.join("Example App").join("Local Storage");
    write_fixture_file(cache.join("cache.bin"), b"abc");
    write_fixture_file(durable.join("state.bin"), b"keep");

    let output = isolated::isolated_rebecca(&temp)
        .env("REBECCA_STEAM_DISCOVERY", "none")
        .env("REBECCA_INSTALLED_APPLICATIONS", "Example App")
        .env("LOCALAPPDATA", &local)
        .env("APPDATA", &roaming)
        .args(["apps", "scan", "--format", "json", "--no-progress"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let envelope = common::support::api_envelope(&output.stdout);
    assert_eq!(envelope["command"], "apps scan");
    assert_eq!(envelope["payload_kind"], "app-leftovers-cleanup-plan");

    let value: serde_json::Value = envelope["data"].clone();
    assert_eq!(value["request"]["workflow"], "app-leftovers");
    assert_eq!(value["request"]["mode"], "dry-run");
    assert_eq!(value["summary"]["allowed_targets"], 1);
    assert_eq!(value["summary"]["estimated_bytes"], 3);

    let targets = value["targets"].as_array().unwrap();
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0]["rule_id"], "windows.app-leftover-local-cache");
    assert_eq!(targets[0]["status"], "allowed");
    assert!(
        PathBuf::from(targets[0]["path"].as_str().unwrap())
            .ends_with(Path::new("Example App").join("Cache"))
    );
    assert!(
        !targets[0]["path"]
            .as_str()
            .unwrap()
            .contains("Local Storage")
    );
}

#[test]
fn apps_scan_ndjson_uses_apps_scan_command_identity() {
    let temp = tempfile::tempdir().unwrap();
    let (local, roaming) = appdata_roots(&temp);
    write_fixture_file(
        local.join("Example App").join("Cache").join("cache.bin"),
        b"abc",
    );

    let output = isolated::isolated_rebecca(&temp)
        .env("REBECCA_STEAM_DISCOVERY", "none")
        .env("REBECCA_INSTALLED_APPLICATIONS", "Example App")
        .env("LOCALAPPDATA", &local)
        .env("APPDATA", &roaming)
        .args(["apps", "scan", "--format", "ndjson", "--no-progress"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let events = stdout
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();

    assert_eq!(events.first().unwrap()["event_kind"], "started");
    assert_eq!(events.last().unwrap()["event_kind"], "completed");
    assert!(events.iter().all(|event| event["command"] == "apps scan"));
    assert_eq!(
        events.last().unwrap()["payload_kind"],
        "app-leftovers-cleanup-plan"
    );
}

#[test]
fn apps_scan_json_builds_wechat_leftovers_plan() {
    let temp = tempfile::tempdir().unwrap();
    let (local, roaming) = appdata_roots(&temp);
    write_app_leftover_fixture(&temp, "WeChat");

    let output = isolated::isolated_rebecca(&temp)
        .env("REBECCA_STEAM_DISCOVERY", "none")
        .env("REBECCA_INSTALLED_APPLICATIONS", "WeChat")
        .env("LOCALAPPDATA", &local)
        .env("APPDATA", &roaming)
        .args(["apps", "scan", "--format", "json", "--no-progress"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let envelope = common::support::api_envelope(&output.stdout);
    assert_eq!(envelope["command"], "apps scan");
    assert_eq!(envelope["payload_kind"], "app-leftovers-cleanup-plan");

    let value: serde_json::Value = envelope["data"].clone();
    assert_eq!(value["request"]["workflow"], "app-leftovers");
    assert_eq!(value["summary"]["allowed_targets"], 1);
    assert_eq!(value["summary"]["estimated_bytes"], 3);

    let targets = value["targets"].as_array().unwrap();
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0]["rule_id"], "windows.app-leftover-local-cache");
    assert_eq!(targets[0]["status"], "allowed");
    assert_eq!(targets[0]["estimate_source"], "fresh-scan");
    assert!(
        PathBuf::from(targets[0]["path"].as_str().unwrap())
            .ends_with(Path::new("WeChat").join("Cache"))
    );
    assert!(
        !targets[0]["path"]
            .as_str()
            .unwrap()
            .contains("Local Storage")
    );
}

#[cfg(windows)]
#[test]
fn apps_clean_yes_deletes_wechat_leftover_cache_contents() {
    let temp = tempfile::tempdir().unwrap();
    let (local, roaming) = appdata_roots(&temp);
    write_app_leftover_fixture(&temp, "WeChat");
    let cache_dir = local.join("WeChat").join("Cache");
    let durable_state = local.join("WeChat").join("Local Storage").join("state.bin");

    let output = isolated::isolated_rebecca(&temp)
        .env("REBECCA_STEAM_DISCOVERY", "none")
        .env("REBECCA_INSTALLED_APPLICATIONS", "WeChat")
        .env("LOCALAPPDATA", &local)
        .env("APPDATA", &roaming)
        .args([
            "apps",
            "clean",
            "--yes",
            "--format",
            "json",
            "--no-progress",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    assert!(
        cache_dir.exists(),
        "app cache directory should be preserved"
    );
    assert_eq!(
        fs::read_dir(&cache_dir).unwrap().count(),
        0,
        "app cache directory should be emptied"
    );
    assert!(
        durable_state.exists(),
        "durable app state must remain untouched"
    );

    let value: serde_json::Value = common::support::api_data(&output.stdout);
    assert_eq!(value["request"]["workflow"], "app-leftovers");
    assert_eq!(value["request"]["mode"], "recycle-bin");
    assert_eq!(value["summary"]["completed_targets"], 1);
    assert_eq!(value["summary"]["blocked_targets"], 0);
    assert_eq!(value["targets"].as_array().unwrap().len(), 1);
    assert_eq!(value["targets"][0]["status"], "completed");
    assert_eq!(
        value["targets"][0]["rule_id"],
        "windows.app-leftover-local-cache"
    );
}

#[cfg(windows)]
#[test]
fn apps_clean_yes_respects_exclude_path_during_execution() {
    let temp = tempfile::tempdir().unwrap();
    let (local, roaming) = appdata_roots(&temp);
    write_app_leftover_fixture(&temp, "WeChat");
    let code_cache_dir = local.join("WeChat").join("Code Cache");
    write_fixture_file(code_cache_dir.join("code.bin"), b"def");
    let cache_dir = local.join("WeChat").join("Cache");

    let output = isolated::isolated_rebecca(&temp)
        .env("REBECCA_STEAM_DISCOVERY", "none")
        .env("REBECCA_INSTALLED_APPLICATIONS", "WeChat")
        .env("LOCALAPPDATA", &local)
        .env("APPDATA", &roaming)
        .args([
            "apps",
            "clean",
            "--yes",
            "--format",
            "json",
            "--no-progress",
            "--exclude",
            cache_dir.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    assert!(
        cache_dir.exists(),
        "excluded app cache directory should be preserved"
    );
    assert_eq!(fs::read_dir(&cache_dir).unwrap().count(), 1);
    assert!(
        code_cache_dir.exists(),
        "allowed app cache directory should still exist"
    );
    assert_eq!(
        fs::read_dir(&code_cache_dir).unwrap().count(),
        0,
        "allowed app cache directory should be emptied"
    );

    let value: serde_json::Value = common::support::api_data(&output.stdout);
    assert_eq!(value["request"]["workflow"], "app-leftovers");
    assert_eq!(value["request"]["mode"], "recycle-bin");
    assert_eq!(value["summary"]["total_targets"], 2);
    assert_eq!(value["summary"]["completed_targets"], 1);
    assert_eq!(value["summary"]["blocked_targets"], 1);
    assert_eq!(value["targets"].as_array().unwrap().len(), 2);
    assert!(value["targets"].as_array().unwrap().iter().any(|target| {
        target["status"] == "blocked"
            && target["reason_code"] == "safety-policy-blocked"
            && target["path"]
                .as_str()
                .unwrap()
                .contains(r"AppData\Local\WeChat\Cache")
    }));
    assert!(value["targets"].as_array().unwrap().iter().any(|target| {
        target["status"] == "completed"
            && target["rule_id"] == "windows.app-leftover-local-cache"
            && target["path"]
                .as_str()
                .unwrap()
                .contains(r"AppData\Local\WeChat\Code Cache")
    }));
}

#[test]
fn apps_scan_json_honors_exclude_flag() {
    let temp = tempfile::tempdir().unwrap();
    let (local, roaming) = appdata_roots(&temp);
    let cache = local.join("Example App").join("Cache");
    write_fixture_file(cache.join("cache.bin"), b"abc");

    let output = isolated::isolated_rebecca(&temp)
        .env("REBECCA_STEAM_DISCOVERY", "none")
        .env("REBECCA_INSTALLED_APPLICATIONS", "Example App")
        .env("LOCALAPPDATA", &local)
        .env("APPDATA", &roaming)
        .args([
            "apps",
            "scan",
            "--format",
            "json",
            "--no-progress",
            "--exclude",
            cache.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let value: serde_json::Value = common::support::api_data(&output.stdout);
    assert_eq!(value["summary"]["allowed_targets"], 0);
    assert_eq!(value["summary"]["blocked_targets"], 1);

    let blocked = &value["targets"].as_array().unwrap()[0];
    assert_eq!(blocked["status"], "blocked");
    assert_eq!(blocked["reason_code"], "safety-policy-blocked");
    assert!(
        blocked["reason"]
            .as_str()
            .unwrap()
            .contains("user-protected path")
    );
}

#[test]
fn apps_scan_human_output_names_app_leftovers_workflow() {
    let temp = tempfile::tempdir().unwrap();
    let (local, roaming) = appdata_roots(&temp);
    write_fixture_file(
        local.join("Example App").join("Cache").join("cache.bin"),
        b"abc",
    );

    let output = isolated::isolated_rebecca(&temp)
        .env("REBECCA_STEAM_DISCOVERY", "none")
        .env("REBECCA_INSTALLED_APPLICATIONS", "Example App")
        .env("LOCALAPPDATA", &local)
        .env("APPDATA", &roaming)
        .args(["apps", "scan", "--no-progress"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Workflow: App leftovers"));
    assert!(stdout.contains("Cleanup mode: dry-run"));
    assert!(stdout.contains("windows.app-leftover-local-cache"));
}

#[test]
fn apps_clean_defaults_to_preview_without_deleting() {
    let temp = tempfile::tempdir().unwrap();
    let (local, roaming) = appdata_roots(&temp);
    let cache_file = local.join("Example App").join("Cache").join("cache.bin");
    write_fixture_file(&cache_file, b"abc");

    let output = isolated::isolated_rebecca(&temp)
        .env("REBECCA_STEAM_DISCOVERY", "none")
        .env("REBECCA_INSTALLED_APPLICATIONS", "Example App")
        .env("LOCALAPPDATA", &local)
        .env("APPDATA", &roaming)
        .args(["apps", "clean", "--format", "json", "--no-progress"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );
    assert!(cache_file.exists(), "apps clean should preview by default");

    let value: serde_json::Value = common::support::api_data(&output.stdout);
    assert_eq!(value["request"]["workflow"], "app-leftovers");
    assert_eq!(value["request"]["mode"], "dry-run");
    assert_eq!(value["summary"]["allowed_targets"], 1);

    let scan_cache_dir = temp.path().join("rebecca-cache").join("scan");
    let cache_entries = fs::read_dir(scan_cache_dir).unwrap().count();
    assert_eq!(cache_entries, 1);
}

#[test]
fn apps_clean_no_scan_cache_disables_preview_cache_writes() {
    let temp = tempfile::tempdir().unwrap();
    let (local, roaming) = appdata_roots(&temp);
    let cache_file = local.join("Example App").join("Cache").join("cache.bin");
    write_fixture_file(&cache_file, b"abc");

    let output = isolated::isolated_rebecca(&temp)
        .env("REBECCA_STEAM_DISCOVERY", "none")
        .env("REBECCA_INSTALLED_APPLICATIONS", "Example App")
        .env("LOCALAPPDATA", &local)
        .env("APPDATA", &roaming)
        .args([
            "apps",
            "clean",
            "--format",
            "json",
            "--no-progress",
            "--no-scan-cache",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );
    assert!(cache_file.exists(), "apps clean should preview by default");

    let value: serde_json::Value = common::support::api_data(&output.stdout);
    assert_eq!(value["request"]["workflow"], "app-leftovers");
    assert_eq!(value["request"]["mode"], "dry-run");
    assert_eq!(value["summary"]["allowed_targets"], 1);
    assert!(!temp.path().join("rebecca-cache").join("scan").exists());
}

#[test]
fn apps_clean_dry_run_blocks_rebecca_owned_storage_overlap() {
    let temp = tempfile::tempdir().unwrap();
    let local = temp
        .path()
        .join("rebecca-cache")
        .join("AppData")
        .join("Local");
    let roaming = temp.path().join("AppData").join("Roaming");
    let cache_file = local.join("Example App").join("Cache").join("cache.bin");
    write_fixture_file(&cache_file, b"abc");

    let output = isolated::isolated_rebecca(&temp)
        .env("REBECCA_STEAM_DISCOVERY", "none")
        .env("REBECCA_INSTALLED_APPLICATIONS", "Example App")
        .env("LOCALAPPDATA", &local)
        .env("APPDATA", &roaming)
        .args([
            "apps",
            "clean",
            "--dry-run",
            "--format",
            "json",
            "--no-progress",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );
    assert!(cache_file.exists(), "dry-run must not delete files");

    let value: serde_json::Value = common::support::api_data(&output.stdout);
    assert_eq!(value["summary"]["allowed_targets"], 0);
    assert_eq!(value["summary"]["blocked_targets"], 1);
    assert!(
        value["summary"]["issue_matrix"]
            .as_array()
            .unwrap()
            .iter()
            .any(|issue| issue["status"] == "blocked"
                && issue["reason_code"] == "safety-policy-blocked"
                && issue["targets"] == 1)
    );

    let blocked = &value["targets"].as_array().unwrap()[0];
    assert_eq!(blocked["status"], "blocked");
    assert_eq!(blocked["reason_code"], "safety-policy-blocked");
    assert!(
        blocked["reason"]
            .as_str()
            .unwrap()
            .contains("Rebecca-owned Cache dir")
    );
}

#[test]
fn apps_scan_empty_inventory_reports_empty_plan() {
    let temp = tempfile::tempdir().unwrap();

    let output = isolated::isolated_rebecca(&temp)
        .env("REBECCA_APP_DISCOVERY", "none")
        .args(["apps", "scan", "--format", "json", "--no-progress"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let value: serde_json::Value = common::support::api_data(&output.stdout);
    assert_eq!(value["request"]["workflow"], "app-leftovers");
    assert_eq!(value["summary"]["total_targets"], 0);
    assert_eq!(value["summary"]["allowed_targets"], 0);
}

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
        .args(["apps", "scan", "--json", "--no-progress"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
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
        .args(["apps", "clean", "--json", "--no-progress"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );
    assert!(cache_file.exists(), "apps clean should preview by default");

    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["request"]["workflow"], "app-leftovers");
    assert_eq!(value["request"]["mode"], "dry-run");
    assert_eq!(value["summary"]["allowed_targets"], 1);
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
        .args(["apps", "clean", "--dry-run", "--json", "--no-progress"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );
    assert!(cache_file.exists(), "dry-run must not delete files");

    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
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
        .args(["apps", "scan", "--json", "--no-progress"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["request"]["workflow"], "app-leftovers");
    assert_eq!(value["summary"]["total_targets"], 0);
    assert_eq!(value["summary"]["allowed_targets"], 0);
}

use std::fs;

mod common;

fn write_temp_cache_fixture(temp: &tempfile::TempDir) -> (std::path::PathBuf, std::path::PathBuf) {
    let temp_cache = temp.path().join("temp");
    fs::create_dir_all(&temp_cache).unwrap();
    let cache_file = temp_cache.join("cache.tmp");
    fs::write(&cache_file, b"cache").unwrap();
    (temp_cache, cache_file)
}

fn save_user_temp_plan(
    temp: &tempfile::TempDir,
    temp_cache: &std::path::Path,
    plan_file: &std::path::Path,
) {
    let output = common::isolated::isolated_rebecca(temp)
        .env("REBECCA_STEAM_DISCOVERY", "none")
        .env("TEMP", temp_cache)
        .env("TMPDIR", temp_cache)
        .args([
            "clean",
            "--dry-run",
            "--format",
            "json",
            "--no-progress",
            "--no-scan-cache",
            "--rule",
            common::support::current_platform_user_temp_rule_id(),
            "--save-plan",
            plan_file.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );
}

#[test]
fn clean_yes_receipt_reports_recoverable_trash_execution() {
    let temp = tempfile::tempdir().unwrap();
    let (temp_cache, cache_file) = write_temp_cache_fixture(&temp);
    let receipt_file = temp.path().join("receipts").join("cleanup-receipt.json");

    let output = common::isolated::isolated_rebecca(&temp)
        .env("REBECCA_STEAM_DISCOVERY", "none")
        .env("TEMP", &temp_cache)
        .env("TMPDIR", &temp_cache)
        .args([
            "clean",
            "--yes",
            "--format",
            "json",
            "--no-progress",
            "--no-scan-cache",
            "--rule",
            common::support::current_platform_user_temp_rule_id(),
            "--receipt",
            receipt_file.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );
    assert!(!cache_file.exists(), "cache file should move to test trash");

    let data = common::support::api_data(&output.stdout);
    assert_eq!(data["request"]["mode"], "recoverable-delete");
    assert_eq!(data["summary"]["completed_targets"], 1);

    let receipt: serde_json::Value =
        serde_json::from_slice(&fs::read(&receipt_file).unwrap()).unwrap();
    assert_eq!(receipt["schema"], "rebecca.cleanup-receipt.v1");
    assert_eq!(receipt["command"], "clean");
    assert_eq!(receipt["workflow"], "rules");
    assert_eq!(receipt["mode"], "recoverable-delete");
    if cfg!(target_os = "windows") {
        assert_eq!(receipt["destination"], "windows-recycle-bin");
    } else {
        assert_eq!(receipt["destination"], "system-trash");
    }
    assert_eq!(receipt["summary"]["completed_targets"], 1);
    assert_eq!(receipt["summary"]["pending_reclaim_bytes"], 5);
    assert_eq!(
        receipt["execution_report"]["summary"]["pending_reclaim_bytes"],
        5
    );
    assert!(
        receipt["targets"]
            .as_array()
            .unwrap()
            .iter()
            .any(|target| target["status"] == "completed" && target["pending_reclaim_bytes"] == 5)
    );
    assert!(
        receipt["next_steps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|step| step["kind"] == "empty-trash"
                && step["command"]
                    .as_str()
                    .unwrap()
                    .contains("trash empty --yes"))
    );
}

#[test]
fn plan_run_yes_receipt_records_revalidated_execution() {
    let temp = tempfile::tempdir().unwrap();
    let (temp_cache, cache_file) = write_temp_cache_fixture(&temp);
    let plan_file = temp.path().join("cleanup-plan.json");
    let receipt_file = temp.path().join("cleanup-receipt.json");
    save_user_temp_plan(&temp, &temp_cache, &plan_file);

    let output = common::isolated::isolated_rebecca(&temp)
        .args([
            "--format",
            "json",
            "plan",
            "run",
            plan_file.to_str().unwrap(),
            "--yes",
            "--receipt",
            receipt_file.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );
    assert!(!cache_file.exists(), "saved plan target should execute");

    let envelope = common::support::api_envelope(&output.stdout);
    assert_eq!(envelope["command"], "plan run");
    assert_eq!(envelope["data"]["summary"]["completed_targets"], 1);

    let receipt: serde_json::Value =
        serde_json::from_slice(&fs::read(&receipt_file).unwrap()).unwrap();
    assert_eq!(receipt["schema"], "rebecca.cleanup-receipt.v1");
    assert_eq!(receipt["command"], "plan run");
    assert_eq!(receipt["summary"]["completed_targets"], 1);
    assert_eq!(
        receipt["execution_report"]["summary"]["completed_actions"],
        1
    );
}

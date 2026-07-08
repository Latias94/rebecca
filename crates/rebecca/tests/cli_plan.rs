use std::fs;

mod common;

fn write_temp_cache_fixture(temp: &tempfile::TempDir) -> (std::path::PathBuf, std::path::PathBuf) {
    let temp_cache = temp.path().join("temp");
    fs::create_dir_all(&temp_cache).unwrap();
    let cache_file = temp_cache.join("cache.tmp");
    fs::write(&cache_file, b"cache").unwrap();
    (temp_cache, cache_file)
}

fn write_fixture_file(path: impl AsRef<std::path::Path>, bytes: &[u8]) {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, bytes).unwrap();
}

fn write_rust_project(dir: impl AsRef<std::path::Path>) {
    write_fixture_file(dir.as_ref().join("Cargo.toml"), b"[package]");
}

fn save_user_temp_plan(
    temp: &tempfile::TempDir,
    temp_cache: &std::path::Path,
    plan_file: &std::path::Path,
) -> serde_json::Value {
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
    common::support::api_data(&output.stdout)
}

#[test]
fn clean_save_plan_writes_reviewable_dry_run_file() {
    let temp = tempfile::tempdir().unwrap();
    let (temp_cache, cache_file) = write_temp_cache_fixture(&temp);
    let plan_file = temp.path().join("cleanup-plan.json");

    let data = save_user_temp_plan(&temp, &temp_cache, &plan_file);

    assert!(cache_file.exists(), "saving a plan must not delete files");
    assert_eq!(data["request"]["mode"], "dry-run");
    assert!(data["summary"]["allowed_targets"].as_u64().unwrap() >= 1);

    let saved: serde_json::Value = serde_json::from_slice(&fs::read(&plan_file).unwrap()).unwrap();
    assert_eq!(saved["schema"], "rebecca.saved-cleanup-plan.v1");
    assert_eq!(saved["plan"]["request"]["mode"], "dry-run");
    assert!(!saved["target_fingerprints"].as_array().unwrap().is_empty());
    assert!(
        saved["target_fingerprints"]
            .as_array()
            .unwrap()
            .iter()
            .any(|fingerprint| fingerprint["metadata"]["kind"] == "directory")
    );
}

#[test]
fn plan_inspect_json_reports_saved_cleanup_plan() {
    let temp = tempfile::tempdir().unwrap();
    let (temp_cache, _cache_file) = write_temp_cache_fixture(&temp);
    let plan_file = temp.path().join("cleanup-plan.json");
    save_user_temp_plan(&temp, &temp_cache, &plan_file);

    let output = common::isolated::isolated_rebecca(&temp)
        .args([
            "--format",
            "json",
            "plan",
            "inspect",
            plan_file.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let envelope = common::support::api_envelope(&output.stdout);
    assert_eq!(envelope["command"], "plan inspect");
    assert_eq!(envelope["payload_kind"], "saved-cleanup-plan");
    assert_eq!(envelope["data"]["schema"], "rebecca.saved-cleanup-plan.v1");
    assert_eq!(envelope["data"]["plan"]["request"]["mode"], "dry-run");
}

#[test]
fn plan_run_without_yes_revalidates_without_deleting() {
    let temp = tempfile::tempdir().unwrap();
    let (temp_cache, cache_file) = write_temp_cache_fixture(&temp);
    let plan_file = temp.path().join("cleanup-plan.json");
    save_user_temp_plan(&temp, &temp_cache, &plan_file);

    let output = common::isolated::isolated_rebecca(&temp)
        .args([
            "--format",
            "json",
            "plan",
            "run",
            plan_file.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );
    assert!(
        cache_file.exists(),
        "plan run without --yes must not delete"
    );

    let envelope = common::support::api_envelope(&output.stdout);
    assert_eq!(envelope["command"], "plan run");
    assert_eq!(envelope["payload_kind"], "cleanup-plan");
    assert_eq!(envelope["data"]["request"]["mode"], "dry-run");
    assert!(
        envelope["data"]["summary"]["allowed_targets"]
            .as_u64()
            .unwrap()
            >= 1
    );
}

#[test]
fn purge_save_plan_runs_as_project_artifact_plan() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    let target_file = workspace
        .join("app")
        .join("target")
        .join("debug")
        .join("app.bin");
    write_fixture_file(&target_file, b"rust");
    write_rust_project(workspace.join("app"));
    let plan_file = temp.path().join("purge-plan.json");

    let save = common::isolated::isolated_rebecca(&temp)
        .args([
            "purge",
            "--dry-run",
            "--format",
            "json",
            "--no-progress",
            "--root",
            workspace.to_str().unwrap(),
            "--min-age-days",
            "0",
            "--save-plan",
            plan_file.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(
        save.status.success(),
        "stderr: {}",
        common::support::stderr(&save)
    );
    assert!(target_file.exists(), "saving a purge plan must not delete");

    let output = common::isolated::isolated_rebecca(&temp)
        .args([
            "--format",
            "json",
            "plan",
            "run",
            plan_file.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );
    assert!(
        target_file.exists(),
        "plan run without --yes must not delete"
    );

    let envelope = common::support::api_envelope(&output.stdout);
    assert_eq!(envelope["command"], "plan run");
    assert_eq!(envelope["payload_kind"], "project-artifact-cleanup-plan");
    assert_eq!(envelope["data"]["request"]["workflow"], "project-artifacts");
    assert_eq!(envelope["data"]["summary"]["allowed_targets"], 1);
}

#[test]
fn plan_run_yes_executes_still_valid_saved_targets() {
    let temp = tempfile::tempdir().unwrap();
    let (temp_cache, cache_file) = write_temp_cache_fixture(&temp);
    let plan_file = temp.path().join("cleanup-plan.json");
    save_user_temp_plan(&temp, &temp_cache, &plan_file);

    let output = common::isolated::isolated_rebecca(&temp)
        .args([
            "--format",
            "json",
            "plan",
            "run",
            plan_file.to_str().unwrap(),
            "--yes",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );
    assert!(temp_cache.exists(), "user temp root should be preserved");
    assert!(
        !cache_file.exists(),
        "temp contents should move to test trash"
    );

    let envelope = common::support::api_envelope(&output.stdout);
    assert_eq!(envelope["command"], "plan run");
    assert_eq!(envelope["data"]["request"]["mode"], "recoverable-delete");
    assert_eq!(envelope["data"]["summary"]["completed_targets"], 1);
    assert_eq!(envelope["data"]["summary"]["pending_reclaim_bytes"], 5);
}

#[test]
fn plan_run_yes_blocks_saved_target_when_path_type_changed() {
    let temp = tempfile::tempdir().unwrap();
    let (temp_cache, cache_file) = write_temp_cache_fixture(&temp);
    let plan_file = temp.path().join("cleanup-plan.json");
    save_user_temp_plan(&temp, &temp_cache, &plan_file);

    let mut saved: serde_json::Value =
        serde_json::from_slice(&fs::read(&plan_file).unwrap()).unwrap();
    let allowed_fingerprint = saved["target_fingerprints"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|fingerprint| fingerprint["status"] == "allowed")
        .expect("saved plan should include an allowed target fingerprint");
    allowed_fingerprint["metadata"]["kind"] = serde_json::json!("file");
    fs::write(&plan_file, serde_json::to_vec_pretty(&saved).unwrap()).unwrap();

    let output = common::isolated::isolated_rebecca(&temp)
        .args([
            "--format",
            "json",
            "plan",
            "run",
            plan_file.to_str().unwrap(),
            "--yes",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );
    assert!(
        temp_cache.is_dir(),
        "stale saved plan should not remove root"
    );
    assert!(
        cache_file.exists(),
        "stale saved plan should not remove files"
    );

    let envelope = common::support::api_envelope(&output.stdout);
    assert_eq!(envelope["data"]["summary"]["allowed_targets"], 0);
    assert!(
        envelope["data"]["summary"]["issue_matrix"]
            .as_array()
            .unwrap()
            .iter()
            .any(|issue| issue["reason_code"] == "saved-plan-target-changed")
    );
    assert!(
        envelope["data"]["targets"]
            .as_array()
            .unwrap()
            .iter()
            .any(|target| target["reason_code"] == "saved-plan-target-changed")
    );
}

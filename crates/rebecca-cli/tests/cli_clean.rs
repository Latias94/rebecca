use std::fs;
use std::path::{Path, PathBuf};

mod common;
#[path = "common/isolated.rs"]
mod isolated;

fn steam_dry_run_json_output(
    temp: &tempfile::TempDir,
    case: &common::steam::SteamRuleCase,
) -> serde_json::Value {
    let steam = temp.path().join("Steam");
    case.write_fixture(&steam);

    let mut command = isolated::isolated_rebecca(temp);
    command.env("REBECCA_STEAM_DISCOVERY_PATH", &steam).args([
        "clean",
        "--dry-run",
        "--json",
        "--rule",
        case.rule_id,
    ]);
    if case.allow_moderate {
        command.args(["--allow-moderate"]);
    }

    let output = command.output().unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    serde_json::from_slice(&output.stdout).unwrap()
}

fn write_fixture_file(path: impl AsRef<Path>, bytes: &[u8]) {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, bytes).unwrap();
}

fn write_slack_cache_fixture(temp: &tempfile::TempDir) {
    let slack = temp.path().join("roaming").join("Slack");
    write_fixture_file(slack.join("Cache").join("cache.bin"), b"ab");
    write_fixture_file(slack.join("Code Cache").join("code.bin"), b"cde");
    write_fixture_file(slack.join("GPUCache").join("gpu.bin"), b"fghi");
    write_fixture_file(
        slack.join("Local Storage").join("leveldb").join("LOG"),
        b"keep",
    );
    write_fixture_file(
        slack
            .join("IndexedDB")
            .join("indexeddb.leveldb")
            .join("LOG"),
        b"keep",
    );
    write_fixture_file(
        slack
            .join("Service Worker")
            .join("CacheStorage")
            .join("index.bin"),
        b"keep",
    );
}

#[test]
fn clean_dry_run_json_builds_plan_without_deleting() {
    let temp = tempfile::tempdir().unwrap();
    let temp_cache = temp.path().join("temp");
    fs::create_dir_all(&temp_cache).unwrap();
    let file = temp_cache.join("cache.tmp");
    fs::write(&file, b"cache").unwrap();

    let output = isolated::isolated_rebecca(&temp)
        .env("REBECCA_STEAM_DISCOVERY", "none")
        .env("TEMP", &temp_cache)
        .env("LOCALAPPDATA", temp.path().join("local"))
        .env("APPDATA", temp.path().join("roaming"))
        .args(["clean", "--dry-run", "--json", "--category", "system"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );
    assert!(file.exists(), "dry-run must not delete files");

    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["request"]["mode"], "dry-run");
    assert!(value["summary"]["allowed_targets"].as_u64().unwrap() >= 1);
}

#[test]
fn clean_dry_run_json_reports_slack_cache_rule() {
    let temp = tempfile::tempdir().unwrap();
    write_slack_cache_fixture(&temp);

    let output = isolated::isolated_rebecca(&temp)
        .args([
            "clean",
            "--dry-run",
            "--json",
            "--rule",
            "windows.slack-cache",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["summary"]["allowed_targets"], 3);
    assert_eq!(value["summary"]["skipped_targets"], 0);
    assert_eq!(value["summary"]["estimated_bytes"], 9);

    let targets = value["targets"].as_array().unwrap();
    let target_paths = targets
        .iter()
        .map(|target| PathBuf::from(target["path"].as_str().unwrap()))
        .collect::<Vec<_>>();

    for expected in [
        Path::new("Slack").join("Cache"),
        Path::new("Slack").join("Code Cache"),
        Path::new("Slack").join("GPUCache"),
    ] {
        assert!(
            target_paths.iter().any(|path| path.ends_with(&expected)),
            "missing Slack target {expected:?}"
        );
    }

    assert!(targets.iter().all(|target| {
        target["rule_id"] == "windows.slack-cache"
            && !target["path"].as_str().unwrap().contains("Local Storage")
            && !target["path"].as_str().unwrap().contains("IndexedDB")
            && !target["path"].as_str().unwrap().contains("Service Worker")
    }));
}

#[test]
fn clean_dry_run_json_honors_exclude_flag() {
    let temp = tempfile::tempdir().unwrap();
    write_slack_cache_fixture(&temp);
    let protected_cache = temp.path().join("roaming").join("Slack").join("Cache");

    let output = isolated::isolated_rebecca(&temp)
        .args([
            "clean",
            "--dry-run",
            "--json",
            "--rule",
            "windows.slack-cache",
            "--exclude",
            protected_cache.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["summary"]["total_targets"], 3);
    assert_eq!(value["summary"]["allowed_targets"], 2);
    assert_eq!(value["summary"]["blocked_targets"], 1);
    assert_eq!(value["summary"]["estimated_bytes"], 7);

    let blocked = value["targets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|target| target["status"] == "blocked")
        .expect("expected excluded target to be blocked");
    assert_eq!(blocked["rule_id"], "windows.slack-cache");
    assert_eq!(blocked["reason_code"], "safety-policy-blocked");
    assert!(
        blocked["reason"]
            .as_str()
            .unwrap()
            .contains("user-protected path")
    );
    assert!(
        PathBuf::from(blocked["path"].as_str().unwrap())
            .ends_with(Path::new("Slack").join("Cache"))
    );
}

#[test]
fn clean_dry_run_json_honors_config_protected_paths() {
    let temp = tempfile::tempdir().unwrap();
    write_slack_cache_fixture(&temp);
    let config_dir = temp.path().join("rebecca-config");
    fs::create_dir_all(&config_dir).unwrap();
    let protected_cache = temp.path().join("roaming").join("Slack").join("GPUCache");
    fs::write(
        config_dir.join("config.toml"),
        format!(
            r#"
version = 1

[protection]
protected_paths = ['{}']
"#,
            protected_cache.display()
        ),
    )
    .unwrap();

    let output = isolated::isolated_rebecca(&temp)
        .args([
            "clean",
            "--dry-run",
            "--json",
            "--rule",
            "windows.slack-cache",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["summary"]["allowed_targets"], 2);
    assert_eq!(value["summary"]["blocked_targets"], 1);
    assert!(value["targets"].as_array().unwrap().iter().any(|target| {
        target["status"] == "blocked"
            && target["reason"]
                .as_str()
                .unwrap()
                .contains("user-protected path")
            && PathBuf::from(target["path"].as_str().unwrap())
                .ends_with(Path::new("Slack").join("GPUCache"))
    }));
}

#[test]
fn clean_exclude_rejects_relative_paths() {
    let temp = tempfile::tempdir().unwrap();
    let output = isolated::isolated_rebecca(&temp)
        .args([
            "clean",
            "--dry-run",
            "--json",
            "--rule",
            "windows.user-temp",
            "--exclude",
            "relative/cache",
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = common::support::stderr(&output);
    assert!(stderr.contains("invalid protected path"));
    assert!(stderr.contains("path must be absolute"));
}

#[test]
fn clean_dry_run_json_deduplicates_overlapping_system_targets() {
    let temp = tempfile::tempdir().unwrap();
    let local = temp.path().join("local");
    let temp_cache = local.join("Temp");
    fs::create_dir_all(&temp_cache).unwrap();
    fs::write(temp_cache.join("cache.tmp"), b"cache").unwrap();

    let output = isolated::isolated_rebecca(&temp)
        .env("TEMP", &temp_cache)
        .env("LOCALAPPDATA", &local)
        .env("APPDATA", temp.path().join("roaming"))
        .args([
            "clean",
            "--dry-run",
            "--json",
            "--rule",
            "windows.user-temp",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["summary"]["total_targets"], 2);
    assert_eq!(value["summary"]["allowed_targets"], 1);
    assert_eq!(value["summary"]["skipped_targets"], 1);
    assert_eq!(value["summary"]["estimated_bytes"], 5);
    assert_eq!(value["summary"]["issue_matrix"][0]["status"], "skipped");
    assert_eq!(
        value["summary"]["issue_matrix"][0]["reason_code"],
        "duplicate-target-path"
    );
    assert_eq!(value["summary"]["issue_matrix"][0]["targets"], 1);
    assert!(
        value["targets"]
            .as_array()
            .unwrap()
            .iter()
            .any(|target| target["reason"] == "duplicate target path already covered")
    );
    assert!(
        value["targets"]
            .as_array()
            .unwrap()
            .iter()
            .any(|target| target["reason_code"] == "duplicate-target-path")
    );
}

#[test]
fn clean_dry_run_blocks_rebecca_storage_targets_from_config() {
    let temp = tempfile::tempdir().unwrap();
    let temp_cache = temp.path().join("temp");
    let config_dir = temp.path().join("rebecca-config");
    fs::create_dir_all(&temp_cache).unwrap();
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(temp_cache.join("cache.tmp"), b"cache").unwrap();
    fs::write(
        config_dir.join("config.toml"),
        format!(
            r#"
version = 1

[app_paths]
cache_dir = '{}'
"#,
            temp_cache.display()
        ),
    )
    .unwrap();

    let output = isolated::isolated_rebecca(&temp)
        .env_remove("REBECCA_CACHE_DIR")
        .env("TEMP", &temp_cache)
        .args([
            "clean",
            "--dry-run",
            "--json",
            "--rule",
            "windows.user-temp",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
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

    let blocked = value["targets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|target| target["status"] == "blocked")
        .expect("expected a blocked Rebecca-owned storage target");
    assert_eq!(blocked["reason_code"], "safety-policy-blocked");
    assert!(
        blocked["reason"]
            .as_str()
            .unwrap()
            .contains("Rebecca-owned Cache dir")
    );
    assert!(temp_cache.join("cache.tmp").exists());
}

#[test]
fn clean_human_output_reports_issue_matrix_for_skipped_targets() {
    let temp = tempfile::tempdir().unwrap();
    let local = temp.path().join("local");
    let temp_cache = local.join("Temp");
    fs::create_dir_all(&temp_cache).unwrap();
    fs::write(temp_cache.join("cache.tmp"), b"cache").unwrap();

    let output = isolated::isolated_rebecca(&temp)
        .env("TEMP", &temp_cache)
        .env("LOCALAPPDATA", &local)
        .env("APPDATA", temp.path().join("roaming"))
        .args(["clean", "--dry-run", "--category", "system"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Issue matrix:"));
    assert!(stdout.contains("- skipped duplicate-target-path: 1 target, 0 (0 B)"));
}

#[test]
fn clean_human_output_reports_slack_cache_rule() {
    let temp = tempfile::tempdir().unwrap();
    write_slack_cache_fixture(&temp);

    let output = isolated::isolated_rebecca(&temp)
        .args([
            "clean",
            "--dry-run",
            "--no-progress",
            "--rule",
            "windows.slack-cache",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Targets: 3"));
    assert!(stdout.contains("Allowed: 3"));
    assert!(stdout.contains("Target details:"));
    assert!(stdout.contains("allowed (3)"));
    assert!(stdout.contains("windows.slack-cache"));
    assert!(stdout.contains(&Path::new("Slack").join("Cache").display().to_string()));
    assert!(!stdout.contains("Local Storage"));
    assert!(!stdout.contains("IndexedDB"));
    assert!(!stdout.contains("Service Worker"));
}

#[test]
fn clean_human_output_reports_blocked_target_details() {
    let temp = tempfile::tempdir().unwrap();
    let temp_cache = temp.path().join("temp");
    let config_dir = temp.path().join("rebecca-config");
    fs::create_dir_all(&temp_cache).unwrap();
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(temp_cache.join("cache.tmp"), b"cache").unwrap();
    fs::write(
        config_dir.join("config.toml"),
        format!(
            r#"
version = 1

[app_paths]
cache_dir = '{}'
"#,
            temp_cache.display()
        ),
    )
    .unwrap();

    let output = isolated::isolated_rebecca(&temp)
        .env_remove("REBECCA_CACHE_DIR")
        .env("TEMP", &temp_cache)
        .args([
            "clean",
            "--dry-run",
            "--no-progress",
            "--rule",
            "windows.user-temp",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Issue matrix:"));
    assert!(stdout.contains("- blocked safety-policy-blocked: 1 target, 0 (0 B)"));
    assert!(stdout.contains("Target details:"));
    assert!(stdout.contains("blocked (1)"));
    assert!(stdout.contains("windows.user-temp"));
    assert!(stdout.contains(&temp_cache.display().to_string()));
    assert!(stdout.contains("Rebecca-owned Cache dir"));
}

#[test]
fn clean_human_output_highlights_largest_targets_by_size() {
    let temp = tempfile::tempdir().unwrap();
    let local = temp.path().join("local");
    let edge_cache = local.join("Microsoft/Edge/User Data/Default/Cache");
    let chrome_profile_code_cache = local.join("Google/Chrome/User Data/Profile 1/Code Cache");
    let chrome_default_cache = local.join("Google/Chrome/User Data/Default/Cache");

    fs::create_dir_all(&edge_cache).unwrap();
    fs::create_dir_all(&chrome_profile_code_cache).unwrap();
    fs::create_dir_all(&chrome_default_cache).unwrap();
    fs::write(edge_cache.join("edge.bin"), b"1234567890").unwrap();
    fs::write(chrome_profile_code_cache.join("code.bin"), b"123456").unwrap();
    fs::write(chrome_default_cache.join("chrome.bin"), b"1234").unwrap();

    let output = isolated::isolated_rebecca(&temp)
        .env("LOCALAPPDATA", &local)
        .args(["clean", "--dry-run", "--category", "browser"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Estimated bytes: 20 (20 B)"));
    assert!(stdout.contains("Largest estimated targets:"));
    assert!(stdout.contains("Target details:"));
    assert!(stdout.contains("allowed (3)"));
    assert!(stdout.contains("skipped ("));

    let largest_section = stdout
        .split("Largest estimated targets:")
        .nth(1)
        .expect("largest section should be present")
        .split("Target details:")
        .next()
        .expect("target details section should follow largest section");
    let edge_position = largest_section
        .find("windows.edge-cache")
        .expect("edge target should be in largest section");
    let chrome_position = largest_section
        .find("windows.chrome-cache")
        .expect("chrome target should be in largest section");

    assert!(
        edge_position < chrome_position,
        "largest section should sort targets by estimated bytes"
    );
}

#[test]
fn clean_dry_run_accepts_no_progress_flag() {
    let temp = tempfile::tempdir().unwrap();
    let temp_cache = temp.path().join("temp");
    fs::create_dir_all(&temp_cache).unwrap();
    fs::write(temp_cache.join("cache.tmp"), b"cache").unwrap();

    let output = isolated::isolated_rebecca(&temp)
        .env("TEMP", &temp_cache)
        .args([
            "clean",
            "--dry-run",
            "--no-progress",
            "--rule",
            "windows.user-temp",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Cleanup mode: dry-run"));
    assert!(stdout.contains("Target details:"));
}

#[test]
fn clean_dry_run_does_not_write_scan_cache_by_default_for_file_targets() {
    let temp = tempfile::tempdir().unwrap();
    let explorer = temp
        .path()
        .join("local")
        .join("Microsoft")
        .join("Windows")
        .join("Explorer");
    fs::create_dir_all(&explorer).unwrap();
    fs::write(explorer.join("thumbcache_96.db"), b"thumb").unwrap();

    let output = isolated::isolated_rebecca(&temp)
        .args([
            "clean",
            "--dry-run",
            "--json",
            "--rule",
            "windows.thumbnail-cache",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["summary"]["estimated_bytes"], 5);
    assert!(!temp.path().join("rebecca-cache").join("scan").exists());
}

#[test]
fn clean_dry_run_scan_cache_flag_writes_file_target_cache() {
    let temp = tempfile::tempdir().unwrap();
    let explorer = temp
        .path()
        .join("local")
        .join("Microsoft")
        .join("Windows")
        .join("Explorer");
    fs::create_dir_all(&explorer).unwrap();
    fs::write(explorer.join("thumbcache_96.db"), b"thumb").unwrap();

    let output = isolated::isolated_rebecca(&temp)
        .args([
            "clean",
            "--dry-run",
            "--json",
            "--scan-cache",
            "--rule",
            "windows.thumbnail-cache",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["summary"]["estimated_bytes"], 5);

    let scan_cache_dir = temp.path().join("rebecca-cache").join("scan");
    let cache_entries = fs::read_dir(scan_cache_dir).unwrap().count();
    assert_eq!(cache_entries, 1);
}

#[test]
fn clean_dry_run_scan_cache_flag_reuses_directory_target_cache() {
    let temp = tempfile::tempdir().unwrap();
    let edge_cache = temp
        .path()
        .join("local")
        .join("Microsoft")
        .join("Edge")
        .join("User Data")
        .join("Default")
        .join("Cache");
    fs::create_dir_all(&edge_cache).unwrap();
    fs::write(edge_cache.join("cache.bin"), b"edge").unwrap();

    let output = isolated::isolated_rebecca(&temp)
        .args([
            "clean",
            "--dry-run",
            "--json",
            "--scan-cache",
            "--rule",
            "windows.edge-cache",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["summary"]["estimated_bytes"], 4);

    let scan_cache_dir = temp.path().join("rebecca-cache").join("scan");
    let cache_files = fs::read_dir(scan_cache_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    assert_eq!(cache_files.len(), 1);

    let cache_file = &cache_files[0];
    let mut record: serde_json::Value =
        serde_json::from_slice(&fs::read(cache_file).unwrap()).unwrap();
    record["report"]["bytes_scanned"] = serde_json::json!(99);
    fs::write(cache_file, serde_json::to_vec_pretty(&record).unwrap()).unwrap();

    let output = isolated::isolated_rebecca(&temp)
        .args([
            "clean",
            "--dry-run",
            "--json",
            "--scan-cache",
            "--rule",
            "windows.edge-cache",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["summary"]["estimated_bytes"], 99);
    assert!(value.get("scan_cache").is_none());
    assert!(value["summary"].get("scan_cache").is_none());
}

#[test]
fn clean_dry_run_scan_cache_policy_expires_directory_records_from_config() {
    let temp = tempfile::tempdir().unwrap();
    let config_dir = temp.path().join("rebecca-config");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(
        config_dir.join("config.toml"),
        r#"
version = 1

[scan_cache]
directory_record_max_age_seconds = 1
"#,
    )
    .unwrap();
    let edge_cache = temp
        .path()
        .join("local")
        .join("Microsoft")
        .join("Edge")
        .join("User Data")
        .join("Default")
        .join("Cache");
    fs::create_dir_all(&edge_cache).unwrap();
    fs::write(edge_cache.join("cache.bin"), b"edge").unwrap();

    let output = isolated::isolated_rebecca(&temp)
        .args([
            "clean",
            "--dry-run",
            "--json",
            "--scan-cache",
            "--rule",
            "windows.edge-cache",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let scan_cache_dir = temp.path().join("rebecca-cache").join("scan");
    let cache_files = fs::read_dir(scan_cache_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    assert_eq!(cache_files.len(), 1);

    let cache_file = &cache_files[0];
    let mut record: serde_json::Value =
        serde_json::from_slice(&fs::read(cache_file).unwrap()).unwrap();
    let stale_written_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        .saturating_sub(2);
    record["report"]["bytes_scanned"] = serde_json::json!(99);
    record["written_at_unix_seconds"] = serde_json::json!(stale_written_at);
    fs::write(cache_file, serde_json::to_vec_pretty(&record).unwrap()).unwrap();

    let output = isolated::isolated_rebecca(&temp)
        .args([
            "clean",
            "--dry-run",
            "--json",
            "--scan-cache",
            "--rule",
            "windows.edge-cache",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["summary"]["estimated_bytes"], 4);
}

#[test]
fn clean_dry_run_scan_cache_reports_invalid_policy_config() {
    let temp = tempfile::tempdir().unwrap();
    let config_dir = temp.path().join("rebecca-config");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(
        config_dir.join("config.toml"),
        r#"
[scan_cache]
directory_record_max_age_seconds = 0
"#,
    )
    .unwrap();

    let output = isolated::isolated_rebecca(&temp)
        .args([
            "clean",
            "--dry-run",
            "--json",
            "--scan-cache",
            "--rule",
            "windows.edge-cache",
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());

    let stderr = common::support::stderr(&output);
    assert!(stderr.contains("config parse failed"));
    assert!(stderr.contains("scan_cache.directory_record_max_age_seconds must be at least 1"));
    assert!(stderr.contains("config.toml"));
}

#[test]
fn clean_human_output_summarizes_scan_cache_activity() {
    let temp = tempfile::tempdir().unwrap();
    let edge_cache = temp
        .path()
        .join("local")
        .join("Microsoft")
        .join("Edge")
        .join("User Data")
        .join("Default")
        .join("Cache");
    fs::create_dir_all(&edge_cache).unwrap();
    fs::write(edge_cache.join("cache.bin"), b"edge").unwrap();

    let output = isolated::isolated_rebecca(&temp)
        .args([
            "clean",
            "--dry-run",
            "--json",
            "--scan-cache",
            "--rule",
            "windows.edge-cache",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let output = isolated::isolated_rebecca(&temp)
        .args([
            "clean",
            "--dry-run",
            "--no-progress",
            "--scan-cache",
            "--rule",
            "windows.edge-cache",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Scan cache summary: 1 hit, 0 misses, 0 skipped writes"));
}

#[test]
fn clean_dry_run_json_expands_steam_rules_with_discovery_override() {
    for case in common::steam::STEAM_INSTALL_RULE_CASES {
        let temp = tempfile::tempdir().unwrap();
        let value = steam_dry_run_json_output(&temp, case);
        assert_eq!(value["summary"]["total_targets"], 1, "{}", case.rule_id);
        assert_eq!(value["summary"]["allowed_targets"], 1, "{}", case.rule_id);
        assert_eq!(value["summary"]["skipped_targets"], 0, "{}", case.rule_id);
        assert_eq!(
            value["summary"]["estimated_bytes"],
            case.bytes.len() as u64,
            "{}",
            case.rule_id
        );

        let targets = value["targets"].as_array().unwrap();
        assert_eq!(targets.len(), 1, "{}", case.rule_id);
        assert_eq!(targets[0]["rule_id"], case.rule_id);
        assert_eq!(targets[0]["status"], "allowed");
        assert_eq!(
            targets[0]["restore_hint"].as_str().unwrap(),
            case.expected_restore_hint.unwrap()
        );
        assert!(
            targets[0]["path"]
                .as_str()
                .unwrap()
                .ends_with(&case.relative_path.replace('/', r"\"))
        );
    }
}

#[test]
fn clean_dry_run_json_uses_install_root_when_libraryfolders_is_unreadable() {
    let temp = tempfile::tempdir().unwrap();
    let steam = temp.path().join("Steam");
    let steamapps = steam.join("steamapps");
    let httpcache = steam.join("appcache").join("httpcache");
    std::fs::create_dir_all(steamapps.join("libraryfolders.vdf")).unwrap();
    fs::create_dir_all(&httpcache).unwrap();
    fs::write(httpcache.join("cache.bin"), b"abcd").unwrap();

    let output = isolated::isolated_rebecca(&temp)
        .env("REBECCA_STEAM_DISCOVERY_PATH", &steam)
        .args([
            "clean",
            "--dry-run",
            "--json",
            "--rule",
            "windows.steam-install-cache",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["summary"]["total_targets"], 1);
    assert_eq!(value["summary"]["allowed_targets"], 1);
    assert_eq!(value["summary"]["skipped_targets"], 0);
    assert_eq!(value["summary"]["estimated_bytes"], 4);

    let targets = value["targets"].as_array().unwrap();
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0]["rule_id"], "windows.steam-install-cache");
    assert_eq!(targets[0]["status"], "allowed");
    assert!(
        targets[0]["path"]
            .as_str()
            .unwrap()
            .ends_with(r"appcache\httpcache")
    );
}

#[test]
fn clean_dry_run_json_allows_moderate_rules_with_opt_in() {
    let temp = tempfile::tempdir().unwrap();
    let roaming = temp.path().join("roaming");
    let npm_cache = roaming.join("npm-cache").join("_cacache");
    fs::create_dir_all(&npm_cache).unwrap();
    fs::write(npm_cache.join("index.bin"), b"abcd").unwrap();

    let output = isolated::isolated_rebecca(&temp)
        .env("APPDATA", &roaming)
        .args([
            "clean",
            "--dry-run",
            "--json",
            "--allow-moderate",
            "--rule",
            "windows.npm-cache",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["summary"]["total_targets"], 1);
    assert_eq!(value["summary"]["allowed_targets"], 1);
    assert_eq!(value["summary"]["skipped_targets"], 0);
    assert_eq!(value["summary"]["estimated_bytes"], 4);

    let targets = value["targets"].as_array().unwrap();
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0]["rule_id"], "windows.npm-cache");
    assert_eq!(targets[0]["status"], "allowed");
}

#[test]
fn clean_dry_run_json_accepts_allow_risky_flag() {
    let temp = tempfile::tempdir().unwrap();
    let roaming = temp.path().join("roaming");
    let npm_cache = roaming.join("npm-cache").join("_cacache");
    fs::create_dir_all(&npm_cache).unwrap();
    fs::write(npm_cache.join("index.bin"), b"abcd").unwrap();

    let output = isolated::isolated_rebecca(&temp)
        .env("APPDATA", &roaming)
        .args([
            "clean",
            "--dry-run",
            "--json",
            "--allow-risky",
            "--rule",
            "windows.npm-cache",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["summary"]["total_targets"], 1);
    assert_eq!(value["summary"]["allowed_targets"], 1);
    assert_eq!(value["summary"]["skipped_targets"], 0);
    assert_eq!(value["summary"]["estimated_bytes"], 4);
    assert_eq!(value["request"]["allow_risky"], true);

    let targets = value["targets"].as_array().unwrap();
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0]["rule_id"], "windows.npm-cache");
    assert_eq!(targets[0]["status"], "allowed");
}

#[test]
fn clean_unknown_rule_returns_clear_error() {
    let temp = tempfile::tempdir().unwrap();
    let output = isolated::isolated_rebecca(&temp)
        .args(["clean", "--dry-run", "--json", "--rule", "missing.rule"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(common::support::stderr(&output).contains("invalid rule id"));
}

#[test]
fn clean_unknown_category_returns_clear_error() {
    let temp = tempfile::tempdir().unwrap();
    let output = isolated::isolated_rebecca(&temp)
        .args(["clean", "--dry-run", "--json", "--category", "missing"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(common::support::stderr(&output).contains("invalid category"));
}

#[cfg(not(windows))]
#[test]
fn non_windows_execution_is_reported_as_unsupported() {
    let temp = tempfile::tempdir().unwrap();
    let output = isolated::isolated_rebecca(&temp)
        .env("TEMP", temp.path().join("temp"))
        .args(["clean", "--yes"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(common::support::stderr(&output).contains("Windows-only"));
}

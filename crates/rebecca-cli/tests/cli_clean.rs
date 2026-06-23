use std::fs;

mod support;
#[test]
fn clean_dry_run_json_builds_plan_without_deleting() {
    let temp = tempfile::tempdir().unwrap();
    let temp_cache = temp.path().join("temp");
    fs::create_dir_all(&temp_cache).unwrap();
    let file = temp_cache.join("cache.tmp");
    fs::write(&file, b"cache").unwrap();

    let output = support::isolated_rebecca(&temp)
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
        support::stderr(&output)
    );
    assert!(file.exists(), "dry-run must not delete files");

    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["request"]["mode"], "dry-run");
    assert!(value["summary"]["allowed_targets"].as_u64().unwrap() >= 1);
}

#[test]
fn clean_dry_run_json_deduplicates_overlapping_system_targets() {
    let temp = tempfile::tempdir().unwrap();
    let local = temp.path().join("local");
    let temp_cache = local.join("Temp");
    fs::create_dir_all(&temp_cache).unwrap();
    fs::write(temp_cache.join("cache.tmp"), b"cache").unwrap();

    let output = support::isolated_rebecca(&temp)
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
        support::stderr(&output)
    );

    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["summary"]["total_targets"], 2);
    assert_eq!(value["summary"]["allowed_targets"], 1);
    assert_eq!(value["summary"]["skipped_targets"], 1);
    assert_eq!(value["summary"]["estimated_bytes"], 5);
    assert!(
        value["targets"]
            .as_array()
            .unwrap()
            .iter()
            .any(|target| target["reason"] == "duplicate target path already covered")
    );
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

    let output = support::isolated_rebecca(&temp)
        .env("LOCALAPPDATA", &local)
        .args(["clean", "--dry-run", "--category", "browser"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        support::stderr(&output)
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

    let output = support::isolated_rebecca(&temp)
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
        support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Cleanup mode: dry-run"));
    assert!(stdout.contains("Target details:"));
}

#[test]
fn clean_dry_run_json_expands_steam_rule_with_discovery_override() {
    let temp = tempfile::tempdir().unwrap();
    let steam = temp.path().join("Steam");
    let appcache = steam.join("appcache");
    let librarycache = appcache.join("librarycache");
    fs::create_dir_all(&librarycache).unwrap();
    fs::write(librarycache.join("cache.bin"), b"abc").unwrap();

    let output = support::isolated_rebecca(&temp)
        .env("REBECCA_STEAM_DISCOVERY_PATH", &steam)
        .args([
            "clean",
            "--dry-run",
            "--json",
            "--rule",
            "windows.steam-install-library-cache",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        support::stderr(&output)
    );

    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["summary"]["total_targets"], 1);
    assert_eq!(value["summary"]["allowed_targets"], 1);
    assert_eq!(value["summary"]["skipped_targets"], 0);
    assert_eq!(value["summary"]["estimated_bytes"], 3);

    let targets = value["targets"].as_array().unwrap();
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0]["rule_id"], "windows.steam-install-library-cache");
    assert_eq!(targets[0]["status"], "allowed");
    assert!(
        targets[0]["path"]
            .as_str()
            .unwrap()
            .ends_with(r"appcache\librarycache")
    );
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

    let output = support::isolated_rebecca(&temp)
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
        support::stderr(&output)
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

    let output = support::isolated_rebecca(&temp)
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
        support::stderr(&output)
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

    let output = support::isolated_rebecca(&temp)
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
        support::stderr(&output)
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
    let output = support::isolated_rebecca(&temp)
        .args(["clean", "--dry-run", "--json", "--rule", "missing.rule"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(support::stderr(&output).contains("invalid rule id"));
}

#[cfg(not(windows))]
#[test]
fn non_windows_execution_is_reported_as_unsupported() {
    let temp = tempfile::tempdir().unwrap();
    let output = support::isolated_rebecca(&temp)
        .env("TEMP", temp.path().join("temp"))
        .args(["clean", "--yes"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(support::stderr(&output).contains("Windows-only"));
}

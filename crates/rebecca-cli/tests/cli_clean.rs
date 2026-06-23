use std::fs;
use std::process::Command;

#[test]
fn clean_dry_run_json_builds_plan_without_deleting() {
    let temp = tempfile::tempdir().unwrap();
    let temp_cache = temp.path().join("temp");
    fs::create_dir_all(&temp_cache).unwrap();
    let file = temp_cache.join("cache.tmp");
    fs::write(&file, b"cache").unwrap();

    let output = isolated_rebecca(&temp)
        .env("TEMP", &temp_cache)
        .env("LOCALAPPDATA", temp.path().join("local"))
        .env("APPDATA", temp.path().join("roaming"))
        .args(["clean", "--dry-run", "--json", "--category", "system"])
        .output()
        .unwrap();

    assert!(output.status.success(), "stderr: {}", stderr(&output));
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

    let output = isolated_rebecca(&temp)
        .env("TEMP", &temp_cache)
        .env("LOCALAPPDATA", &local)
        .env("APPDATA", temp.path().join("roaming"))
        .args(["clean", "--dry-run", "--json", "--category", "system"])
        .output()
        .unwrap();

    assert!(output.status.success(), "stderr: {}", stderr(&output));

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
fn clean_unknown_rule_returns_clear_error() {
    let temp = tempfile::tempdir().unwrap();
    let output = isolated_rebecca(&temp)
        .args(["clean", "--dry-run", "--json", "--rule", "missing.rule"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(stderr(&output).contains("invalid rule id"));
}

#[cfg(not(windows))]
#[test]
fn non_windows_execution_is_reported_as_unsupported() {
    let temp = tempfile::tempdir().unwrap();
    let output = isolated_rebecca(&temp)
        .env("TEMP", temp.path().join("temp"))
        .args(["clean", "--yes"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(stderr(&output).contains("Windows-only"));
}

fn isolated_rebecca(temp: &tempfile::TempDir) -> Command {
    let roaming = temp.path().join("roaming");
    let local = temp.path().join("local");
    let config = temp.path().join("config");
    let data = temp.path().join("data");
    let cache = temp.path().join("cache");
    let temp_dir = temp.path().join("temp");

    for path in [&roaming, &local, &config, &data, &cache, &temp_dir] {
        std::fs::create_dir_all(path).unwrap();
    }

    let mut command = Command::new(env!("CARGO_BIN_EXE_rebecca"));
    command
        .env("HOME", temp.path())
        .env("USERPROFILE", temp.path())
        .env("APPDATA", roaming)
        .env("LOCALAPPDATA", local)
        .env("XDG_CONFIG_HOME", config)
        .env("XDG_DATA_HOME", data)
        .env("XDG_CACHE_HOME", cache)
        .env("TEMP", temp_dir)
        .env("REBECCA_CONFIG_DIR", temp.path().join("rebecca-config"))
        .env("REBECCA_STATE_DIR", temp.path().join("rebecca-state"))
        .env("REBECCA_CACHE_DIR", temp.path().join("rebecca-cache"))
        .env(
            "REBECCA_HISTORY_FILE",
            temp.path().join("rebecca-state").join("history.jsonl"),
        );
    command
}

fn stderr(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

use std::process::Command;

#[test]
fn config_paths_json_is_parseable() {
    let temp = tempfile::tempdir().unwrap();
    let output = isolated_rebecca(&temp)
        .args(["config", "paths", "--json"])
        .output()
        .unwrap();

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

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
}

#[test]
fn doctor_permissions_prints_permission_label() {
    let output = Command::new(env!("CARGO_BIN_EXE_rebecca"))
        .args(["doctor", "permissions"])
        .output()
        .unwrap();

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Privilege level:"));
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

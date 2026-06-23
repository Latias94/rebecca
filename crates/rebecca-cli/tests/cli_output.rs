#[path = "common/command.rs"]
mod command;
#[path = "common/isolated.rs"]
mod isolated;
#[path = "common/support.rs"]
mod support;

#[test]
fn config_paths_json_is_parseable() {
    let temp = tempfile::tempdir().unwrap();
    let output = isolated::isolated_rebecca(&temp)
        .args(["config", "paths", "--json"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        support::stderr(&output)
    );
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
    let output = command::rebecca()
        .args(["doctor", "permissions"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        support::stderr(&output)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Privilege level:"));
}

#[test]
fn doctor_steam_prints_discovery_status() {
    let temp = tempfile::tempdir().unwrap();
    let output = isolated::isolated_rebecca(&temp)
        .args(["doctor", "steam"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        support::stderr(&output)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Steam install:"));
}

#[test]
fn doctor_steam_prints_library_list_when_discovered() {
    let temp = tempfile::tempdir().unwrap();
    let steam = temp.path().join("Steam");
    let steamapps = steam.join("steamapps");
    std::fs::create_dir_all(&steamapps).unwrap();
    std::fs::write(
        steamapps.join("libraryfolders.vdf"),
        r#"
"libraryfolders"
{
    "0"
    {
        "path"      "D:\\SteamLibrary"
    }
}
"#,
    )
    .unwrap();

    let output = isolated::isolated_rebecca(&temp)
        .env("REBECCA_STEAM_DISCOVERY_PATH", &steam)
        .env("LOCALAPPDATA", temp.path().join("local"))
        .args(["doctor", "steam"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        support::stderr(&output)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Steam libraries:"));
    assert!(stdout.contains(r"D:\SteamLibrary"));
}

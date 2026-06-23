use std::process::Command;

#[path = "command.rs"]
mod command;

pub fn isolated_rebecca(temp: &tempfile::TempDir) -> Command {
    let roaming = temp.path().join("roaming");
    let local = temp.path().join("local");
    let config = temp.path().join("config");
    let data = temp.path().join("data");
    let cache = temp.path().join("cache");
    let temp_dir = temp.path().join("temp");

    for path in [&roaming, &local, &config, &data, &cache, &temp_dir] {
        std::fs::create_dir_all(path).unwrap();
    }

    let mut command = command::rebecca();
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

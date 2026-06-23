use std::process::Command;

#[test]
fn scan_human_output_uses_lowercase_safety_labels() {
    let output = rebecca()
        .args(["scan", "--category", "development"])
        .output()
        .unwrap();

    assert!(output.status.success(), "stderr: {}", stderr(&output));

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("  - windows.npm-cache [moderate] npm cache"));
}

#[test]
fn clean_human_output_uses_lowercase_status_labels() {
    let temp = tempfile::tempdir().unwrap();
    let temp_cache = temp.path().join("temp");
    std::fs::create_dir_all(&temp_cache).unwrap();
    std::fs::write(temp_cache.join("cache.tmp"), b"cache").unwrap();

    let output = rebecca()
        .env("TEMP", &temp_cache)
        .args(["clean", "--dry-run", "--rule", "windows.user-temp"])
        .output()
        .unwrap();

    assert!(output.status.success(), "stderr: {}", stderr(&output));

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Cleanup mode: dry-run"));
    assert!(stdout.contains("allowed (2)"));
}

fn rebecca() -> Command {
    Command::new(env!("CARGO_BIN_EXE_rebecca"))
}

fn stderr(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

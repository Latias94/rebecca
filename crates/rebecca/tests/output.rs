mod common;
#[path = "common/isolated.rs"]
mod isolated;

#[test]
fn scan_human_output_uses_lowercase_safety_labels() {
    let output = common::command::rebecca()
        .args(["scan", "--category", "development"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("  - windows.npm-cache [moderate] npm cache"));
}

#[test]
fn clean_human_output_uses_lowercase_status_labels() {
    let temp = tempfile::tempdir().unwrap();
    let local = temp.path().join("local");
    let temp_cache = local.join("Temp");

    std::fs::create_dir_all(&temp_cache).unwrap();
    std::fs::write(temp_cache.join("cache.tmp"), b"cache").unwrap();

    let output = isolated::isolated_rebecca(&temp)
        .env("TEMP", &temp_cache)
        .args(["clean", "--dry-run", "--rule", "windows.user-temp"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Cleanup mode: dry-run"));
    assert!(stdout.contains("allowed (1)"));
    assert!(stdout.contains("skipped (1)"));
    assert!(stdout.contains("[restore: Temporary files owned by the current user.]"));
}

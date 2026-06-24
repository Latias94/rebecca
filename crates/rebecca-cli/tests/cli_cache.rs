use std::fs;

mod common;
#[path = "common/isolated.rs"]
mod isolated;

#[test]
fn cache_purge_json_defaults_to_preview_without_deleting() {
    let temp = tempfile::tempdir().unwrap();
    let cache_dir = temp.path().join("rebecca-cache");
    fs::create_dir_all(cache_dir.join("nested")).unwrap();
    fs::write(cache_dir.join("cache.bin"), b"abc").unwrap();
    fs::write(cache_dir.join("nested").join("nested.bin"), b"de").unwrap();

    let output = isolated::isolated_rebecca(&temp)
        .args(["cache", "purge", "--json"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );
    assert!(cache_dir.join("cache.bin").exists());

    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["mode"], "dry-run");
    assert_eq!(value["deleted"], false);
    assert_eq!(value["cache_dir_lifecycle"], "rebuildable-cache");
    assert_eq!(value["cache_dir_retention"], "rebuildable");
    assert_eq!(value["cache_dir_exists"], true);
    assert_eq!(value["preserves_cache_dir"], true);
    assert_eq!(value["summary"]["total_entries"], 2);
    assert_eq!(value["summary"]["would_delete_entries"], 2);
    assert_eq!(value["summary"]["deleted_entries"], 0);
    assert_eq!(value["summary"]["estimated_bytes"], 5);
    assert_eq!(value["summary"]["reclaimed_bytes"], 0);
    assert!(
        value["summary"]["issue_matrix"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn cache_purge_human_output_reports_scope_and_status_counts() {
    let temp = tempfile::tempdir().unwrap();
    let cache_dir = temp.path().join("rebecca-cache");
    fs::create_dir_all(cache_dir.join("nested")).unwrap();
    fs::write(cache_dir.join("cache.bin"), b"abc").unwrap();
    fs::write(cache_dir.join("nested").join("nested.bin"), b"de").unwrap();

    let output = isolated::isolated_rebecca(&temp)
        .args(["cache", "purge"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Lifecycle: rebuildable cache (rebuildable)"));
    assert!(stdout.contains("Cache directory exists: yes"));
    assert!(stdout.contains("Preserves cache directory: yes"));
    assert!(stdout.contains("Entry status: 2 would delete, 0 deleted, 0 skipped, 0 failed"));
    assert!(stdout.contains("Run with --yes to purge these rebuildable cache entries."));
}

#[test]
fn cache_purge_yes_deletes_direct_contents_but_keeps_cache_dir() {
    let temp = tempfile::tempdir().unwrap();
    let cache_dir = temp.path().join("rebecca-cache");
    fs::create_dir_all(cache_dir.join("nested")).unwrap();
    fs::write(cache_dir.join("cache.bin"), b"abc").unwrap();
    fs::write(cache_dir.join("nested").join("nested.bin"), b"de").unwrap();

    let output = isolated::isolated_rebecca(&temp)
        .args(["cache", "purge", "--yes", "--json"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );
    assert!(cache_dir.exists());
    assert_eq!(fs::read_dir(&cache_dir).unwrap().count(), 0);

    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["mode"], "delete");
    assert_eq!(value["deleted"], true);
    assert_eq!(value["cache_dir_exists"], true);
    assert_eq!(value["preserves_cache_dir"], true);
    assert_eq!(value["summary"]["deleted_entries"], 2);
    assert_eq!(value["summary"]["reclaimed_bytes"], 5);
    assert!(
        value["summary"]["issue_matrix"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn cache_purge_rejects_overlap_with_state_dir() {
    let temp = tempfile::tempdir().unwrap();
    let state_dir = temp.path().join("rebecca-state");
    fs::create_dir_all(&state_dir).unwrap();

    let output = isolated::isolated_rebecca(&temp)
        .env("REBECCA_CACHE_DIR", &state_dir)
        .args(["cache", "purge", "--json"])
        .output()
        .unwrap();

    assert!(!output.status.success());

    let stderr = common::support::stderr(&output);
    assert!(stderr.contains("overlaps preserved"));
    assert!(stderr.contains("State dir"));
}

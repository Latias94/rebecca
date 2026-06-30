use std::fs;
use std::path::{Path, PathBuf};

mod common;
#[path = "common/isolated.rs"]
mod isolated;

fn write_fixture_file(path: impl AsRef<Path>, bytes: &[u8]) {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, bytes).unwrap();
}

fn write_node_project(dir: impl AsRef<Path>) {
    write_fixture_file(dir.as_ref().join("package.json"), b"{}");
}

fn write_rust_project(dir: impl AsRef<Path>) {
    write_fixture_file(dir.as_ref().join("Cargo.toml"), b"[package]");
}

#[test]
fn inspect_help_lists_space_and_artifacts_subcommands() {
    let output = common::command::rebecca()
        .args(["inspect", "--help"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("space"));
    assert!(stdout.contains("artifacts"));
}

#[test]
fn inspect_space_json_reports_top_entries_and_diagnostics() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    let missing = temp.path().join("missing");
    write_fixture_file(root.join("zeta").join("data.bin"), b"abc");
    write_fixture_file(root.join("alpha").join("data.bin"), b"abc");
    write_fixture_file(root.join("small.txt"), b"x");

    let output = isolated::isolated_rebecca(&temp)
        .args([
            "inspect",
            "space",
            "--format",
            "json",
            "--root",
            root.to_str().unwrap(),
            "--root",
            missing.to_str().unwrap(),
            "--top",
            "2",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let envelope = common::support::api_envelope_v2(&output.stdout);
    assert_eq!(envelope["command"], "inspect space");
    assert_eq!(envelope["payload_kind"], "inspect-space");

    let value = &envelope["data"];
    assert_eq!(value["totals"]["estimated_bytes"], 7);
    assert_eq!(value["totals"]["files"], 3);
    assert_eq!(value["top_entries"].as_array().unwrap().len(), 2);
    assert_eq!(
        PathBuf::from(value["top_entries"][0]["path"].as_str().unwrap()),
        root.join("alpha")
    );
    assert_eq!(
        PathBuf::from(value["top_entries"][1]["path"].as_str().unwrap()),
        root.join("zeta")
    );
    assert_eq!(value["diagnostics"][0]["kind"], "root-missing");
}

#[test]
fn inspect_space_ndjson_uses_v2_completed_event() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    write_fixture_file(root.join("entry.bin"), b"abc");

    let output = isolated::isolated_rebecca(&temp)
        .args([
            "inspect",
            "space",
            "--format",
            "ndjson",
            "--root",
            root.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let events = stdout
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(events.len(), 1);
    let completed = events.first().unwrap();
    assert_eq!(completed["api_version"], "rebecca.cli.v2");
    assert_eq!(completed["event_kind"], "completed");
    assert_eq!(completed["command"], "inspect space");
    assert_eq!(completed["payload_kind"], "inspect-space");
    assert_eq!(completed["data"]["totals"]["estimated_bytes"], 3);
}

#[test]
fn inspect_artifacts_json_reports_read_only_project_artifact_insight() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    let node_modules_file = workspace.join("app").join("node_modules").join("pkg.bin");
    let target_file = workspace
        .join("app")
        .join("target")
        .join("debug")
        .join("app.bin");
    write_fixture_file(&node_modules_file, b"abc");
    write_fixture_file(&target_file, b"rust");
    write_node_project(workspace.join("app"));
    write_rust_project(workspace.join("app"));

    let output = isolated::isolated_rebecca(&temp)
        .args([
            "inspect",
            "artifacts",
            "--format",
            "json",
            "--no-progress",
            "--root",
            workspace.to_str().unwrap(),
            "--min-age-days",
            "0",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );
    assert!(node_modules_file.exists());
    assert!(target_file.exists());
    assert!(
        !temp
            .path()
            .join("rebecca-state")
            .join("history.jsonl")
            .exists()
    );

    let envelope = common::support::api_envelope_v2(&output.stdout);
    assert_eq!(envelope["command"], "inspect artifacts");
    assert_eq!(envelope["payload_kind"], "inspect-artifacts");

    let value = &envelope["data"];
    assert_eq!(value["summary"]["total_targets"], 2);
    assert_eq!(value["summary"]["estimated_bytes"], 7);
    assert_eq!(value["top_targets"][0]["artifact"], "target");
    assert_eq!(value["top_targets"][1]["artifact"], "node_modules");
}

#[test]
fn purge_inspect_compatibility_matches_inspect_artifacts_data() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    write_fixture_file(
        workspace.join("app").join("node_modules").join("pkg.bin"),
        b"abc",
    );
    write_node_project(workspace.join("app"));

    let inspect_output = isolated::isolated_rebecca(&temp)
        .args([
            "inspect",
            "artifacts",
            "--format",
            "json",
            "--no-progress",
            "--root",
            workspace.to_str().unwrap(),
            "--min-age-days",
            "0",
        ])
        .output()
        .unwrap();
    let purge_output = isolated::isolated_rebecca(&temp)
        .args([
            "purge",
            "inspect",
            "--format",
            "json",
            "--no-progress",
            "--root",
            workspace.to_str().unwrap(),
            "--min-age-days",
            "0",
        ])
        .output()
        .unwrap();

    assert!(
        inspect_output.status.success(),
        "stderr: {}",
        common::support::stderr(&inspect_output)
    );
    assert!(
        purge_output.status.success(),
        "stderr: {}",
        common::support::stderr(&purge_output)
    );

    let inspect = common::support::api_envelope_v2(&inspect_output.stdout);
    let purge = common::support::api_envelope_v2(&purge_output.stdout);
    assert_eq!(inspect["payload_kind"], "inspect-artifacts");
    assert_eq!(purge["payload_kind"], "inspect-artifacts");
    assert_eq!(inspect["data"], purge["data"]);
}

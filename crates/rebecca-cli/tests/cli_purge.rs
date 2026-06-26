use std::fs;
use std::path::{Path, PathBuf};

mod common;
#[path = "common/isolated.rs"]
mod isolated;

const CACHEDIR_TAG_SIGNATURE: &str = "Signature: 8a477f597d28d172789f06886806bc55";

fn write_fixture_file(path: impl AsRef<Path>, bytes: &[u8]) {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, bytes).unwrap();
}

fn write_cachedir_tag(dir: impl AsRef<Path>) {
    write_fixture_file(dir.as_ref().join("CACHEDIR.TAG"), &cachedir_tag_bytes());
}

fn cachedir_tag_bytes() -> Vec<u8> {
    format!("{CACHEDIR_TAG_SIGNATURE}\n# cache directory\n").into_bytes()
}

#[test]
fn purge_help_shows_project_artifact_options() {
    let output = common::command::rebecca()
        .args(["purge", "--help"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--root"));
    assert!(stdout.contains("--max-depth"));
    assert!(stdout.contains("--min-age-days"));
    assert!(stdout.contains("--exclude"));
}

#[test]
fn purge_json_builds_project_artifact_plan_without_deleting() {
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

    let output = isolated::isolated_rebecca(&temp)
        .args([
            "purge",
            "--json",
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
    assert!(
        node_modules_file.exists(),
        "purge should preview by default"
    );
    assert!(target_file.exists(), "purge should preview by default");

    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["request"]["workflow"], "project-artifacts");
    assert_eq!(value["request"]["mode"], "dry-run");
    assert_eq!(value["request"]["project_artifact_min_age_days"], 0);
    assert_eq!(
        PathBuf::from(
            value["request"]["project_artifact_roots"][0]
                .as_str()
                .unwrap()
        ),
        workspace
    );
    assert_eq!(value["summary"]["allowed_targets"], 2);
    assert_eq!(value["summary"]["estimated_bytes"], 7);

    let targets = value["targets"].as_array().unwrap();
    assert!(targets.iter().any(|target| {
        target["rule_id"] == "windows.project-artifact-node-modules"
            && PathBuf::from(target["path"].as_str().unwrap())
                .ends_with(Path::new("app").join("node_modules"))
    }));
    assert!(targets.iter().any(|target| {
        target["rule_id"] == "windows.project-artifact-target"
            && PathBuf::from(target["path"].as_str().unwrap())
                .ends_with(Path::new("app").join("target"))
    }));
}

#[test]
fn purge_json_skips_recent_artifacts_by_default() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    write_fixture_file(
        workspace.join("app").join("node_modules").join("pkg.bin"),
        b"abc",
    );

    let output = isolated::isolated_rebecca(&temp)
        .args([
            "purge",
            "--json",
            "--no-progress",
            "--root",
            workspace.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["summary"]["allowed_targets"], 0);
    assert_eq!(value["summary"]["skipped_targets"], 1);
    assert_eq!(value["summary"]["estimated_bytes"], 0);

    let target = &value["targets"].as_array().unwrap()[0];
    assert_eq!(target["status"], "skipped");
    assert_eq!(target["reason_code"], "project-artifact-recently-modified");
    assert!(
        target["reason"]
            .as_str()
            .unwrap()
            .contains("modified within the last 7 days")
    );
}

#[test]
fn purge_json_reports_cachedir_tag_artifacts() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    let cache = workspace.join("app").join("tool-cache");
    write_fixture_file(cache.join("entry.bin"), b"abc");
    write_cachedir_tag(&cache);

    let output = isolated::isolated_rebecca(&temp)
        .args([
            "purge",
            "--json",
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

    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["summary"]["allowed_targets"], 1);
    assert_eq!(
        value["summary"]["estimated_bytes"],
        3 + cachedir_tag_bytes().len() as u64
    );

    let target = &value["targets"].as_array().unwrap()[0];
    assert_eq!(target["rule_id"], "windows.project-artifact-cachedir-tag");
    assert_eq!(target["status"], "allowed");
    assert!(
        PathBuf::from(target["path"].as_str().unwrap())
            .ends_with(Path::new("app").join("tool-cache"))
    );
}

#[test]
fn purge_json_honors_exclude_flag() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    let node_modules = workspace.join("app").join("node_modules");
    write_fixture_file(node_modules.join("pkg.bin"), b"abc");

    let output = isolated::isolated_rebecca(&temp)
        .args([
            "purge",
            "--json",
            "--no-progress",
            "--root",
            workspace.to_str().unwrap(),
            "--exclude",
            node_modules.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["summary"]["allowed_targets"], 0);
    assert_eq!(value["summary"]["blocked_targets"], 1);

    let blocked = &value["targets"].as_array().unwrap()[0];
    assert_eq!(blocked["rule_id"], "windows.project-artifact-node-modules");
    assert_eq!(blocked["status"], "blocked");
    assert_eq!(blocked["reason_code"], "safety-policy-blocked");
    assert!(
        blocked["reason"]
            .as_str()
            .unwrap()
            .contains("user-protected path")
    );
}

#[test]
fn purge_rejects_missing_root() {
    let temp = tempfile::tempdir().unwrap();
    let missing = temp.path().join("missing");

    let output = isolated::isolated_rebecca(&temp)
        .args(["purge", "--json", "--root", missing.to_str().unwrap()])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = common::support::stderr(&output);
    assert!(stderr.contains("purge root"));
    assert!(stderr.contains("not accessible"));
}

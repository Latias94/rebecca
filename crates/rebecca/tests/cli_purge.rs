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

fn write_node_project(dir: impl AsRef<Path>) {
    write_fixture_file(dir.as_ref().join("package.json"), b"{}");
}

fn write_rust_project(dir: impl AsRef<Path>) {
    write_fixture_file(dir.as_ref().join("Cargo.toml"), b"[package]");
}

fn write_config(temp: &tempfile::TempDir, contents: impl AsRef<str>) {
    let config_dir = temp.path().join("rebecca-config");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(config_dir.join("config.toml"), contents.as_ref()).unwrap();
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
    assert!(stdout.contains("--reclaim-limit-bytes"));
    assert!(stdout.contains("--artifact"));
    assert!(stdout.contains("--list-artifacts"));
    assert!(stdout.contains("--exclude"));
    assert!(stdout.contains("inspect"));
}

#[test]
fn purge_inspect_help_rejects_yes_option() {
    let output = common::command::rebecca()
        .args(["purge", "inspect", "--help"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--root"));
    assert!(stdout.contains("--scan-cache"));
    assert!(!stdout.contains("--yes"));

    let output = common::command::rebecca()
        .args(["purge", "inspect", "--yes"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(common::support::stderr(&output).contains("unexpected argument '--yes'"));
}

#[test]
fn purge_list_artifacts_human_reports_supported_selectors_without_loading_config() {
    let temp = tempfile::tempdir().unwrap();
    write_config(&temp, "[purge\n");

    let output = isolated::isolated_rebecca(&temp)
        .args(["purge", "--list-artifacts"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Supported project artifacts:"));
    assert!(stdout.contains("node_modules"));
    assert!(stdout.contains("node-modules"));
    assert!(stdout.contains("windows.project-artifact-node-modules"));
    assert!(stdout.contains("CACHEDIR.TAG"));
    assert!(stdout.contains("dotnet-bin"));
    assert!(stdout.contains("composer-vendor"));
}

#[test]
fn purge_list_artifacts_json_reports_machine_readable_catalog() {
    let output = common::command::rebecca()
        .args(["purge", "--list-artifacts", "--format", "json"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let envelope = common::support::api_envelope(&output.stdout);
    assert_eq!(envelope["command"], "purge");
    assert_eq!(envelope["payload_kind"], "project-artifact-catalog");

    let value: serde_json::Value = envelope["data"].clone();
    let artifacts = value.as_array().unwrap();
    assert!(artifacts.iter().any(|artifact| {
        artifact["artifact"] == "node_modules"
            && artifact["aliases"] == serde_json::json!(["node-modules"])
            && artifact["rule_id"] == "windows.project-artifact-node-modules"
            && artifact["rule_suffix"] == "node-modules"
            && artifact["default_min_age_days"] == 7
            && artifact["trim_eligible"] == true
            && artifact["deletion_style"] == "delete-whole-path"
            && artifact["ranking"] == "heavy-dependency-tree"
    }));
    assert!(artifacts.iter().any(|artifact| {
        artifact["artifact"] == "CACHEDIR.TAG"
            && artifact["rule_id"] == "windows.project-artifact-cachedir-tag"
    }));
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
    write_node_project(workspace.join("app"));
    write_rust_project(workspace.join("app"));

    let output = isolated::isolated_rebecca(&temp)
        .args([
            "purge",
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
    assert!(
        node_modules_file.exists(),
        "purge should preview by default"
    );
    assert!(target_file.exists(), "purge should preview by default");

    let envelope = common::support::api_envelope(&output.stdout);
    assert_eq!(envelope["command"], "purge");
    assert_eq!(envelope["payload_kind"], "project-artifact-cleanup-plan");

    let value: serde_json::Value = envelope["data"].clone();
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
    let node_modules_target = targets
        .iter()
        .find(|target| {
            target["rule_id"] == "windows.project-artifact-node-modules"
                && PathBuf::from(target["path"].as_str().unwrap())
                    .ends_with(Path::new("app").join("node_modules"))
        })
        .unwrap();
    assert_eq!(
        node_modules_target["project_artifact"]["matched_context"],
        "node-project"
    );
    assert!(
        PathBuf::from(
            node_modules_target["project_artifact"]["project_root"]
                .as_str()
                .unwrap()
        )
        .ends_with("app")
    );
    assert!(
        PathBuf::from(
            node_modules_target["project_artifact"]["project_anchor"]
                .as_str()
                .unwrap()
        )
        .ends_with(Path::new("app").join("package.json"))
    );

    let target_artifact = targets
        .iter()
        .find(|target| {
            target["rule_id"] == "windows.project-artifact-target"
                && PathBuf::from(target["path"].as_str().unwrap())
                    .ends_with(Path::new("app").join("target"))
        })
        .unwrap();
    assert_eq!(
        target_artifact["project_artifact"]["matched_context"],
        "target-project"
    );
    assert!(
        PathBuf::from(
            target_artifact["project_artifact"]["project_anchor"]
                .as_str()
                .unwrap()
        )
        .ends_with(Path::new("app").join("Cargo.toml"))
    );
}

#[test]
fn purge_human_output_groups_project_artifacts_by_project_path() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    write_fixture_file(
        workspace.join("app").join("node_modules").join("pkg.bin"),
        b"abc",
    );
    write_node_project(workspace.join("app"));
    write_fixture_file(
        workspace
            .join("app")
            .join("target")
            .join("debug")
            .join("app.bin"),
        b"rust",
    );
    write_rust_project(workspace.join("app"));

    let output = isolated::isolated_rebecca(&temp)
        .args([
            "purge",
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

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Workflow: Project artifacts"));
    assert!(stdout.contains("Project artifact summary:"));
    assert!(stdout.contains("- target: 1 target, 4 bytes (4 B) [1 allowed]"));
    assert!(stdout.contains("- node_modules: 1 target, 3 bytes (3 B) [1 allowed]"));
    assert!(stdout.contains("Largest project artifact targets:"));
    let largest_section = stdout
        .split("Largest project artifact targets:")
        .nth(1)
        .expect("largest project artifact section should be present")
        .split("Recently modified artifacts:")
        .next()
        .unwrap()
        .split("Project artifact details:")
        .next()
        .unwrap();
    let target_position = largest_section
        .find("- target [allowed]")
        .expect("target artifact should be listed in largest section");
    let node_modules_position = largest_section
        .find("- node_modules [allowed]")
        .expect("node_modules artifact should be listed in largest section");
    assert!(
        target_position < node_modules_position,
        "largest project artifacts should be sorted by estimated bytes"
    );
    assert!(stdout.contains("Project artifact details:"));
    assert!(stdout.contains(&workspace.join("app").display().to_string()));
    assert!(stdout.contains("- node_modules [allowed]"));
    assert!(stdout.contains("- target [allowed]"));
}

#[test]
fn purge_ndjson_uses_purge_command_identity() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    write_fixture_file(
        workspace
            .join("app")
            .join("target")
            .join("debug")
            .join("app.bin"),
        b"rust",
    );
    write_rust_project(workspace.join("app"));

    let output = isolated::isolated_rebecca(&temp)
        .args([
            "purge",
            "--format",
            "ndjson",
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

    let stdout = String::from_utf8_lossy(&output.stdout);
    let events = stdout
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();

    assert_eq!(events.first().unwrap()["event_kind"], "started");
    assert_eq!(events.last().unwrap()["event_kind"], "completed");
    assert!(events.iter().all(|event| event["command"] == "purge"));
    assert_eq!(
        events.last().unwrap()["payload_kind"],
        "project-artifact-cleanup-plan"
    );
}

#[test]
fn purge_inspect_json_returns_read_only_project_artifact_insight() {
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
    assert_eq!(envelope["command"], "purge inspect");
    assert_eq!(envelope["payload_kind"], "inspect-artifacts");

    let value = &envelope["data"];
    assert_eq!(value["summary"]["total_targets"], 2);
    assert_eq!(value["summary"]["estimated_bytes"], 7);
    assert_eq!(
        PathBuf::from(value["roots"][0].as_str().unwrap()),
        workspace
    );

    let top_targets = value["top_targets"].as_array().unwrap();
    assert_eq!(top_targets.len(), 2);
    assert_eq!(top_targets[0]["artifact"], "target");
    assert_eq!(top_targets[0]["estimated_bytes"], 4);
    assert_eq!(top_targets[0]["estimate_source"], "fresh-scan");
    assert_eq!(top_targets[1]["artifact"], "node_modules");

    let artifact_totals = value["totals_by_artifact"].as_array().unwrap();
    assert!(artifact_totals.iter().any(|total| {
        total["label"] == "node_modules" && total["targets"] == 1 && total["estimated_bytes"] == 3
    }));
    assert!(artifact_totals.iter().any(|total| {
        total["label"] == "target" && total["targets"] == 1 && total["estimated_bytes"] == 4
    }));
}

#[test]
fn purge_inspect_honors_filters_depth_exclude_and_configured_roots() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    let node_modules = workspace.join("app").join("node_modules");
    let target = workspace.join("app").join("level1").join("target");
    write_fixture_file(node_modules.join("pkg.bin"), b"abc");
    write_node_project(workspace.join("app"));
    write_fixture_file(target.join("debug").join("app.bin"), b"rust");
    write_rust_project(workspace.join("app").join("level1"));
    write_config(
        &temp,
        format!(
            r#"
[purge]
roots = ['{}']
max_depth = 1
min_age_days = 0
"#,
            workspace.display()
        ),
    );

    let output = isolated::isolated_rebecca(&temp)
        .args([
            "purge",
            "inspect",
            "--format",
            "json",
            "--no-progress",
            "--max-depth",
            "3",
            "--artifact",
            "target",
            "--exclude",
            target.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let value: serde_json::Value = common::support::api_data_v2(&output.stdout);
    assert_eq!(value["summary"]["total_targets"], 1);
    assert_eq!(value["summary"]["allowed_targets"], 0);
    assert_eq!(value["summary"]["blocked_targets"], 1);

    let top_targets = value["top_targets"].as_array().unwrap();
    assert_eq!(top_targets.len(), 1);
    assert_eq!(top_targets[0]["artifact"], "target");
    assert_eq!(top_targets[0]["status"], "blocked");
    assert!(
        top_targets[0]["reason"]
            .as_str()
            .unwrap()
            .contains("user-protected path")
    );
}

#[test]
fn purge_inspect_human_sorts_top_artifacts_and_reports_diagnostics() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    let missing = temp.path().join("missing-workspace");
    write_fixture_file(
        workspace.join("app").join("node_modules").join("pkg.bin"),
        b"abc",
    );
    write_node_project(workspace.join("app"));
    write_fixture_file(
        workspace
            .join("app")
            .join("target")
            .join("debug")
            .join("app.bin"),
        b"rust",
    );
    write_rust_project(workspace.join("app"));
    write_config(
        &temp,
        format!(
            r#"
[purge]
roots = ['{}', '{}']
max_depth = 3
min_age_days = 0
"#,
            workspace.display(),
            missing.display()
        ),
    );

    let output = isolated::isolated_rebecca(&temp)
        .args(["purge", "inspect", "--no-progress"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Project artifact insight"));
    assert!(stdout.contains("Discovery diagnostics:"));
    assert!(stdout.contains("root-missing"));
    let target_position = stdout
        .find("  - target [allowed] 4 bytes")
        .expect("target should appear in top artifacts");
    let node_modules_position = stdout
        .find("  - node_modules [allowed] 3 bytes")
        .expect("node_modules should appear in top artifacts");
    assert!(target_position < node_modules_position);
}

#[test]
fn purge_inspect_ndjson_uses_read_only_insight_payload() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    write_fixture_file(
        workspace.join("app").join("node_modules").join("pkg.bin"),
        b"abc",
    );
    write_node_project(workspace.join("app"));

    let output = isolated::isolated_rebecca(&temp)
        .args([
            "purge",
            "inspect",
            "--format",
            "ndjson",
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

    let stdout = String::from_utf8_lossy(&output.stdout);
    let events = stdout
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(events.first().unwrap()["event_kind"], "started");
    assert!(
        events
            .iter()
            .all(|event| event["api_version"] == "rebecca.cli.v2")
    );
    assert!(
        events
            .iter()
            .all(|event| event["command"] == "purge inspect")
    );
    let completed = events.last().unwrap();
    assert_eq!(completed["event_kind"], "completed");
    assert_eq!(completed["payload_kind"], "inspect-artifacts");
    assert_eq!(completed["data"]["summary"]["total_targets"], 1);
    assert_eq!(
        completed["data"]["top_targets"][0]["artifact"],
        "node_modules"
    );
}

#[test]
fn purge_human_output_highlights_recently_modified_artifacts() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    write_fixture_file(
        workspace.join("app").join("node_modules").join("pkg.bin"),
        b"abc",
    );
    write_node_project(workspace.join("app"));

    let output = isolated::isolated_rebecca(&temp)
        .args([
            "purge",
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

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Project artifact summary:"));
    assert!(stdout.contains("- node_modules: 1 target, 0 bytes (0 B) [1 skipped]"));
    assert!(stdout.contains("Recently modified artifacts:"));
    assert!(stdout.contains("- node_modules [skipped]"));
    assert!(stdout.contains("modified within the last 7 days"));
}

#[test]
fn purge_json_filters_selected_artifacts() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    write_fixture_file(
        workspace.join("app").join("node_modules").join("pkg.bin"),
        b"abc",
    );
    write_node_project(workspace.join("app"));
    write_fixture_file(
        workspace
            .join("app")
            .join("target")
            .join("debug")
            .join("app.bin"),
        b"rust",
    );
    write_rust_project(workspace.join("app"));

    let output = isolated::isolated_rebecca(&temp)
        .args([
            "purge",
            "--format",
            "json",
            "--no-progress",
            "--root",
            workspace.to_str().unwrap(),
            "--artifact",
            "target",
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

    let value: serde_json::Value = common::support::api_data(&output.stdout);
    assert_eq!(value["request"]["project_artifact_selectors"][0], "target");
    assert_eq!(value["summary"]["allowed_targets"], 1);
    assert_eq!(value["summary"]["estimated_bytes"], 4);

    let targets = value["targets"].as_array().unwrap();
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0]["rule_id"], "windows.project-artifact-target");
}

#[test]
fn purge_json_filters_context_sensitive_vendor_artifacts() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    let composer_vendor = workspace.join("php-app").join("vendor");
    write_fixture_file(composer_vendor.join("pkg").join("autoload.php"), b"php");
    write_fixture_file(workspace.join("php-app").join("composer.json"), b"{}");
    write_fixture_file(
        workspace
            .join("go-app")
            .join("vendor")
            .join("pkg")
            .join("dep.go"),
        b"go",
    );
    write_fixture_file(workspace.join("go-app").join("go.mod"), b"module example");

    let output = isolated::isolated_rebecca(&temp)
        .args([
            "purge",
            "--format",
            "json",
            "--no-progress",
            "--root",
            workspace.to_str().unwrap(),
            "--artifact",
            "vendor",
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

    let value: serde_json::Value = common::support::api_data(&output.stdout);
    assert_eq!(value["summary"]["allowed_targets"], 1);

    let targets = value["targets"].as_array().unwrap();
    assert_eq!(targets.len(), 1);
    assert_eq!(
        targets[0]["rule_id"],
        "windows.project-artifact-composer-vendor"
    );
    assert_eq!(
        PathBuf::from(targets[0]["path"].as_str().unwrap()),
        composer_vendor
    );
}

#[test]
fn purge_uses_configured_roots_when_root_flag_is_absent() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    write_fixture_file(
        workspace.join("app").join("node_modules").join("pkg.bin"),
        b"abc",
    );
    write_node_project(workspace.join("app"));
    write_config(
        &temp,
        format!(
            r#"
[purge]
roots = ['{}']
max_depth = 2
min_age_days = 0
"#,
            workspace.display()
        ),
    );

    let output = isolated::isolated_rebecca(&temp)
        .args(["purge", "--format", "json", "--no-progress"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let value: serde_json::Value = common::support::api_data(&output.stdout);
    assert_eq!(
        PathBuf::from(
            value["request"]["project_artifact_roots"][0]
                .as_str()
                .unwrap()
        ),
        workspace
    );
    assert_eq!(value["request"]["project_artifact_max_depth"], 2);
    assert_eq!(value["request"]["project_artifact_min_age_days"], 0);
    assert_eq!(value["summary"]["allowed_targets"], 1);
}

#[test]
fn purge_json_reports_missing_configured_root_as_discovery_diagnostic() {
    let temp = tempfile::tempdir().unwrap();
    let missing = temp.path().join("missing-workspace");
    write_config(
        &temp,
        format!(
            r#"
[purge]
roots = ['{}']
max_depth = 2
min_age_days = 0
"#,
            missing.display()
        ),
    );

    let output = isolated::isolated_rebecca(&temp)
        .args(["purge", "--format", "json", "--no-progress"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let value: serde_json::Value = common::support::api_data(&output.stdout);
    assert_eq!(value["summary"]["total_targets"], 0);
    assert!(value["targets"].as_array().unwrap().is_empty());

    let diagnostics = value["discovery_diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0]["kind"], "root-missing");
    assert_eq!(
        PathBuf::from(diagnostics[0]["path"].as_str().unwrap()),
        missing
    );
    assert!(
        diagnostics[0]["detail"]
            .as_str()
            .unwrap()
            .contains("does not exist")
    );
}

#[test]
fn purge_human_reports_partial_discovery_diagnostics() {
    let temp = tempfile::tempdir().unwrap();
    let missing = temp.path().join("missing-workspace");
    write_config(
        &temp,
        format!(
            r#"
[purge]
roots = ['{}']
max_depth = 2
min_age_days = 0
"#,
            missing.display()
        ),
    );

    let output = isolated::isolated_rebecca(&temp)
        .args(["purge", "--no-progress"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Project artifact discovery diagnostics: 1 observation"));
    assert!(stdout.contains("Partial discovery may have skipped some paths."));
    assert!(stdout.contains("root-missing"));
    assert!(stdout.contains(&missing.display().to_string()));
}

#[test]
fn purge_ndjson_completed_event_includes_discovery_diagnostics() {
    let temp = tempfile::tempdir().unwrap();
    let missing = temp.path().join("missing-workspace");
    write_config(
        &temp,
        format!(
            r#"
[purge]
roots = ['{}']
max_depth = 2
min_age_days = 0
"#,
            missing.display()
        ),
    );

    let output = isolated::isolated_rebecca(&temp)
        .args(["purge", "--format", "ndjson", "--no-progress"])
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
    let completed = events
        .last()
        .expect("ndjson should include a completed event");

    assert_eq!(completed["event_kind"], "completed");
    assert_eq!(completed["data"]["summary"]["total_targets"], 0);
    assert_eq!(
        completed["data"]["discovery_diagnostics"][0]["kind"],
        "root-missing"
    );
}

#[test]
fn purge_root_flag_overrides_configured_roots() {
    let temp = tempfile::tempdir().unwrap();
    let configured_workspace = temp.path().join("configured-workspace");
    let cli_workspace = temp.path().join("cli-workspace");
    write_fixture_file(
        configured_workspace
            .join("app")
            .join("node_modules")
            .join("pkg.bin"),
        b"configured",
    );
    write_node_project(configured_workspace.join("app"));
    write_fixture_file(
        cli_workspace
            .join("app")
            .join("target")
            .join("debug")
            .join("app.bin"),
        b"cli",
    );
    write_rust_project(cli_workspace.join("app"));
    write_config(
        &temp,
        format!(
            r#"
[purge]
roots = ['{}']
max_depth = 2
min_age_days = 0
"#,
            configured_workspace.display()
        ),
    );

    let output = isolated::isolated_rebecca(&temp)
        .args([
            "purge",
            "--format",
            "json",
            "--no-progress",
            "--root",
            cli_workspace.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let value: serde_json::Value = common::support::api_data(&output.stdout);
    assert_eq!(
        PathBuf::from(
            value["request"]["project_artifact_roots"][0]
                .as_str()
                .unwrap()
        ),
        cli_workspace
    );

    let targets = value["targets"].as_array().unwrap();
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0]["rule_id"], "windows.project-artifact-target");
    assert!(!targets.iter().any(|target| {
        PathBuf::from(target["path"].as_str().unwrap()).starts_with(&configured_workspace)
    }));
}

#[test]
fn purge_depth_and_min_age_flags_override_config() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    write_fixture_file(
        workspace
            .join("level1")
            .join("level2")
            .join("node_modules")
            .join("pkg.bin"),
        b"abc",
    );
    write_node_project(workspace.join("level1").join("level2"));
    write_config(
        &temp,
        format!(
            r#"
[purge]
roots = ['{}']
max_depth = 1
min_age_days = 30
"#,
            workspace.display()
        ),
    );

    let output = isolated::isolated_rebecca(&temp)
        .args([
            "purge",
            "--format",
            "json",
            "--no-progress",
            "--max-depth",
            "3",
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

    let value: serde_json::Value = common::support::api_data(&output.stdout);
    assert_eq!(value["request"]["project_artifact_max_depth"], 3);
    assert_eq!(value["request"]["project_artifact_min_age_days"], 0);
    assert_eq!(value["summary"]["allowed_targets"], 1);
}

#[test]
fn purge_reclaim_limit_and_older_than_alias_select_largest_artifacts() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    let small_node_modules = workspace.join("small").join("node_modules");
    let large_target = workspace.join("large").join("target");
    let medium_target = workspace.join("medium").join("target");
    write_fixture_file(small_node_modules.join("pkg.bin"), b"abc");
    write_node_project(workspace.join("small"));
    write_fixture_file(large_target.join("debug").join("app.bin"), b"12345");
    write_rust_project(workspace.join("large"));
    write_fixture_file(medium_target.join("debug").join("app.bin"), b"rust");
    write_rust_project(workspace.join("medium"));

    let output = isolated::isolated_rebecca(&temp)
        .args([
            "purge",
            "--format",
            "json",
            "--no-progress",
            "--root",
            workspace.to_str().unwrap(),
            "--older-than-days",
            "0",
            "--reclaim-limit-bytes",
            "5",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let value: serde_json::Value = common::support::api_data(&output.stdout);
    assert_eq!(value["request"]["project_artifact_min_age_days"], 0);
    assert_eq!(value["request"]["project_artifact_reclaim_limit_bytes"], 5);
    assert_eq!(value["summary"]["allowed_targets"], 1);
    assert_eq!(value["summary"]["skipped_targets"], 2);
    assert_eq!(value["summary"]["estimated_bytes"], 5);

    let allowed = value["targets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|target| target["status"] == "allowed")
        .unwrap();
    assert_eq!(
        PathBuf::from(allowed["path"].as_str().unwrap()),
        large_target
    );

    assert!(
        value["targets"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|target| target["status"] == "skipped")
            .all(|target| target["reason_code"] == "reclaim-limit-exceeded"
                && target["estimated_bytes"] == 0
                && target["estimate_source"] == "not-measured")
    );
}

#[test]
fn purge_json_skips_recent_artifacts_by_default() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    write_fixture_file(
        workspace.join("app").join("node_modules").join("pkg.bin"),
        b"abc",
    );
    write_node_project(workspace.join("app"));

    let output = isolated::isolated_rebecca(&temp)
        .args([
            "purge",
            "--format",
            "json",
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

    let value: serde_json::Value = common::support::api_data(&output.stdout);
    assert_eq!(value["summary"]["allowed_targets"], 0);
    assert_eq!(value["summary"]["skipped_targets"], 1);
    assert_eq!(value["summary"]["estimated_bytes"], 0);

    let target = &value["targets"].as_array().unwrap()[0];
    assert_eq!(target["status"], "skipped");
    assert_eq!(target["estimate_source"], "not-measured");
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

    let value: serde_json::Value = common::support::api_data(&output.stdout);
    assert_eq!(value["summary"]["allowed_targets"], 1);
    assert_eq!(
        value["summary"]["estimated_bytes"],
        3 + cachedir_tag_bytes().len() as u64
    );

    let target = &value["targets"].as_array().unwrap()[0];
    assert_eq!(target["rule_id"], "windows.project-artifact-cachedir-tag");
    assert_eq!(target["status"], "allowed");
    assert_eq!(
        target["project_artifact"]["matched_context"],
        "cachedir-tag"
    );
    assert!(
        PathBuf::from(
            target["project_artifact"]["project_anchor"]
                .as_str()
                .unwrap()
        )
        .ends_with(Path::new("tool-cache").join("CACHEDIR.TAG"))
    );
    assert!(
        PathBuf::from(target["path"].as_str().unwrap())
            .ends_with(Path::new("app").join("tool-cache"))
    );
}

#[test]
fn purge_reports_estimate_source_for_scan_cache_reuse() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    write_fixture_file(
        workspace.join("app").join("node_modules").join("pkg.bin"),
        b"abc",
    );
    write_node_project(workspace.join("app"));

    let output = isolated::isolated_rebecca(&temp)
        .args([
            "purge",
            "--format",
            "json",
            "--no-progress",
            "--scan-cache",
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

    let value: serde_json::Value = common::support::api_data(&output.stdout);
    assert_eq!(value["summary"]["estimated_bytes"], 3);
    assert_eq!(value["targets"][0]["estimate_source"], "fresh-scan");

    let scan_cache_dir = temp.path().join("rebecca-cache").join("scan");
    let cache_files = fs::read_dir(scan_cache_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    assert_eq!(cache_files.len(), 1);

    let cache_file = &cache_files[0];
    let mut record: serde_json::Value =
        serde_json::from_slice(&fs::read(cache_file).unwrap()).unwrap();
    record["report"]["bytes_scanned"] = serde_json::json!(99);
    fs::write(cache_file, serde_json::to_vec_pretty(&record).unwrap()).unwrap();

    let output = isolated::isolated_rebecca(&temp)
        .args([
            "purge",
            "--format",
            "json",
            "--no-progress",
            "--scan-cache",
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

    let value: serde_json::Value = common::support::api_data(&output.stdout);
    assert_eq!(value["summary"]["estimated_bytes"], 99);
    assert_eq!(value["targets"][0]["estimate_source"], "scan-cache");

    let output = isolated::isolated_rebecca(&temp)
        .args([
            "purge",
            "--no-progress",
            "--scan-cache",
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

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("[estimate: scan-cache]"));
}

#[test]
fn purge_human_output_shows_modified_time_for_artifacts() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    write_fixture_file(
        workspace.join("app").join("node_modules").join("pkg.bin"),
        b"abc",
    );
    write_node_project(workspace.join("app"));

    let output = isolated::isolated_rebecca(&temp)
        .args([
            "purge",
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

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("modified at"));
}

#[test]
fn purge_json_honors_exclude_flag() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    let node_modules = workspace.join("app").join("node_modules");
    write_fixture_file(node_modules.join("pkg.bin"), b"abc");
    write_node_project(workspace.join("app"));

    let output = isolated::isolated_rebecca(&temp)
        .args([
            "purge",
            "--format",
            "json",
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

    let value: serde_json::Value = common::support::api_data(&output.stdout);
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
        .args([
            "purge",
            "--format",
            "json",
            "--root",
            missing.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = common::support::stderr(&output);
    assert!(stderr.contains("purge root"));
    assert!(stderr.contains("not accessible"));
}

#[test]
fn purge_rejects_unknown_artifact_selector() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).unwrap();

    let output = isolated::isolated_rebecca(&temp)
        .args([
            "purge",
            "--format",
            "json",
            "--root",
            workspace.to_str().unwrap(),
            "--artifact",
            "missing-artifact",
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = common::support::stderr(&output);
    assert!(stderr.contains("invalid project artifact selector"));
    assert!(stderr.contains("missing-artifact"));
}

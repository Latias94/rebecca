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
    assert!(stdout.contains("--artifact"));
    assert!(stdout.contains("--list-artifacts"));
    assert!(stdout.contains("--exclude"));
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
        .args(["purge", "--list-artifacts", "--json"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let artifacts = value.as_array().unwrap();
    assert!(artifacts.iter().any(|artifact| {
        artifact["artifact"] == "node_modules"
            && artifact["rule_id"] == "windows.project-artifact-node-modules"
            && artifact["rule_suffix"] == "node-modules"
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
fn purge_human_output_groups_project_artifacts_by_project_path() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    write_fixture_file(
        workspace.join("app").join("node_modules").join("pkg.bin"),
        b"abc",
    );
    write_fixture_file(
        workspace
            .join("app")
            .join("target")
            .join("debug")
            .join("app.bin"),
        b"rust",
    );

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
    assert!(stdout.contains("Project artifact details:"));
    assert!(stdout.contains(&workspace.join("app").display().to_string()));
    assert!(stdout.contains("- node_modules [allowed]"));
    assert!(stdout.contains("- target [allowed]"));
}

#[test]
fn purge_human_output_highlights_recently_modified_artifacts() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    write_fixture_file(
        workspace.join("app").join("node_modules").join("pkg.bin"),
        b"abc",
    );

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
    write_fixture_file(
        workspace
            .join("app")
            .join("target")
            .join("debug")
            .join("app.bin"),
        b"rust",
    );

    let output = isolated::isolated_rebecca(&temp)
        .args([
            "purge",
            "--json",
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

    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
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
            "--json",
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

    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
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
        .args(["purge", "--json", "--no-progress"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
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
    write_fixture_file(
        cli_workspace
            .join("app")
            .join("target")
            .join("debug")
            .join("app.bin"),
        b"cli",
    );
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
            "--json",
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

    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
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
            "--json",
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

    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["request"]["project_artifact_max_depth"], 3);
    assert_eq!(value["request"]["project_artifact_min_age_days"], 0);
    assert_eq!(value["summary"]["allowed_targets"], 1);
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
fn purge_human_output_shows_modified_time_for_artifacts() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    write_fixture_file(
        workspace.join("app").join("node_modules").join("pkg.bin"),
        b"abc",
    );

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

#[test]
fn purge_rejects_unknown_artifact_selector() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).unwrap();

    let output = isolated::isolated_rebecca(&temp)
        .args([
            "purge",
            "--json",
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

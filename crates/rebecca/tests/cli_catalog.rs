mod common;

fn catalog_envelope(stdout: &[u8]) -> serde_json::Value {
    serde_json::from_slice(stdout).unwrap()
}

fn catalog_data(stdout: &[u8]) -> serde_json::Value {
    catalog_envelope(stdout)["data"].clone()
}

#[test]
fn catalog_human_output_lists_rules_artifacts_warnings_and_safety() {
    let output = common::command::rebecca()
        .args(["catalog"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Rebecca catalog:"));
    assert!(stdout.contains("- cleanup-rule:"));
    assert!(stdout.contains("windows.chrome-cache"));
    assert!(stdout.contains("- project-artifact:"));
    assert!(stdout.contains("node_modules"));
    assert!(stdout.contains("- warning:"));
    assert!(stdout.contains("active-process"));
    assert!(stdout.contains("- safety-category:"));
    assert!(stdout.contains("application-durable-data"));
    assert!(stdout.contains("- action-kind:"));
    assert!(stdout.contains("delete"));
}

#[test]
fn catalog_help_lists_validate_subcommand_without_hiding_filters() {
    let output = common::command::rebecca()
        .args(["catalog", "--help"])
        .output()
        .unwrap();

    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("validate"));
    assert!(stdout.contains("--kind"));
    assert!(stdout.contains("--warning"));
}

#[test]
fn catalog_json_uses_v2_envelope_and_lists_warning_entries() {
    let output = common::command::rebecca()
        .args(["catalog", "--format", "json", "--kind", "warning"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let envelope = catalog_envelope(&output.stdout);
    assert_eq!(envelope["api_version"], "rebecca.cli.v2");
    assert_eq!(envelope["command"], "catalog");
    assert_eq!(envelope["payload_kind"], "catalog");

    let items = envelope["data"].as_array().unwrap();
    assert!(items.iter().all(|item| item["kind"] == "warning"));
    assert!(items.iter().any(|item| {
        item["id"] == "active-process"
            && item["description"]
                .as_str()
                .is_some_and(|description| description.contains("running application"))
    }));
}

#[test]
fn catalog_validate_human_output_reports_builtin_catalog_health() {
    let output = common::command::rebecca()
        .args(["catalog", "validate"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Rebecca catalog validation: ok"));
    assert!(stdout.contains("Cleanup rules:"));
    assert!(stdout.contains("Targets:"));
    assert!(stdout.contains("built-in metadata gates pass"));
    assert!(stdout.contains("restricted reference provenance is no-copy"));
    assert!(stdout.contains("browser rules stay inside regenerable cache boundaries"));
}

#[test]
fn catalog_validate_json_reports_machine_readable_health_summary() {
    let output = common::command::rebecca()
        .args(["catalog", "validate", "--format", "json"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let envelope = catalog_envelope(&output.stdout);
    assert_eq!(envelope["api_version"], "rebecca.cli.v2");
    assert_eq!(envelope["command"], "catalog validate");
    assert_eq!(envelope["payload_kind"], "catalog-validation");
    assert_eq!(envelope["data"]["valid"], true);
    assert!(envelope["data"]["rule_count"].as_u64().unwrap() > 0);
    assert!(envelope["data"]["target_count"].as_u64().unwrap() > 0);
    assert!(
        envelope["data"]["categories"]
            .as_array()
            .unwrap()
            .iter()
            .any(|category| category == "browser")
    );
    let checks = envelope["data"]["checks"].as_array().unwrap();
    assert!(
        checks
            .iter()
            .any(|check| check == "restricted reference provenance is no-copy")
    );
    assert!(
        checks
            .iter()
            .any(|check| check == "protected target shapes are blocked")
    );
}

#[test]
fn catalog_filters_cleanup_rules_by_category_safety_and_rule() {
    let output = common::command::rebecca()
        .args([
            "catalog",
            "--format",
            "json",
            "--kind",
            "cleanup-rule",
            "--category",
            "development",
            "--safety-level",
            "moderate",
            "--rule",
            "windows.npm-cache",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let items = catalog_data(&output.stdout).as_array().unwrap().clone();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["kind"], "cleanup-rule");
    assert_eq!(items[0]["id"], "windows.npm-cache");
    assert_eq!(items[0]["category"], "development");
    assert_eq!(items[0]["safety_level"], "moderate");
}

#[test]
fn catalog_filters_project_artifacts_by_selector() {
    let output = common::command::rebecca()
        .args([
            "catalog",
            "--format",
            "json",
            "--kind",
            "project-artifact",
            "--artifact",
            "node-modules",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let items = catalog_data(&output.stdout).as_array().unwrap().clone();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["kind"], "project-artifact");
    assert_eq!(items[0]["artifact"], "node_modules");
    assert_eq!(items[0]["aliases"], serde_json::json!(["node-modules"]));
    assert_eq!(items[0]["rule_id"], "windows.project-artifact-node-modules");
    assert_eq!(items[0]["default_min_age_days"], 7);
    assert_eq!(items[0]["trim_eligible"], true);
    assert_eq!(items[0]["deletion_style"], "delete-whole-path");
    assert_eq!(items[0]["ranking"], "heavy-dependency-tree");
}

#[test]
fn catalog_invalid_selector_reports_machine_readable_error() {
    let output = common::command::rebecca()
        .args([
            "catalog",
            "--format",
            "json",
            "--warning",
            "missing-warning",
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());

    let envelope: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(envelope["api_version"], "rebecca.cli.v2");
    assert_eq!(envelope["kind"], "error");
    assert_eq!(envelope["command"], "catalog");
    assert_eq!(envelope["error"]["code"], "invalid-catalog-selector");
    assert!(
        envelope["error"]["detail"]
            .as_str()
            .unwrap()
            .contains("catalog selection did not match")
    );
}

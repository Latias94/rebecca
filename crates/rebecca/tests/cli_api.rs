mod common;

const API_DOCS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/api/cli/v1");

fn parse_json(bytes: &[u8]) -> serde_json::Value {
    serde_json::from_slice(bytes).unwrap()
}

fn read_doc_json(relative: &str) -> serde_json::Value {
    let path = std::path::Path::new(API_DOCS).join(relative);
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

fn validator_for_payload_def(def_name: &str) -> jsonschema::Validator {
    let payloads = read_doc_json("payloads.schema.json");
    let schema = serde_json::json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$defs": payloads["$defs"].clone(),
        "$ref": format!("#/$defs/{def_name}"),
    });
    jsonschema::validator_for(&schema).unwrap()
}

fn assert_success_schema(value: &serde_json::Value) {
    assert_eq!(value["api_version"], "rebecca.cli.v1");
    assert_eq!(value["kind"], "success");
    assert!(
        value["command"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
    );
    assert!(
        value["payload_kind"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
    );
    assert!(value["generated_at_unix_seconds"].as_u64().is_some());
    assert!(value.get("data").is_some());
}

fn assert_error_schema(value: &serde_json::Value) {
    assert_eq!(value["api_version"], "rebecca.cli.v1");
    assert_error_shape(value);
}

fn assert_error_shape(value: &serde_json::Value) {
    assert_eq!(value["kind"], "error");
    assert!(
        value["command"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
    );
    assert!(value["generated_at_unix_seconds"].as_u64().is_some());
    assert!(
        value["error"]["code"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
    );
    assert!(
        value["error"]["title"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
    );
    assert!(
        value["error"]["detail"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
    );
    assert!(
        value["error"]["exit_code"]
            .as_u64()
            .is_some_and(|value| value > 0)
    );
}

fn assert_event_schema(value: &serde_json::Value) {
    assert_eq!(value["api_version"], "rebecca.cli.v1");
    assert_event_shape(value);
}

fn assert_event_shape(value: &serde_json::Value) {
    assert_eq!(value["kind"], "event");
    assert!(
        value["command"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
    );
    assert!(value["sequence"].as_u64().is_some());
    assert!(
        value["event_kind"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
    );
    assert!(value["generated_at_unix_seconds"].as_u64().is_some());
}

#[test]
fn clean_format_json_returns_success_envelope_without_human_text() {
    let temp = tempfile::tempdir().unwrap();
    let temp_cache = temp.path().join("temp");
    std::fs::create_dir_all(&temp_cache).unwrap();
    std::fs::write(temp_cache.join("cache.tmp"), b"cache").unwrap();

    let output = common::isolated::isolated_rebecca(&temp)
        .env("REBECCA_STEAM_DISCOVERY", "none")
        .env("TEMP", &temp_cache)
        .env("TMPDIR", &temp_cache)
        .args(["clean", "--format", "json", "--category", "system"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains("Workflow:"));
    assert!(!stdout.contains("Targets:"));

    let value = parse_json(&output.stdout);
    assert_success_schema(&value);
    assert_eq!(value["api_version"], "rebecca.cli.v1");
    assert_eq!(value["kind"], "success");
    assert_eq!(value["command"], "clean");
    assert_eq!(value["payload_kind"], "cleanup-plan");
    assert!(value["generated_at_unix_seconds"].as_u64().is_some());
    assert_eq!(value["data"]["request"]["mode"], "dry-run");
    assert!(
        value["data"]["summary"]["allowed_targets"]
            .as_u64()
            .unwrap()
            >= 1
    );
}

#[test]
fn history_format_json_returns_empty_history_envelope() {
    let temp = tempfile::tempdir().unwrap();
    let output = common::isolated::isolated_rebecca(&temp)
        .args(["history", "--format", "json"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let value = parse_json(&output.stdout);
    assert_success_schema(&value);
    assert_eq!(value["api_version"], "rebecca.cli.v1");
    assert_eq!(value["kind"], "success");
    assert_eq!(value["command"], "history");
    assert_eq!(value["payload_kind"], "history-list");
    assert_eq!(value["data"].as_array().unwrap().len(), 0);
}

#[test]
fn clean_format_json_unknown_rule_returns_error_envelope_on_stderr() {
    let temp = tempfile::tempdir().unwrap();
    let output = common::isolated::isolated_rebecca(&temp)
        .args(["clean", "--format", "json", "--rule", "missing.rule"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());

    let value = parse_json(&output.stderr);
    assert_error_schema(&value);
    assert_eq!(value["api_version"], "rebecca.cli.v1");
    assert_eq!(value["kind"], "error");
    assert_eq!(value["command"], "clean");
    assert_eq!(value["error"]["code"], "invalid-rule-id");
    assert_eq!(value["error"]["exit_code"], 1);
    assert!(
        value["error"]["detail"]
            .as_str()
            .unwrap()
            .contains("missing.rule")
    );
}

#[test]
fn catalog_format_json_invalid_selector_returns_error_envelope() {
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

    let value = parse_json(&output.stderr);
    assert_error_schema(&value);
    assert_eq!(value["command"], "catalog");
    assert_eq!(value["error"]["code"], "invalid-catalog-selector");
    assert!(
        value["error"]["detail"]
            .as_str()
            .unwrap()
            .contains("catalog selection did not match")
    );
}

#[test]
fn capabilities_format_json_reports_gui_backend_contract() {
    let output = common::command::rebecca()
        .args(["capabilities", "--format", "json"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let envelope = common::support::api_envelope(&output.stdout);
    assert_success_schema(&envelope);
    assert_eq!(envelope["command"], "capabilities");
    assert_eq!(envelope["payload_kind"], "capabilities");

    let data = &envelope["data"];
    assert_eq!(data["api_version"], "rebecca.cli.v1");
    assert!(data["cli_version"].as_str().is_some());
    assert!(data["platform"]["current"].as_str().is_some());
    assert!(data["features"]["rules"].as_bool().is_some());
    assert!(data["features"]["ntfs"].as_bool().is_some());
    assert!(
        data["output_formats"]
            .as_array()
            .unwrap()
            .contains(&"json".into())
    );
    assert!(
        data["long_running_commands"]
            .as_array()
            .unwrap()
            .contains(&"clean".into())
    );
    assert!(
        data["schema_documents"]
            .as_array()
            .unwrap()
            .contains(&"config".into())
    );
    assert!(
        data["recommended_startup_commands"]
            .as_array()
            .unwrap()
            .contains(&"doctor permissions".into())
    );
    let commands = data["commands"].as_array().unwrap();
    assert!(commands.iter().any(|command| {
        command["name"] == "rules validate"
            && command["payload_kind"] == "rule-validation"
            && command["machine_readable"] == true
            && command["schema_documents"]
                .as_array()
                .unwrap()
                .contains(&"cleaner-manifest-v1".into())
    }));
    assert!(commands.iter().any(|command| {
        command["name"] == "clean"
            && command["required_execution_flag"] == "--yes"
            && command["preflight_commands"]
                .as_array()
                .unwrap()
                .contains(&"doctor permissions".into())
            && command["macos_privacy_relevant"] == true
    }));

    let validator = validator_for_payload_def("capabilities");
    validator.validate(data).unwrap();
}

#[test]
fn schema_export_format_json_returns_requested_schema_document() {
    let output = common::command::rebecca()
        .args([
            "schema",
            "export",
            "--document",
            "payloads",
            "--format",
            "json",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let envelope = common::support::api_envelope(&output.stdout);
    assert_success_schema(&envelope);
    assert_eq!(envelope["command"], "schema export");
    assert_eq!(envelope["payload_kind"], "cli-schema");
    assert_eq!(envelope["data"]["document"], "payloads");
    assert_eq!(envelope["data"]["api_version"], "rebecca.cli.v1");
    assert_eq!(
        envelope["data"]["schema"]["$id"],
        "https://rebecca.local/schemas/cli/v1/payloads.schema.json"
    );

    let validator = validator_for_payload_def("cliSchema");
    validator.validate(&envelope["data"]).unwrap();
}

#[test]
fn schema_export_format_json_returns_config_schema_document() {
    let output = common::command::rebecca()
        .args([
            "schema",
            "export",
            "--document",
            "config",
            "--format",
            "json",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let envelope = common::support::api_envelope(&output.stdout);
    assert_success_schema(&envelope);
    assert_eq!(envelope["data"]["document"], "config");
    assert_eq!(
        envelope["data"]["schema"]["$id"],
        "https://rebecca.dev/schemas/cli/v1/config.schema.json"
    );

    let validator = validator_for_payload_def("cliSchema");
    validator.validate(&envelope["data"]).unwrap();
}

#[test]
fn rules_validate_format_json_accepts_external_manifest_without_importing_it() {
    let temp = tempfile::tempdir().unwrap();
    let rule_file = temp.path().join("example-cache.toml");
    std::fs::write(
        &rule_file,
        r#"
manifest_version = 1
id = "example-cache"
category = "development"
name = "Example cache"
safety_level = "safe"
restore_hint = "Example rebuilds this cache automatically."

[provenance]
source = "reference-only"
license = "example-user-rule"
notes = "Local user-authored validation fixture; no external rule data copied."

[[platforms]]
platform = "macos"

[[platforms.targets]]
kind = "template"
value = "MACOS_CACHE_HOME/Example"
search_kind = "file"
"#,
    )
    .unwrap();

    let output = common::command::rebecca()
        .args([
            "rules",
            "validate",
            "--format",
            "json",
            "--file",
            rule_file.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let envelope = common::support::api_envelope(&output.stdout);
    assert_success_schema(&envelope);
    assert_eq!(envelope["command"], "rules validate");
    assert_eq!(envelope["payload_kind"], "rule-validation");
    assert_eq!(envelope["data"]["valid"], true);
    assert_eq!(envelope["data"]["rule_count"], 1);
    assert_eq!(envelope["data"]["target_count"], 1);
    assert_eq!(envelope["data"]["rules"][0], "macos.example-cache");
    assert_eq!(envelope["data"]["enabled"], false);
    assert_eq!(envelope["data"]["summary"]["diagnostics"], 0);
    assert_eq!(
        envelope["data"]["rule_previews"][0]["rule_id"],
        "macos.example-cache"
    );
    assert_eq!(
        envelope["data"]["rule_previews"][0]["enabled_by_validation"],
        false
    );

    let validator = validator_for_payload_def("ruleValidation");
    validator.validate(&envelope["data"]).unwrap();
}

#[test]
fn rules_validate_format_json_missing_inputs_returns_rule_catalog_error() {
    let output = common::command::rebecca()
        .args(["rules", "validate", "--format", "json"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(output.stderr.is_empty());
    let envelope = common::support::api_envelope(&output.stdout);
    assert_success_schema(&envelope);
    assert_eq!(envelope["command"], "rules validate");
    assert_eq!(envelope["payload_kind"], "rule-validation");
    assert_eq!(envelope["data"]["valid"], false);
    assert_eq!(
        envelope["data"]["diagnostics"][0]["code"],
        "rule-catalog-invalid"
    );
    assert!(
        envelope["data"]["diagnostics"][0]["message"]
            .as_str()
            .is_some_and(|detail| detail.contains("at least one --file or --dir"))
    );

    let validator = validator_for_payload_def("ruleValidation");
    validator.validate(&envelope["data"]).unwrap();
}

#[cfg(unix)]
#[test]
fn rules_validate_dir_does_not_follow_symlinked_directories() {
    let temp = tempfile::tempdir().unwrap();
    let rules_dir = temp.path().join("rules");
    let linked_dir = temp.path().join("linked");
    std::fs::create_dir_all(&rules_dir).unwrap();
    std::fs::create_dir_all(&linked_dir).unwrap();
    std::fs::write(
        rules_dir.join("example-cache.toml"),
        r#"
manifest_version = 1
id = "example-cache"
category = "development"
name = "Example cache"
safety_level = "safe"
restore_hint = "Example rebuilds this cache automatically."

[provenance]
source = "reference-only"
license = "example-user-rule"
notes = "Local user-authored validation fixture; no external rule data copied."

[[platforms]]
platform = "macos"

[[platforms.targets]]
kind = "template"
value = "MACOS_CACHE_HOME/Example"
search_kind = "file"
"#,
    )
    .unwrap();
    std::fs::write(
        linked_dir.join("bad.toml"),
        r#"
manifest_version = 1
id = "bad-state"
category = "development"
name = "Bad credential target"
safety_level = "safe"
restore_hint = "Do not use."

[provenance]
source = "reference-only"
license = "example-user-rule"
notes = "Local user-authored validation fixture; no external rule data copied."

[[platforms]]
platform = "macos"

[[platforms.targets]]
kind = "template"
value = "MACOS_HOME/.ssh"
"#,
    )
    .unwrap();
    std::os::unix::fs::symlink(&linked_dir, rules_dir.join("linked")).unwrap();

    let output = common::command::rebecca()
        .args([
            "rules",
            "validate",
            "--format",
            "json",
            "--dir",
            rules_dir.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let envelope = common::support::api_envelope(&output.stdout);
    assert_success_schema(&envelope);
    assert_eq!(envelope["data"]["rule_count"], 1);
    assert_eq!(envelope["data"]["rules"][0], "macos.example-cache");
    assert_eq!(envelope["data"]["summary"]["diagnostics"], 0);
    assert_eq!(
        envelope["data"]["rule_previews"][0]["rule_id"],
        "macos.example-cache"
    );
    assert_eq!(
        envelope["data"]["rule_previews"][0]["enabled_by_validation"],
        false
    );
}

#[test]
fn rules_validate_format_json_rejects_protected_external_manifest() {
    let temp = tempfile::tempdir().unwrap();
    let rule_file = temp.path().join("bad.toml");
    std::fs::write(
        &rule_file,
        r#"
manifest_version = 1
id = "bad-state"
category = "development"
name = "Bad credential target"
safety_level = "safe"
restore_hint = "Do not use."

[provenance]
source = "reference-only"
license = "example-user-rule"
notes = "Local user-authored validation fixture; no external rule data copied."

[[platforms]]
platform = "macos"

[[platforms.targets]]
kind = "template"
value = "MACOS_HOME/.ssh"
"#,
    )
    .unwrap();

    let output = common::command::rebecca()
        .args([
            "rules",
            "validate",
            "--format",
            "json",
            "--file",
            rule_file.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(output.stderr.is_empty());
    let envelope = common::support::api_envelope(&output.stdout);
    assert_success_schema(&envelope);
    assert_eq!(envelope["command"], "rules validate");
    assert_eq!(envelope["payload_kind"], "rule-validation");
    assert_eq!(envelope["data"]["valid"], false);
    assert_eq!(
        envelope["data"]["diagnostics"][0]["code"],
        "rule-catalog-invalid"
    );
    assert!(
        envelope["data"]["diagnostics"][0]["message"]
            .as_str()
            .is_some_and(|detail| detail.contains("blocked by credentials"))
    );

    let validator = validator_for_payload_def("ruleValidation");
    validator.validate(&envelope["data"]).unwrap();
}

#[test]
fn config_validate_format_json_reports_effective_config_contract() {
    let temp = tempfile::tempdir().unwrap();
    let output = common::isolated::isolated_rebecca(&temp)
        .args(["config", "validate", "--format", "json"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let envelope = common::support::api_envelope(&output.stdout);
    assert_success_schema(&envelope);
    assert_eq!(envelope["command"], "config validate");
    assert_eq!(envelope["payload_kind"], "config-validation");
    assert_eq!(envelope["data"]["valid"], true);
    assert_eq!(envelope["data"]["schema_version"], 1);
    assert_eq!(envelope["data"]["summary"]["diagnostics"], 0);
    assert!(
        envelope["data"]["diagnostics"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let validator = validator_for_payload_def("configValidation");
    validator.validate(&envelope["data"]).unwrap();
}

#[test]
fn config_validate_format_json_missing_explicit_file_returns_read_error() {
    let temp = tempfile::tempdir().unwrap();
    let missing_config = temp.path().join("missing.toml");
    let output = common::isolated::isolated_rebecca(&temp)
        .args([
            "config",
            "validate",
            "--format",
            "json",
            "--file",
            missing_config.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(output.stderr.is_empty());
    let envelope = common::support::api_envelope(&output.stdout);
    assert_success_schema(&envelope);
    assert_eq!(envelope["command"], "config validate");
    assert_eq!(envelope["payload_kind"], "config-validation");
    assert_eq!(envelope["data"]["valid"], false);
    assert_eq!(
        envelope["data"]["diagnostics"][0]["code"],
        "config-read-failed"
    );

    let validator = validator_for_payload_def("configValidation");
    validator.validate(&envelope["data"]).unwrap();
}

#[test]
fn config_show_format_json_returns_loaded_and_effective_config() {
    let temp = tempfile::tempdir().unwrap();
    let output = common::isolated::isolated_rebecca(&temp)
        .args(["config", "show", "--format", "json"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let envelope = common::support::api_envelope(&output.stdout);
    assert_success_schema(&envelope);
    assert_eq!(envelope["command"], "config show");
    assert_eq!(envelope["payload_kind"], "config-view");
    assert_eq!(envelope["data"]["schema_version"], 1);
    assert!(
        envelope["data"]["config"]["scan_cache"]["directory_record_max_age_seconds"]
            .as_u64()
            .is_some()
    );
    assert!(
        envelope["data"]["runtime"]["app_paths"]["history_file"]
            .as_str()
            .unwrap()
            .contains("history.jsonl")
    );

    let validator = validator_for_payload_def("configView");
    validator.validate(&envelope["data"]).unwrap();
}

#[test]
fn config_show_format_json_malformed_explicit_file_returns_parse_error() {
    let temp = tempfile::tempdir().unwrap();
    let config_file = temp.path().join("broken.toml");
    std::fs::write(&config_file, "version = 2\n").unwrap();

    let output = common::isolated::isolated_rebecca(&temp)
        .args([
            "config",
            "show",
            "--format",
            "json",
            "--file",
            config_file.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let envelope = parse_json(&output.stderr);
    assert_error_schema(&envelope);
    assert_eq!(envelope["command"], "config show");
    assert_eq!(envelope["error"]["code"], "config-parse-failed");
}

#[test]
fn invalid_format_is_rejected_by_clap() {
    let output = common::command::rebecca()
        .args(["clean", "--format", "invalid"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = common::support::stderr(&output);
    assert!(stderr.contains("invalid value"));
    assert!(stderr.contains("json"));
    assert!(stderr.contains("ndjson"));
}

#[test]
fn parse_error_format_json_returns_error_envelope() {
    let output = common::command::rebecca()
        .args(["clean", "--format", "json", "--bogus"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());

    let value = parse_json(&output.stderr);
    assert_error_schema(&value);
    assert_eq!(value["command"], "clean");
    assert_eq!(value["error"]["code"], "invalid-arguments");
    assert_eq!(value["error"]["source"], "clap");
    assert!(
        value["error"]["detail"]
            .as_str()
            .unwrap()
            .contains("unexpected argument")
    );
}

#[test]
fn parse_error_format_ndjson_returns_error_event() {
    let output = common::command::rebecca()
        .args(["inspect", "artifacts", "--format", "ndjson", "--bogus"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(output.stderr.is_empty());

    let events = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(events.len(), 1);
    let event = events.first().unwrap();
    assert_event_schema(event);
    assert_eq!(event["command"], "inspect artifacts");
    assert_eq!(event["event_kind"], "error");
    assert_eq!(event["error"]["code"], "invalid-arguments");
}

#[test]
fn destructive_commands_reject_dry_run_yes_in_machine_format() {
    let cases = [
        (
            ["clean", "--format", "json", "--dry-run", "--yes"].as_slice(),
            "clean",
        ),
        (
            ["apps", "clean", "--format", "json", "--dry-run", "--yes"].as_slice(),
            "apps clean",
        ),
        (
            ["purge", "--format", "json", "--dry-run", "--yes"].as_slice(),
            "purge",
        ),
        (
            ["cache", "prune", "--format", "json", "--dry-run", "--yes"].as_slice(),
            "cache prune",
        ),
        (
            ["cache", "purge", "--format", "json", "--dry-run", "--yes"].as_slice(),
            "cache purge",
        ),
    ];

    for (args, command) in cases {
        let output = common::command::rebecca().args(args).output().unwrap();

        assert!(!output.status.success(), "{command} should fail");
        assert!(
            output.stdout.is_empty(),
            "{command} should not write stdout"
        );
        let value = parse_json(&output.stderr);
        assert_error_schema(&value);
        assert_eq!(value["command"], command);
        assert_eq!(value["error"]["code"], "validation-error");
        assert!(
            value["error"]["detail"]
                .as_str()
                .unwrap()
                .contains("--dry-run cannot be combined with --yes")
        );
    }
}

#[test]
fn clean_format_ndjson_emits_lifecycle_events() {
    let temp = tempfile::tempdir().unwrap();
    let temp_cache = temp.path().join("temp");
    std::fs::create_dir_all(&temp_cache).unwrap();
    std::fs::write(temp_cache.join("cache.tmp"), b"cache").unwrap();

    let output = common::isolated::isolated_rebecca(&temp)
        .env("REBECCA_STEAM_DISCOVERY", "none")
        .env("TEMP", &temp_cache)
        .env("TMPDIR", &temp_cache)
        .args([
            "clean",
            "--format",
            "ndjson",
            "--scan-cache",
            "--category",
            "system",
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

    assert!(
        events.len() >= 3,
        "expected started, progress, and completed events: {stdout}"
    );
    assert_eq!(events.first().unwrap()["event_kind"], "started");
    assert_eq!(events.last().unwrap()["event_kind"], "completed");
    assert!(
        events
            .iter()
            .all(|event| event["api_version"] == "rebecca.cli.v1")
    );
    assert!(events.iter().all(|event| event["kind"] == "event"));
    assert!(events.iter().all(|event| {
        assert_event_schema(event);
        true
    }));
    assert!(events.iter().all(|event| event["command"] == "clean"));
    assert!(events.windows(2).all(|pair| {
        pair[0]["sequence"].as_u64().unwrap() < pair[1]["sequence"].as_u64().unwrap()
    }));
    assert!(
        events.iter().any(|event| event["event_kind"]
            .as_str()
            .unwrap()
            .starts_with("scan-cache-"))
            || events
                .iter()
                .any(|event| event["event_kind"] == "target-finished")
    );
}

#[test]
fn clean_format_ndjson_omits_file_measured_events_by_default() {
    let temp = tempfile::tempdir().unwrap();
    let temp_cache = temp.path().join("temp");
    std::fs::create_dir_all(&temp_cache).unwrap();
    for index in 0..8 {
        std::fs::write(temp_cache.join(format!("cache-{index}.tmp")), b"cache").unwrap();
    }

    let output = common::isolated::isolated_rebecca(&temp)
        .env("TEMP", &temp_cache)
        .env("TMPDIR", &temp_cache)
        .args([
            "clean",
            "--format",
            "ndjson",
            "--no-scan-cache",
            "--rule",
            common::support::current_platform_user_temp_rule_id(),
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

    assert!(
        events
            .iter()
            .any(|event| event["event_kind"] == "target-finished")
    );
    assert!(events.last().unwrap()["event_kind"] == "completed");
    assert!(
        events
            .iter()
            .all(|event| event["event_kind"] != "file-measured"),
        "default ndjson should not include file-level events: {stdout}"
    );
}

#[test]
fn clean_format_ndjson_file_progress_detail_emits_file_measured_events() {
    let temp = tempfile::tempdir().unwrap();
    let temp_cache = temp.path().join("temp");
    std::fs::create_dir_all(&temp_cache).unwrap();
    for index in 0..8 {
        std::fs::write(temp_cache.join(format!("cache-{index}.tmp")), b"cache").unwrap();
    }

    let output = common::isolated::isolated_rebecca(&temp)
        .env("TEMP", &temp_cache)
        .env("TMPDIR", &temp_cache)
        .args([
            "clean",
            "--format",
            "ndjson",
            "--progress-detail",
            "file",
            "--no-scan-cache",
            "--rule",
            common::support::current_platform_user_temp_rule_id(),
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
    let file_events = events
        .iter()
        .filter(|event| event["event_kind"] == "file-measured")
        .count();

    assert_eq!(
        file_events, 8,
        "verbose ndjson should include file events: {stdout}"
    );
    assert!(events.last().unwrap()["event_kind"] == "completed");
}

#[test]
fn clean_format_ndjson_unknown_rule_terminates_with_error_event() {
    let temp = tempfile::tempdir().unwrap();
    let output = common::isolated::isolated_rebecca(&temp)
        .args(["clean", "--format", "ndjson", "--rule", "missing.rule"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(output.stderr.is_empty());

    let stdout = String::from_utf8_lossy(&output.stdout);
    let events = stdout
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();

    assert_eq!(events.first().unwrap()["event_kind"], "started");
    assert!(events.windows(2).all(|pair| {
        pair[0]["sequence"].as_u64().unwrap() < pair[1]["sequence"].as_u64().unwrap()
    }));

    let error = events.last().unwrap();
    assert_event_schema(error);
    assert_eq!(error["event_kind"], "error");
    assert_eq!(error["error"]["code"], "invalid-rule-id");
    assert_eq!(error["error"]["exit_code"], 1);
}

#[test]
fn inspect_artifacts_format_ndjson_invalid_root_returns_error_event() {
    let temp = tempfile::tempdir().unwrap();
    let missing = temp.path().join("missing-workspace");
    let output = common::isolated::isolated_rebecca(&temp)
        .args([
            "inspect",
            "artifacts",
            "--format",
            "ndjson",
            "--root",
            missing.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(output.stderr.is_empty());

    let stdout = String::from_utf8_lossy(&output.stdout);
    let events = stdout
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();

    let error = events.last().unwrap();
    assert_event_schema(error);
    assert_eq!(error["command"], "inspect artifacts");
    assert_eq!(error["event_kind"], "error");
    assert_eq!(error["error"]["code"], "invalid-purge-root");
    assert!(
        error["error"]["detail"]
            .as_str()
            .unwrap()
            .contains("is not accessible")
    );
}

#[test]
fn doctor_permissions_format_json_returns_diagnostic_payload() {
    let output = common::command::rebecca()
        .args(["doctor", "permissions", "--format", "json"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let envelope = common::support::api_envelope(&output.stdout);
    assert_success_schema(&envelope);
    assert_eq!(envelope["command"], "doctor permissions");
    assert_eq!(envelope["payload_kind"], "permissions-diagnostic");
    assert!(envelope["data"]["platform"].as_str().is_some());
    assert!(envelope["data"]["platform_supported"].as_bool().is_some());
    assert!(
        envelope["data"]["cleanup_execution_supported"]
            .as_bool()
            .is_some()
    );
    assert!(envelope["data"]["privilege_level"].as_str().is_some());
    assert!(envelope["data"]["suggested_action"].as_str().is_some());
    if cfg!(target_os = "macos") {
        let macos_privacy = envelope["data"]["macos_privacy"].as_object().unwrap();
        assert!(macos_privacy["status"].as_str().is_some());
        assert!(macos_privacy["probes"].as_array().is_some());
        assert!(macos_privacy["action_kind"].as_str().is_some());
        assert!(
            macos_privacy["full_disk_access_relevant"]
                .as_bool()
                .is_some()
        );
        assert!(
            macos_privacy["affected_cleanup_families"]
                .as_array()
                .is_some()
        );
        assert!(macos_privacy["suggested_action"].as_str().is_some());
    } else {
        assert!(envelope["data"].get("macos_privacy").is_none());
    }

    let validator = validator_for_payload_def("permissionsDiagnostic");
    assert!(
        validator.is_valid(&envelope["data"]),
        "permissions diagnostic should match schema: {:?}",
        validator
            .iter_errors(&envelope["data"])
            .map(|error| error.to_string())
            .collect::<Vec<_>>()
    );
}

#[test]
fn doctor_active_processes_format_json_returns_diagnostic_payload() {
    let output = common::command::rebecca()
        .env("REBECCA_ACTIVE_PROCESSES", "slack.exe:4242")
        .args(["doctor", "active-processes", "--format", "json"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let envelope = common::support::api_envelope(&output.stdout);
    assert_success_schema(&envelope);
    assert_eq!(envelope["command"], "doctor active-processes");
    assert_eq!(envelope["payload_kind"], "active-process-diagnostic");

    let validator = validator_for_payload_def("activeProcessDiagnostic");
    assert!(
        validator.is_valid(&envelope["data"]),
        "active process diagnostic should match schema: {:?}",
        validator
            .iter_errors(&envelope["data"])
            .map(|error| error.to_string())
            .collect::<Vec<_>>()
    );
}

#[test]
fn cli_api_schema_documents_are_parseable_draft_2020_12() {
    for relative in [
        "envelope.schema.json",
        "error.schema.json",
        "event.schema.json",
        "payloads.schema.json",
        "config.schema.json",
        "cleaner-manifest-v1.schema.json",
    ] {
        let schema = read_doc_json(relative);
        assert_eq!(
            schema["$schema"],
            "https://json-schema.org/draft/2020-12/schema"
        );
        assert!(schema["$id"].as_str().is_some());
        assert!(schema["title"].as_str().is_some());
    }

    let payloads = read_doc_json("payloads.schema.json");
    let payload_kinds = payloads["$defs"]["payloadKind"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(!payload_kinds.contains(&"project-artifact-insight"));
    assert!(payload_kinds.contains(&"active-process-diagnostic"));
}

#[test]
fn cli_api_catalog_and_inspect_payloads_are_documented_in_v1() {
    let payloads = read_doc_json("payloads.schema.json");
    let payload_kinds = payloads["$defs"]["payloadKind"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(payload_kinds.contains(&"capabilities"));
    assert!(payload_kinds.contains(&"cli-schema"));
    assert!(payload_kinds.contains(&"config-validation"));
    assert!(payload_kinds.contains(&"config-view"));
    assert!(payload_kinds.contains(&"rule-validation"));
    assert!(payload_kinds.contains(&"catalog"));
    assert!(payload_kinds.contains(&"catalog-validation"));
    assert!(payload_kinds.contains(&"cache-doctor"));
    assert!(payload_kinds.contains(&"cache-inventory"));
    assert!(payload_kinds.contains(&"cache-prune-report"));
    assert!(payload_kinds.contains(&"inspect-artifacts"));
    assert!(payload_kinds.contains(&"inspect-lint"));
    assert!(payload_kinds.contains(&"inspect-map"));
    assert!(payload_kinds.contains(&"inspect-map-entry"));
    assert!(payload_kinds.contains(&"inspect-map-group"));
    assert!(payload_kinds.contains(&"inspect-progress"));
    assert!(payload_kinds.contains(&"inspect-space"));

    assert_eq!(payloads["$defs"]["capabilities"]["type"], "object");
    assert_eq!(payloads["$defs"]["cliSchema"]["type"], "object");
    assert_eq!(payloads["$defs"]["configValidation"]["type"], "object");
    assert_eq!(payloads["$defs"]["configView"]["type"], "object");
    assert_eq!(payloads["$defs"]["ruleValidation"]["type"], "object");
    let catalog_item = &payloads["$defs"]["catalogItem"];
    assert_eq!(catalog_item["oneOf"].as_array().unwrap().len(), 5);
    assert_eq!(payloads["$defs"]["inspectSpace"]["type"], "object");
    assert_eq!(payloads["$defs"]["inspectArtifacts"]["type"], "object");
    assert_eq!(payloads["$defs"]["inspectLint"]["type"], "object");
    assert_eq!(payloads["$defs"]["inspectMap"]["type"], "object");
    assert_eq!(payloads["$defs"]["inspectMapEntryEvent"]["type"], "object");
    assert_eq!(payloads["$defs"]["inspectMapGroupEvent"]["type"], "object");
    assert_eq!(payloads["$defs"]["cacheInventory"]["type"], "object");
    assert_eq!(payloads["$defs"]["cacheDoctor"]["type"], "object");
    assert_eq!(payloads["$defs"]["cachePruneReport"]["type"], "object");

    let event = read_doc_json("event.schema.json");
    assert_eq!(
        event["properties"]["api_version"]["const"],
        "rebecca.cli.v1"
    );
    assert_eq!(
        event["properties"]["payload_kind"]["$ref"],
        "payloads.schema.json#/$defs/payloadKind"
    );

    let error = read_doc_json("error.schema.json");
    assert_eq!(
        error["properties"]["api_version"]["const"],
        "rebecca.cli.v1"
    );
}

#[test]
fn cli_api_examples_match_documented_envelope_shapes() {
    let error = read_doc_json("examples/error-invalid-rule.json");
    assert_error_schema(&error);
    assert_eq!(error["error"]["code"], "invalid-rule-id");

    let event = read_doc_json("examples/event-completed.json");
    assert_event_schema(&event);
    assert_eq!(event["event_kind"], "completed");

    let examples_dir = std::path::Path::new(API_DOCS).join("examples");
    let mut success_payload_kinds = std::fs::read_dir(examples_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("success-") && name.ends_with(".json"))
        })
        .map(|path| {
            let value: serde_json::Value =
                serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
            assert_success_schema(&value);
            value["payload_kind"].as_str().unwrap().to_owned()
        })
        .collect::<Vec<_>>();
    success_payload_kinds.sort();

    assert!(success_payload_kinds.contains(&"cleanup-plan".to_string()));
    assert!(success_payload_kinds.contains(&"cache-doctor".to_string()));
    assert!(success_payload_kinds.contains(&"cache-inventory".to_string()));
    assert!(success_payload_kinds.contains(&"cache-prune-report".to_string()));
    assert!(success_payload_kinds.contains(&"project-artifact-cleanup-plan".to_string()));
    assert!(!success_payload_kinds.contains(&"project-artifact-insight".to_string()));
}

#[test]
fn cli_api_catalog_error_example_matches_documented_shape() {
    let error = read_doc_json("examples/error-invalid-warning.json");
    assert_error_schema(&error);
    assert_eq!(error["command"], "catalog");
    assert_eq!(error["error"]["code"], "invalid-catalog-selector");
}

#[test]
fn cli_api_catalog_example_validates_against_payload_schema() {
    let example = read_doc_json("examples/success-catalog.json");
    assert_success_schema(&example);
    assert_eq!(example["payload_kind"], "catalog");

    let validator = validator_for_payload_def("catalog");
    validator.validate(&example["data"]).unwrap();
}

#[test]
fn cli_api_inspect_examples_validate_against_payload_schema() {
    let space = read_doc_json("examples/success-inspect-space.json");
    assert_success_schema(&space);
    assert_eq!(space["payload_kind"], "inspect-space");
    validator_for_payload_def("inspectSpace")
        .validate(&space["data"])
        .unwrap();

    let artifacts = read_doc_json("examples/success-inspect-artifacts.json");
    assert_success_schema(&artifacts);
    assert_eq!(artifacts["payload_kind"], "inspect-artifacts");
    validator_for_payload_def("inspectArtifacts")
        .validate(&artifacts["data"])
        .unwrap();

    let map = read_doc_json("examples/success-inspect-map.json");
    assert_success_schema(&map);
    assert_eq!(map["payload_kind"], "inspect-map");
    validator_for_payload_def("inspectMap")
        .validate(&map["data"])
        .unwrap();

    let lint = read_doc_json("examples/success-inspect-lint.json");
    assert_success_schema(&lint);
    assert_eq!(lint["payload_kind"], "inspect-lint");
    validator_for_payload_def("inspectLint")
        .validate(&lint["data"])
        .unwrap();
}

#[test]
fn cli_api_cache_examples_validate_against_payload_schema() {
    let inspect = read_doc_json("examples/success-cache-inspect.json");
    assert_success_schema(&inspect);
    assert_eq!(inspect["payload_kind"], "cache-inventory");
    validator_for_payload_def("cacheInventory")
        .validate(&inspect["data"])
        .unwrap();

    let doctor = read_doc_json("examples/success-cache-doctor.json");
    assert_success_schema(&doctor);
    assert_eq!(doctor["payload_kind"], "cache-doctor");
    validator_for_payload_def("cacheDoctor")
        .validate(&doctor["data"])
        .unwrap();

    let prune = read_doc_json("examples/success-cache-prune.json");
    assert_success_schema(&prune);
    assert_eq!(prune["payload_kind"], "cache-prune-report");
    validator_for_payload_def("cachePruneReport")
        .validate(&prune["data"])
        .unwrap();
}

#[test]
fn cli_api_gui_backend_examples_validate_against_payload_schema() {
    let capabilities = read_doc_json("examples/success-capabilities.json");
    assert_success_schema(&capabilities);
    assert_eq!(capabilities["payload_kind"], "capabilities");
    validator_for_payload_def("capabilities")
        .validate(&capabilities["data"])
        .unwrap();

    let schema = read_doc_json("examples/success-cli-schema.json");
    assert_success_schema(&schema);
    assert_eq!(schema["payload_kind"], "cli-schema");
    validator_for_payload_def("cliSchema")
        .validate(&schema["data"])
        .unwrap();

    let config_validation = read_doc_json("examples/success-config-validation.json");
    assert_success_schema(&config_validation);
    assert_eq!(config_validation["payload_kind"], "config-validation");
    validator_for_payload_def("configValidation")
        .validate(&config_validation["data"])
        .unwrap();

    let config_view = read_doc_json("examples/success-config-view.json");
    assert_success_schema(&config_view);
    assert_eq!(config_view["payload_kind"], "config-view");
    validator_for_payload_def("configView")
        .validate(&config_view["data"])
        .unwrap();

    let rule_validation = read_doc_json("examples/success-rule-validation.json");
    assert_success_schema(&rule_validation);
    assert_eq!(rule_validation["payload_kind"], "rule-validation");
    validator_for_payload_def("ruleValidation")
        .validate(&rule_validation["data"])
        .unwrap();
}

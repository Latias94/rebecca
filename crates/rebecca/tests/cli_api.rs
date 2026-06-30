mod common;
#[path = "common/isolated.rs"]
mod isolated;

const API_DOCS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/api/cli/v1");

fn parse_json(bytes: &[u8]) -> serde_json::Value {
    serde_json::from_slice(bytes).unwrap()
}

fn read_doc_json(relative: &str) -> serde_json::Value {
    let path = std::path::Path::new(API_DOCS).join(relative);
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
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

    let output = isolated::isolated_rebecca(&temp)
        .env("REBECCA_STEAM_DISCOVERY", "none")
        .env("TEMP", &temp_cache)
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
    let output = isolated::isolated_rebecca(&temp)
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
    let output = isolated::isolated_rebecca(&temp)
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
fn clean_format_ndjson_emits_lifecycle_events() {
    let temp = tempfile::tempdir().unwrap();
    let temp_cache = temp.path().join("temp");
    std::fs::create_dir_all(&temp_cache).unwrap();
    std::fs::write(temp_cache.join("cache.tmp"), b"cache").unwrap();

    let output = isolated::isolated_rebecca(&temp)
        .env("REBECCA_STEAM_DISCOVERY", "none")
        .env("TEMP", &temp_cache)
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
fn clean_format_ndjson_unknown_rule_terminates_with_error_event() {
    let temp = tempfile::tempdir().unwrap();
    let output = isolated::isolated_rebecca(&temp)
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
    assert!(envelope["data"]["privilege_level"].as_str().is_some());
    assert!(envelope["data"]["suggested_action"].as_str().is_some());
}

#[test]
fn cli_api_schema_documents_are_parseable_draft_2020_12() {
    for relative in [
        "envelope.schema.json",
        "error.schema.json",
        "event.schema.json",
        "payloads.schema.json",
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
    assert!(payload_kinds.contains(&"project-artifact-insight"));
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
    assert!(success_payload_kinds.contains(&"project-artifact-cleanup-plan".to_string()));
    assert!(success_payload_kinds.contains(&"project-artifact-insight".to_string()));
}

mod common;

#[test]
fn trash_empty_help_explains_preview_and_drive_scope() {
    let output = common::command::rebecca()
        .args(["trash", "empty", "--help"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Preview or empty the system trash"));
    assert!(stdout.contains("On Windows this uses the Recycle Bin"));
    assert!(stdout.contains("--yes"));
    assert!(stdout.contains("--drive <DRIVE>"));
    assert!(stdout.contains("rebecca trash empty --yes"));
    assert!(stdout.contains("rebecca trash empty --drive E --yes"));
}

#[cfg(windows)]
#[test]
fn trash_empty_json_previews_recycle_bin_without_emptying_it() {
    let output = common::command::rebecca()
        .args(["trash", "empty", "--format", "json", "--drive", "C"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let envelope = common::support::api_envelope(&output.stdout);
    assert_eq!(envelope["command"], "trash empty");
    assert_eq!(envelope["payload_kind"], "trash-report");

    let value = &envelope["data"];
    assert_eq!(value["mode"], "dry-run");
    assert_eq!(value["emptied"], false);
    assert_eq!(value["summary"]["byte_accuracy"], "exact");
    assert!(matches!(
        value["targets"][0]["root"].as_str(),
        Some("C:\\") | Some("C:/")
    ));
    assert_eq!(value["targets"][0]["status"], "would-empty");
}

#[cfg(all(
    unix,
    not(target_os = "macos"),
    not(target_os = "ios"),
    not(target_os = "android")
))]
#[test]
fn trash_empty_json_previews_freedesktop_trash_without_emptying_it() {
    let output = common::command::rebecca()
        .args(["trash", "empty", "--format", "json"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let envelope = common::support::api_envelope(&output.stdout);
    assert_eq!(envelope["command"], "trash empty");
    assert_eq!(envelope["payload_kind"], "trash-report");

    let value = &envelope["data"];
    assert_eq!(value["mode"], "dry-run");
    assert_eq!(value["emptied"], false);
    assert!(value["summary"]["items"].as_u64().is_some());
    assert!(value["summary"]["bytes"].as_u64().is_some());
    assert!(matches!(
        value["summary"]["byte_accuracy"].as_str(),
        Some("exact" | "known-file-bytes")
    ));
    assert_eq!(value["targets"][0]["status"], "would-empty");
}

#[cfg(not(any(
    windows,
    all(
        unix,
        not(target_os = "macos"),
        not(target_os = "ios"),
        not(target_os = "android")
    )
)))]
#[test]
fn trash_empty_json_reports_unsupported_platform() {
    let output = common::command::rebecca()
        .args(["trash", "empty", "--format", "json"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = common::support::stderr(&output);
    assert!(stderr.contains("system trash listing is not supported on this platform yet"));
    assert!(stderr.contains("\"code\": \"platform-unavailable\""));
}

#[cfg(not(any(
    windows,
    all(
        unix,
        not(target_os = "macos"),
        not(target_os = "ios"),
        not(target_os = "android")
    )
)))]
#[test]
fn trash_empty_yes_json_reports_unsupported_platform() {
    let output = common::command::rebecca()
        .args(["trash", "empty", "--yes", "--format", "json"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let envelope: serde_json::Value =
        serde_json::from_slice(&output.stderr).expect("stderr should be a JSON error envelope");
    assert_eq!(envelope["command"], "trash empty");
    assert_eq!(envelope["payload_kind"], "trash-report");
    assert_eq!(envelope["error"]["code"], "platform-unavailable");
    assert!(
        envelope["error"]["detail"]
            .as_str()
            .unwrap()
            .contains("system trash emptying is not supported on this platform yet")
    );
}

mod common;

#[test]
fn root_help_shows_completion_and_rejects_hidden_default_scan() {
    let output = common::command::rebecca().arg("--help").output().unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--format"));
    assert!(stdout.contains("json"));
    assert!(stdout.contains("ndjson"));
    assert!(stdout.contains("completion"));
    assert!(stdout.contains("scan"));
    assert!(stdout.contains("clean"));
    assert!(stdout.contains("purge"));
}

#[test]
fn root_without_subcommand_prints_help_instead_of_scanning() {
    let output = common::command::rebecca().output().unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Windows-first cleanup CLI"));
    assert!(stdout.contains("completion"));
    assert!(!stdout.contains("Rebecca rules:"));
}

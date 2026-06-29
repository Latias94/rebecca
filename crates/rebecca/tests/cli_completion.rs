mod common;

#[test]
fn completion_help_lists_supported_shells() {
    let output = common::command::rebecca()
        .args(["completion", "--help"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("bash"));
    assert!(stdout.contains("powershell"));
    assert!(stdout.contains("zsh"));
}

#[test]
fn completion_generation_includes_current_subcommands() {
    let output = common::command::rebecca()
        .args(["completion", "powershell"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("scan"));
    assert!(stdout.contains("clean"));
    assert!(stdout.contains("purge"));
    assert!(stdout.contains("completion"));
}

#[test]
fn completion_invalid_shell_is_rejected() {
    let output = common::command::rebecca()
        .args(["completion", "invalid"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = common::support::stderr(&output);
    assert!(stderr.contains("invalid value"));
    assert!(stderr.contains("powershell"));
}

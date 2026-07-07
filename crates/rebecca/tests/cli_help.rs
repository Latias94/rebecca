mod common;

fn help_stdout(args: &[&str]) -> String {
    let output = common::command::rebecca().args(args).output().unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn root_help_shows_completion_and_rejects_hidden_default_scan() {
    let stdout = help_stdout(&["--help"]);
    assert!(stdout.contains("--format"));
    assert!(stdout.contains("json"));
    assert!(stdout.contains("ndjson"));
    assert!(stdout.contains("completion"));
    assert!(stdout.contains("catalog"));
    assert!(stdout.contains("inspect"));
    assert!(stdout.contains("scan"));
    assert!(stdout.contains("clean"));
    assert!(stdout.contains("tui"));
    assert!(stdout.contains("purge"));
}

#[test]
fn inspect_help_shows_canonical_read_only_commands() {
    let stdout = help_stdout(&["inspect", "--help"]);
    assert!(stdout.contains("space"));
    assert!(stdout.contains("artifacts"));
    assert!(stdout.contains("lint"));
}

#[test]
fn clean_help_preserves_preview_execution_and_warning_controls() {
    let stdout = help_stdout(&["clean", "--help"]);

    assert!(stdout.contains("Build or execute a cleanup plan"));
    assert!(stdout.contains("--dry-run"));
    assert!(stdout.contains("Preview the cleanup plan without deleting anything"));
    assert!(stdout.contains("--yes"));
    assert!(stdout.contains("Move allowed targets to recoverable trash"));
    assert!(stdout.contains("--allow-moderate"));
    assert!(stdout.contains("--allow-risky"));
    assert!(stdout.contains("--allow-warning <WARNING>"));
    assert!(stdout.contains("--scan-cache"));
    assert!(stdout.contains("--no-scan-cache"));
}

#[test]
fn tui_help_shows_interactive_workbench_controls() {
    let stdout = help_stdout(&["tui", "--help"]);

    assert!(stdout.contains("interactive terminal workbench"));
    assert!(stdout.contains("--root <PATH>"));
    assert!(stdout.contains("--scan-backend <SCAN_BACKEND>"));
    assert!(stdout.contains("--entry-limit <N>"));
    assert!(stdout.contains("--screen-reader"));
    assert!(stdout.contains("--no-color"));
    assert!(!stdout.contains("--once"));
    assert!(!stdout.contains("--replay-keys"));
    assert!(!stdout.contains("--terminal-width"));
}

#[test]
fn tui_alias_renders_once_frame_for_ci() {
    let output = common::command::rebecca()
        .args(["i", "--once"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Rebecca TUI | root-picker"));
    assert!(stdout.contains("Roots"));
}

#[test]
fn tui_without_tty_is_rejected() {
    let output = common::command::rebecca().args(["tui"]).output().unwrap();

    assert!(!output.status.success());
    let stderr = common::support::stderr(&output);
    assert!(stderr.contains("requires an interactive terminal"));
}

#[test]
fn inspect_map_help_preserves_human_output_controls() {
    let stdout = help_stdout(&["inspect", "map", "--help"]);

    assert!(stdout.contains("Inspect ranked disk usage below one or more roots"));
    assert!(stdout.contains("--no-progress"));
    assert!(stdout.contains("--progress-detail <PROGRESS_DETAIL>"));
    assert!(stdout.contains("--full-path"));
    assert!(stdout.contains("Print full paths in human ranked output"));
    assert!(stdout.contains("--no-bars"));
    assert!(stdout.contains("Hide visual usage bars"));
    assert!(stdout.contains("--bar-width <COLUMNS>"));
    assert!(stdout.contains("--screen-reader"));
    assert!(stdout.contains("--group-by <GROUP_KINDS>"));
    assert!(stdout.contains("--table <FORMAT>"));
    assert!(stdout.contains("--cleanup-advice"));
}

#[test]
fn inspect_space_help_shows_progress_controls() {
    let stdout = help_stdout(&["inspect", "space", "--help"]);

    assert!(stdout.contains("Inspect top-level disk usage below one or more roots"));
    assert!(stdout.contains("--no-progress"));
    assert!(stdout.contains("--progress-detail <PROGRESS_DETAIL>"));
}

#[test]
fn cache_doctor_help_preserves_health_contract() {
    let stdout = help_stdout(&["cache", "doctor", "--help"]);

    assert!(stdout.contains("Diagnose Rebecca cache health"));
    assert!(stdout.contains("prune recommendations"));
    assert!(stdout.contains("--format <FORMAT>"));
    assert!(stdout.contains("json"));
    assert!(stdout.contains("ndjson"));
}

#[test]
fn doctor_active_processes_help_preserves_warning_gate_contract() {
    let stdout = help_stdout(&["doctor", "active-processes", "--help"]);

    assert!(stdout.contains("Report warning-bearing cleanup rules"));
    assert!(stdout.contains("applications appear to be running"));
    assert!(stdout.contains("--format <FORMAT>"));
    assert!(stdout.contains("json"));
    assert!(stdout.contains("ndjson"));
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
    assert!(stdout.contains("Cross-platform cleanup CLI"));
    assert!(stdout.contains("completion"));
    assert!(!stdout.contains("Rebecca rules:"));
}

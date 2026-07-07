use std::fs;
use std::path::Path;

mod common;

fn write_fixture_file(path: impl AsRef<Path>, bytes: &[u8]) {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, bytes).unwrap();
}

#[test]
fn tui_once_renders_disk_map_snapshot_for_root() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    write_fixture_file(root.join("big").join("data.bin"), b"abcdef");
    write_fixture_file(root.join("small.txt"), b"x");

    let output = common::isolated::isolated_rebecca(&temp)
        .args(["tui", "--once", "--root", root.to_str().unwrap()])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Rebecca TUI | map"));
    assert!(stdout.contains("Map: workspace"));
    assert!(stdout.contains("big"));
    assert!(stdout.contains("small.txt"));
    assert!(stdout.contains("Status:"));
}

#[test]
fn tui_replay_can_scan_current_directory_from_root_picker() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    write_fixture_file(root.join("cache").join("data.bin"), b"abc");

    let output = common::isolated::isolated_rebecca(&temp)
        .current_dir(&root)
        .args(["i", "--once", "--replay-keys", "enter"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Rebecca TUI | map"));
    assert!(stdout.contains("Map: workspace"));
    assert!(stdout.contains("cache"));
}

#[test]
fn tui_replay_can_change_sort_without_a_terminal() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    write_fixture_file(root.join("alpha.bin"), b"abc");

    let output = common::isolated::isolated_rebecca(&temp)
        .args([
            "tui",
            "--once",
            "--root",
            root.to_str().unwrap(),
            "--replay-keys",
            "s",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("sort allocated"));
    assert!(stdout.contains("Status: Sorted by allocated."));
}

#[test]
fn tui_replay_can_open_history_without_a_terminal() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    write_fixture_file(root.join("alpha.bin"), b"abc");

    let output = common::isolated::isolated_rebecca(&temp)
        .args([
            "tui",
            "--once",
            "--root",
            root.to_str().unwrap(),
            "--replay-keys",
            "g",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Rebecca TUI | history"));
    assert!(stdout.contains("No cleanup history entries yet."));
}

#[test]
fn tui_screen_reader_once_omits_visual_bars() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    write_fixture_file(root.join("cache").join("data.bin"), b"abcdef");

    let output = common::isolated::isolated_rebecca(&temp)
        .args([
            "tui",
            "--once",
            "--screen-reader",
            "--root",
            root.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Rebecca TUI | map"));
    assert!(stdout.contains("cache"));
    assert!(
        !stdout.contains("###"),
        "screen-reader snapshot should not depend on visual bars: {stdout}"
    );
}

#[test]
fn tui_once_respects_hidden_terminal_width_for_ci() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    write_fixture_file(
        root.join("very-long-directory-name-for-width-testing")
            .join("data.bin"),
        b"abcdef",
    );

    let output = common::isolated::isolated_rebecca(&temp)
        .args([
            "tui",
            "--once",
            "--terminal-width",
            "40",
            "--root",
            root.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.lines().all(|line| line.chars().count() <= 40));
}

#[test]
fn tui_rejects_machine_output_modes() {
    let output = common::command::rebecca()
        .args(["--format", "json", "tui", "--once"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = common::support::stderr(&output);
    assert!(stderr.contains("requires --format human"));
}

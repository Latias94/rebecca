use std::fs;
use std::path::Path;

use unicode_width::UnicodeWidthStr;

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
fn tui_replay_can_show_type_distribution_without_a_terminal() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    write_fixture_file(root.join("src").join("main.rs"), b"abc");
    write_fixture_file(root.join("docs").join("readme.md"), b"abcde");
    write_fixture_file(root.join("LICENSE"), b"xy");

    let output = common::isolated::isolated_rebecca(&temp)
        .args([
            "tui",
            "--once",
            "--root",
            root.to_str().unwrap(),
            "--replay-keys",
            "t",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Rebecca TUI | types"));
    assert!(stdout.contains("Types: file kind distribution"));
    assert!(stdout.contains("Files"));
    assert!(stdout.contains("Directories"));
}

#[test]
fn tui_replay_can_show_extension_distribution_without_a_terminal() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    write_fixture_file(root.join("src").join("main.rs"), b"abc");
    write_fixture_file(root.join("docs").join("readme.md"), b"abcde");
    write_fixture_file(root.join("LICENSE"), b"xy");

    let output = common::isolated::isolated_rebecca(&temp)
        .args([
            "tui",
            "--once",
            "--root",
            root.to_str().unwrap(),
            "--replay-keys",
            "x",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Rebecca TUI | extensions"));
    assert!(stdout.contains("Extensions: suffix distribution"));
    assert!(stdout.contains(".md"));
    assert!(stdout.contains(".rs"));
    assert!(stdout.contains("No extension"));
}

#[test]
fn tui_replay_can_filter_map_from_extension_distribution() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    write_fixture_file(root.join("cache.tmp"), b"abcdef");
    write_fixture_file(root.join("notes.txt"), b"abc");

    let output = common::isolated::isolated_rebecca(&temp)
        .args([
            "tui",
            "--once",
            "--root",
            root.to_str().unwrap(),
            "--replay-keys",
            "x enter",
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
    assert!(stdout.contains("filter .tmp"));
    assert!(stdout.contains("Map: workspace [.tmp]"));
    assert!(stdout.contains("cache.tmp"));
    assert!(!stdout.contains("notes.txt"));
}

#[test]
fn tui_replay_can_clear_distribution_filter() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    write_fixture_file(root.join("cache.tmp"), b"abcdef");
    write_fixture_file(root.join("notes.txt"), b"abc");

    let output = common::isolated::isolated_rebecca(&temp)
        .args([
            "tui",
            "--once",
            "--root",
            root.to_str().unwrap(),
            "--replay-keys",
            "x enter backspace",
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
    assert!(stdout.contains("Status: Cleared extension .tmp filter."));
    assert!(stdout.contains("cache.tmp"));
    assert!(stdout.contains("notes.txt"));
}

#[test]
fn tui_replay_can_show_treemap_without_a_terminal() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    write_fixture_file(root.join("big").join("data.bin"), b"abcdef");
    write_fixture_file(root.join("small.txt"), b"x");

    let output = common::isolated::isolated_rebecca(&temp)
        .args([
            "tui",
            "--once",
            "--root",
            root.to_str().unwrap(),
            "--replay-keys",
            "4",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Rebecca TUI | treemap"));
    assert!(stdout.contains("Treemap: workspace"));
    assert!(stdout.contains("big"));
    assert!(stdout.contains("small.txt"));
}

#[test]
fn tui_replay_tab_reaches_treemap_without_a_terminal() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    write_fixture_file(root.join("alpha.bin"), b"abcdef");

    let output = common::isolated::isolated_rebecca(&temp)
        .args([
            "tui",
            "--once",
            "--root",
            root.to_str().unwrap(),
            "--replay-keys",
            "tab",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Rebecca TUI | treemap"));
    assert!(stdout.contains("Treemap: workspace"));
}

#[test]
fn tui_replay_semantic_tab_reaches_treemap_without_a_terminal() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    write_fixture_file(root.join("alpha.bin"), b"abcdef");

    let output = common::isolated::isolated_rebecca(&temp)
        .args([
            "tui",
            "--once",
            "--root",
            root.to_str().unwrap(),
            "--replay-keys",
            "click:tab:treemap",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Rebecca TUI | treemap"));
    assert!(stdout.contains("Treemap: workspace"));
}

#[test]
fn tui_replay_semantic_row_selects_visible_map_row() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    write_fixture_file(root.join("big").join("data.bin"), b"abcdef");
    write_fixture_file(root.join("small.txt"), b"x");

    let output = common::isolated::isolated_rebecca(&temp)
        .args([
            "tui",
            "--once",
            "--root",
            root.to_str().unwrap(),
            "--replay-keys",
            "click:row:1",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout
            .lines()
            .any(|line| { line.starts_with('>') && line.contains("small.txt") })
    );
    assert!(stdout.contains("Status: Selected small.txt."));
}

#[test]
fn tui_replay_semantic_distribution_row_filters_map() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    write_fixture_file(root.join("cache.tmp"), b"abcdef");
    write_fixture_file(root.join("notes.txt"), b"abc");

    let output = common::isolated::isolated_rebecca(&temp)
        .args([
            "tui",
            "--once",
            "--root",
            root.to_str().unwrap(),
            "--replay-keys",
            "click:tab:extensions click:row:0",
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
    assert!(stdout.contains("filter .tmp"));
    assert!(stdout.contains("cache.tmp"));
    assert!(!stdout.contains("notes.txt"));
}

#[test]
fn tui_replay_semantic_wheel_moves_visible_selection() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    write_fixture_file(root.join("big").join("data.bin"), b"abcdef");
    write_fixture_file(root.join("small.txt"), b"x");

    let output = common::isolated::isolated_rebecca(&temp)
        .args([
            "tui",
            "--once",
            "--root",
            root.to_str().unwrap(),
            "--replay-keys",
            "wheel:down",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout
            .lines()
            .any(|line| { line.starts_with('>') && line.contains("small.txt") })
    );
}

#[test]
fn tui_replay_semantic_treemap_tile_selects_visible_row() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    write_fixture_file(root.join("big").join("data.bin"), b"abcdef");
    write_fixture_file(root.join("small.txt"), b"x");

    let output = common::isolated::isolated_rebecca(&temp)
        .args([
            "tui",
            "--once",
            "--root",
            root.to_str().unwrap(),
            "--replay-keys",
            "click:tab:treemap click:tile:0",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Rebecca TUI | treemap"));
    assert!(stdout.contains("Status: Selected "));
}

#[test]
fn tui_replay_double_tab_reaches_type_distribution_without_a_terminal() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    write_fixture_file(root.join("alpha.bin"), b"abcdef");

    let output = common::isolated::isolated_rebecca(&temp)
        .args([
            "tui",
            "--once",
            "--root",
            root.to_str().unwrap(),
            "--replay-keys",
            "tab tab",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Rebecca TUI | types"));
    assert!(stdout.contains("Types: file kind distribution"));
}

#[test]
fn tui_screen_reader_extension_distribution_omits_visual_bars() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    write_fixture_file(root.join("alpha.bin"), b"abcdef");
    write_fixture_file(root.join("beta.log"), b"abc");

    let output = common::isolated::isolated_rebecca(&temp)
        .args([
            "tui",
            "--once",
            "--screen-reader",
            "--root",
            root.to_str().unwrap(),
            "--replay-keys",
            "x",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Rebecca TUI | extensions"));
    assert!(stdout.contains(".bin"));
    assert!(stdout.contains(".log"));
    assert!(stdout.contains("%"));
    assert!(stdout.contains("file"));
    assert!(
        !stdout.contains("###"),
        "screen-reader extension snapshot should not depend on visual bars: {stdout}"
    );
}

#[test]
fn tui_replay_can_refresh_selected_directory_and_restore_previous_scan() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    write_fixture_file(root.join("big").join("data.bin"), b"abcdef");
    write_fixture_file(root.join("small.txt"), b"x");

    let refreshed = common::isolated::isolated_rebecca(&temp)
        .args([
            "tui",
            "--once",
            "--root",
            root.to_str().unwrap(),
            "--replay-keys",
            "r",
        ])
        .output()
        .unwrap();

    assert!(
        refreshed.status.success(),
        "stderr: {}",
        common::support::stderr(&refreshed)
    );
    let refreshed_stdout = String::from_utf8_lossy(&refreshed.stdout);
    assert!(refreshed_stdout.contains("Map: workspace"));
    assert!(refreshed_stdout.contains("big"));
    assert!(refreshed_stdout.contains("small.txt"));
    assert!(refreshed_stdout.contains("Status: Refresh complete for "));

    let opened = common::isolated::isolated_rebecca(&temp)
        .args([
            "tui",
            "--once",
            "--root",
            root.to_str().unwrap(),
            "--replay-keys",
            "r enter",
        ])
        .output()
        .unwrap();

    assert!(
        opened.status.success(),
        "stderr: {}",
        common::support::stderr(&opened)
    );
    let opened_stdout = String::from_utf8_lossy(&opened.stdout);
    assert!(opened_stdout.contains("Map: big"));
    assert!(opened_stdout.contains("data.bin"));

    let restored = common::isolated::isolated_rebecca(&temp)
        .args([
            "tui",
            "--once",
            "--root",
            root.to_str().unwrap(),
            "--replay-keys",
            "r b",
        ])
        .output()
        .unwrap();

    assert!(
        restored.status.success(),
        "stderr: {}",
        common::support::stderr(&restored)
    );
    let restored_stdout = String::from_utf8_lossy(&restored.stdout);
    assert!(restored_stdout.contains("Map: workspace"));
    assert!(restored_stdout.contains("big"));
    assert!(restored_stdout.contains("small.txt"));
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
fn tui_screen_reader_treemap_omits_visual_bars() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    write_fixture_file(root.join("alpha.bin"), b"abcdef");
    write_fixture_file(root.join("beta.log"), b"abc");

    let output = common::isolated::isolated_rebecca(&temp)
        .args([
            "tui",
            "--once",
            "--screen-reader",
            "--root",
            root.to_str().unwrap(),
            "--replay-keys",
            "4",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Rebecca TUI | treemap"));
    assert!(stdout.contains("Treemap: workspace"));
    assert!(stdout.contains("alpha.bin"));
    assert!(stdout.contains("beta.log"));
    assert!(stdout.contains("%"));
    assert!(
        !stdout.contains("###"),
        "screen-reader treemap snapshot should not depend on visual bars: {stdout}"
    );
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
    assert!(
        stdout
            .lines()
            .all(|line| UnicodeWidthStr::width(line) <= 40)
    );
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

#[test]
fn tui_non_tty_rejects_before_replay_or_scan_setup() {
    let output = common::command::rebecca()
        .args(["tui", "--replay-keys", "not-a-key"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = common::support::stderr(&output);
    assert!(stderr.contains("requires an interactive terminal"));
    assert!(!stderr.contains("unknown tui replay key token"));
}

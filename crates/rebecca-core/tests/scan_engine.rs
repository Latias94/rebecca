use std::fs;

use rebecca_core::scan::{measure_path_size, scan_target, scan_targets};
use rebecca_core::{DeleteMode, TargetStatus};

#[test]
fn measures_directory_size_from_fixture_files() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("a.txt"), b"abcd").unwrap();
    fs::create_dir(temp.path().join("nested")).unwrap();
    fs::write(temp.path().join("nested").join("b.txt"), b"ef").unwrap();

    let size = measure_path_size(temp.path()).unwrap();

    assert_eq!(size, 6);
}

#[test]
fn missing_scan_target_is_reported_as_skipped() {
    let temp = tempfile::tempdir().unwrap();
    let target = scan_target(
        "windows.user-temp",
        temp.path().join("missing"),
        DeleteMode::DryRun,
    );

    assert_eq!(target.status, TargetStatus::Skipped);
}

#[test]
fn scan_targets_returns_deterministic_ordering() {
    let temp = tempfile::tempdir().unwrap();
    let first = temp.path().join("first");
    let second = temp.path().join("second");
    fs::create_dir(&first).unwrap();
    fs::create_dir(&second).unwrap();

    let targets = scan_targets(vec![
        ("windows.z".to_string(), second, DeleteMode::DryRun),
        ("windows.a".to_string(), first, DeleteMode::DryRun),
    ]);

    assert_eq!(targets[0].rule_id, "windows.a");
    assert_eq!(targets[1].rule_id, "windows.z");
}

#[cfg(unix)]
#[test]
fn symlink_root_is_blocked_by_default() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let real = temp.path().join("real");
    let link = temp.path().join("link");
    fs::create_dir(&real).unwrap();
    symlink(&real, &link).unwrap();

    let target = scan_target("windows.user-temp", link, DeleteMode::DryRun);

    assert_eq!(target.status, TargetStatus::Blocked);
}

use std::fs;

use rebecca_core::error::{ScanFailureKind, ScanFailurePhase};
use rebecca_core::scan::{
    ScanCancellationToken, ScanProgressEvent, measure_path, measure_path_size,
    measure_path_size_with_progress, scan_target, scan_targets,
};
use rebecca_core::{DeleteMode, RebeccaError, TargetStatus};

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
fn measures_directory_report_from_fixture_files() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("a.txt"), b"abcd").unwrap();
    fs::create_dir(temp.path().join("nested")).unwrap();
    fs::write(temp.path().join("nested").join("b.txt"), b"ef").unwrap();

    let report = measure_path(temp.path()).unwrap();

    assert_eq!(report.bytes_scanned, 6);
    assert_eq!(report.files_scanned, 2);
    assert_eq!(report.directories_scanned, 2);
}

#[test]
fn missing_path_size_reports_structured_scan_failure() {
    let temp = tempfile::tempdir().unwrap();
    let missing = temp.path().join("missing");

    let err = measure_path_size(&missing).unwrap_err();

    let RebeccaError::ScanFailed(failure) = err else {
        panic!("expected structured scan failure");
    };
    assert_eq!(failure.kind, ScanFailureKind::NotFound);
    assert_eq!(failure.phase, ScanFailurePhase::RootMetadata);
    assert_eq!(failure.path, missing);
}

#[test]
fn measuring_directory_reports_file_level_progress() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("a.txt"), b"abcd").unwrap();
    fs::create_dir(temp.path().join("nested")).unwrap();
    fs::write(temp.path().join("nested").join("b.txt"), b"ef").unwrap();

    let token = ScanCancellationToken::new();
    let mut events = Vec::new();
    let size = measure_path_size_with_progress(temp.path(), &token, |event| match event {
        ScanProgressEvent::FileMeasured {
            file_size,
            files_scanned,
            bytes_scanned,
            ..
        } => events.push((file_size, files_scanned, bytes_scanned)),
    })
    .unwrap();

    assert_eq!(size, 6);
    assert_eq!(events.len(), 2);
    assert_eq!(
        events
            .iter()
            .map(|(file_size, _, _)| *file_size)
            .sum::<u64>(),
        6
    );
    assert_eq!(
        events.last().map(|(_, files, bytes)| (*files, *bytes)),
        Some((2, 6))
    );
}

#[test]
fn measuring_directory_can_be_cancelled_during_file_scan() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("a.txt"), b"abcd").unwrap();
    fs::write(temp.path().join("b.txt"), b"ef").unwrap();

    let token = ScanCancellationToken::new();
    let mut events = 0u64;
    let err = measure_path_size_with_progress(temp.path(), &token, |event| match event {
        ScanProgressEvent::FileMeasured { .. } => {
            events += 1;
            token.cancel();
        }
    })
    .unwrap_err();

    assert!(matches!(err, RebeccaError::OperationCancelled(_)));
    assert_eq!(events, 1);
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

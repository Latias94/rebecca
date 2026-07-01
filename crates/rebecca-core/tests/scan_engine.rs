use std::fs;

use rebecca_core::error::{ScanFailureKind, ScanFailurePhase};
use rebecca_core::scan::{
    ScanBackendKind, ScanCancellationToken, ScanEngine, ScanEstimateConfidence, ScanProgressEvent,
    ScanTargetRequest,
};
use rebecca_core::{DeleteMode, RebeccaError, TargetStatus};

#[test]
fn measures_directory_size_from_fixture_files() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("a.txt"), b"abcd").unwrap();
    fs::create_dir(temp.path().join("nested")).unwrap();
    fs::write(temp.path().join("nested").join("b.txt"), b"ef").unwrap();

    let size = ScanEngine::new()
        .measure_path(temp.path())
        .unwrap()
        .bytes_scanned;

    assert_eq!(size, 6);
}

#[test]
fn measures_directory_report_from_fixture_files() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("a.txt"), b"abcd").unwrap();
    fs::create_dir(temp.path().join("nested")).unwrap();
    fs::write(temp.path().join("nested").join("b.txt"), b"ef").unwrap();

    let report = ScanEngine::new().measure_path(temp.path()).unwrap();

    assert_eq!(report.bytes_scanned, 6);
    assert_eq!(report.files_scanned, 2);
    assert_eq!(report.directories_scanned, 2);
}

#[test]
fn measured_scan_reports_portable_backend_metadata() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("a.txt"), b"abcd").unwrap();

    let measured = ScanEngine::new().measure_scan(temp.path()).unwrap();

    assert_eq!(measured.report.bytes_scanned, 4);
    assert_eq!(measured.backend, ScanBackendKind::PortableRecursive);
    assert_eq!(measured.backend.label(), "portable-recursive");
    assert_eq!(measured.confidence, ScanEstimateConfidence::Exact);
    assert_eq!(measured.confidence.label(), "exact");
    assert_eq!(measured.fallback_reason, None);
    assert!(measured.caveats.is_empty());
}

#[test]
fn backend_selection_can_force_portable_backend() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("a.txt"), b"abcd").unwrap();

    let measured = ScanEngine::new()
        .measure_scan_with_backend(
            temp.path(),
            &ScanCancellationToken::new(),
            ScanBackendKind::PortableRecursive,
            |_| {},
        )
        .unwrap();

    assert_eq!(measured.report.bytes_scanned, 4);
    assert_eq!(measured.backend, ScanBackendKind::PortableRecursive);
    assert_eq!(measured.fallback_reason, None);
}

#[test]
fn windows_native_selection_falls_back_to_portable_when_unavailable() {
    #[cfg(windows)]
    {
        let current_dir = std::env::current_dir().unwrap();
        let temp = tempfile::tempdir_in(&current_dir).unwrap();
        fs::write(temp.path().join("a.txt"), b"abcd").unwrap();
        let relative_path = temp.path().strip_prefix(&current_dir).unwrap();

        let measured = ScanEngine::new()
            .measure_scan_with_backend(
                relative_path,
                &ScanCancellationToken::new(),
                ScanBackendKind::WindowsNative,
                |_| {},
            )
            .unwrap();

        assert_eq!(measured.report.bytes_scanned, 4);
        assert_eq!(measured.backend, ScanBackendKind::PortableRecursive);
        assert!(
            measured
                .fallback_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("absolute local path"))
        );
    }

    #[cfg(not(windows))]
    {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("a.txt"), b"abcd").unwrap();

        let measured = ScanEngine::new()
            .measure_scan_with_backend(
                temp.path(),
                &ScanCancellationToken::new(),
                ScanBackendKind::WindowsNative,
                |_| {},
            )
            .unwrap();

        assert_eq!(measured.report.bytes_scanned, 4);
        assert_eq!(measured.backend, ScanBackendKind::PortableRecursive);
        assert!(
            measured
                .fallback_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("only available on Windows"))
        );
    }
}

#[test]
fn windows_ntfs_mft_experimental_selection_falls_back_with_caveat() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("a.txt"), b"abcd").unwrap();

    let measured = ScanEngine::new()
        .measure_scan_with_backend(
            temp.path(),
            &ScanCancellationToken::new(),
            ScanBackendKind::WindowsNtfsMftExperimental,
            |_| {},
        )
        .unwrap();

    assert_eq!(measured.report.bytes_scanned, 4);
    #[cfg(windows)]
    assert_eq!(measured.backend, ScanBackendKind::WindowsNative);
    #[cfg(not(windows))]
    assert_eq!(measured.backend, ScanBackendKind::PortableRecursive);
    assert!(
        measured
            .fallback_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("windows-ntfs-mft-experimental"))
    );
    assert!(
        measured
            .caveats
            .iter()
            .any(|caveat| caveat.code == "experimental-ntfs-mft-fallback")
    );
}

#[cfg(windows)]
#[test]
fn windows_native_backend_matches_portable_fixture_tree() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("a.txt"), b"abcd").unwrap();
    fs::create_dir(temp.path().join("nested")).unwrap();
    fs::write(temp.path().join("nested").join("b.txt"), b"ef").unwrap();

    let engine = ScanEngine::new();
    let portable = engine.measure_scan(temp.path()).unwrap();
    let mut progress_events = 0_u64;
    let native = engine
        .measure_scan_with_backend(
            temp.path(),
            &ScanCancellationToken::new(),
            ScanBackendKind::WindowsNative,
            |event| match event {
                ScanProgressEvent::FileMeasured { .. } => {
                    progress_events = progress_events.saturating_add(1);
                }
            },
        )
        .unwrap();

    assert_eq!(native.report, portable.report);
    assert_eq!(native.backend, ScanBackendKind::WindowsNative);
    assert_eq!(native.fallback_reason, None);
    assert_eq!(progress_events, native.report.files_scanned);
}

#[cfg(windows)]
#[test]
fn windows_native_backend_skips_child_reparse_paths_like_portable() {
    use std::os::windows::fs::symlink_dir;

    let temp = tempfile::tempdir().unwrap();
    let real = temp.path().join("real");
    let link = temp.path().join("link");
    fs::create_dir(&real).unwrap();
    fs::write(real.join("a.txt"), b"abcd").unwrap();

    if symlink_dir(&real, &link).is_err() {
        return;
    }

    let engine = ScanEngine::new();
    let portable = engine.measure_scan(temp.path()).unwrap();
    let native = engine
        .measure_scan_with_backend(
            temp.path(),
            &ScanCancellationToken::new(),
            ScanBackendKind::WindowsNative,
            |_| {},
        )
        .unwrap();

    assert_eq!(native.report, portable.report);
    assert_eq!(native.report.bytes_scanned, 4);
    assert_eq!(native.report.files_scanned, 1);
    assert_eq!(native.report.directories_scanned, 2);
}

#[cfg(windows)]
#[test]
fn windows_native_backend_preserves_cancelled_scan_error() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("a.txt"), b"abcd").unwrap();

    let token = ScanCancellationToken::new();
    token.cancel();
    let err = ScanEngine::new()
        .measure_scan_with_backend(temp.path(), &token, ScanBackendKind::WindowsNative, |_| {})
        .unwrap_err();

    assert!(matches!(err, RebeccaError::OperationCancelled(_)));
}

#[test]
fn measurement_counts_entries_ignored_by_gitignore() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join(".git")).unwrap();
    fs::write(temp.path().join(".gitignore"), b"ignored.bin\n").unwrap();
    fs::write(temp.path().join("ignored.bin"), b"abcd").unwrap();
    fs::write(temp.path().join("kept.bin"), b"ef").unwrap();

    let report = ScanEngine::new().measure_path(temp.path()).unwrap();

    assert_eq!(report.bytes_scanned, 18);
    assert_eq!(report.files_scanned, 3);
    assert_eq!(report.directories_scanned, 2);
}

#[test]
fn measurement_counts_entries_ignored_by_ignore_file() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join(".ignore"), b"ignored-by-ignore.bin\n").unwrap();
    fs::write(temp.path().join("ignored-by-ignore.bin"), b"abcd").unwrap();
    fs::write(temp.path().join("kept.bin"), b"ef").unwrap();

    let report = ScanEngine::new().measure_path(temp.path()).unwrap();

    assert_eq!(report.bytes_scanned, 28);
    assert_eq!(report.files_scanned, 3);
    assert_eq!(report.directories_scanned, 1);
}

#[test]
fn measurement_counts_hidden_files() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join(".hidden.bin"), b"abcd").unwrap();
    fs::write(temp.path().join("visible.bin"), b"ef").unwrap();

    let report = ScanEngine::new().measure_path(temp.path()).unwrap();

    assert_eq!(report.bytes_scanned, 6);
    assert_eq!(report.files_scanned, 2);
    assert_eq!(report.directories_scanned, 1);
}

#[test]
fn missing_path_size_reports_structured_scan_failure() {
    let temp = tempfile::tempdir().unwrap();
    let missing = temp.path().join("missing");

    let err = ScanEngine::new().measure_path(&missing).unwrap_err();

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
    let report = ScanEngine::new()
        .measure_path_with_progress(temp.path(), &token, |event| match event {
            ScanProgressEvent::FileMeasured {
                file_size,
                files_scanned,
                bytes_scanned,
                ..
            } => events.push((file_size, files_scanned, bytes_scanned)),
        })
        .unwrap();

    assert_eq!(report.bytes_scanned, 6);
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
    let err = ScanEngine::new()
        .measure_path_with_progress(temp.path(), &token, |event| match event {
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
    let target = ScanEngine::new().measure_target(ScanTargetRequest::new(
        "windows.user-temp",
        temp.path().join("missing"),
        DeleteMode::DryRun,
    ));

    assert_eq!(target.status, TargetStatus::Skipped);
}

#[test]
fn scan_targets_returns_deterministic_ordering() {
    let temp = tempfile::tempdir().unwrap();
    let first = temp.path().join("first");
    let second = temp.path().join("second");
    fs::create_dir(&first).unwrap();
    fs::create_dir(&second).unwrap();

    let targets = ScanEngine::new().measure_targets(vec![
        ScanTargetRequest::new("windows.z", second, DeleteMode::DryRun),
        ScanTargetRequest::new("windows.a", first, DeleteMode::DryRun),
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

    let target = ScanEngine::new().measure_target(ScanTargetRequest::new(
        "windows.user-temp",
        link,
        DeleteMode::DryRun,
    ));

    assert_eq!(target.status, TargetStatus::Blocked);
}

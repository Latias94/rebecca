use std::fs;
use std::path::Path;
use std::time::Duration;

use criterion::{BatchSize, Criterion, black_box, criterion_group, criterion_main};
use rebecca_core::scan::{
    ScanCancellationToken, ScanProgressEvent, measure_path, measure_path_with_progress,
    scan_targets,
};
use rebecca_core::{DeleteMode, TargetStatus};

const DIRECTORY_COUNT: usize = 32;
const FILES_PER_DIRECTORY: usize = 32;
const BYTES_PER_FILE: usize = 128;

fn scan_baseline(criterion: &mut Criterion) {
    criterion.bench_function("scan_report_1024_files", |bencher| {
        let fixture = tempfile::tempdir().expect("benchmark fixture should be created");
        let expected = create_fixture(fixture.path());

        bencher.iter(|| {
            let report = measure_path(black_box(fixture.path())).expect("scan should succeed");
            assert_eq!(report.files_scanned, expected.files);
            assert_eq!(report.directories_scanned, expected.directories);
            assert_eq!(report.bytes_scanned, expected.bytes);
            black_box(report);
        });
    });

    criterion.bench_function("scan_report_with_file_progress_1024_files", |bencher| {
        let fixture = tempfile::tempdir().expect("benchmark fixture should be created");
        let expected = create_fixture(fixture.path());

        bencher.iter(|| {
            let mut progress_events = 0u64;
            let cancellation = ScanCancellationToken::new();
            let report =
                measure_path_with_progress(black_box(fixture.path()), &cancellation, |event| {
                    match event {
                        ScanProgressEvent::FileMeasured { .. } => {
                            progress_events = progress_events.saturating_add(1);
                        }
                    }
                })
                .expect("scan should succeed");

            assert_eq!(report.files_scanned, expected.files);
            assert_eq!(progress_events, expected.files);
            assert_eq!(report.bytes_scanned, expected.bytes);
            black_box((report, progress_events));
        });
    });

    criterion.bench_function("scan_targets_parallel_1024_files", |bencher| {
        let fixture = tempfile::tempdir().expect("benchmark fixture should be created");
        let expected = create_fixture(fixture.path());
        let scan_targets_fixture =
            create_scan_targets_fixture(fixture.path(), expected.files as usize);

        bencher.iter_batched(
            || scan_targets_fixture.clone(),
            |targets| {
                let scanned = scan_targets(black_box(targets));

                assert_eq!(scanned.len(), expected.files as usize);
                assert!(
                    scanned
                        .iter()
                        .all(|target| matches!(target.status, TargetStatus::Allowed))
                );
                let total: u64 = scanned.iter().map(|target| target.estimated_bytes).sum();
                assert_eq!(total, expected.bytes);
                black_box(scanned);
            },
            BatchSize::SmallInput,
        );
    });
}

#[derive(Debug, Clone, Copy)]
struct ExpectedScan {
    bytes: u64,
    files: u64,
    directories: u64,
}

fn create_scan_targets_fixture(
    root: &Path,
    count: usize,
) -> Vec<(String, std::path::PathBuf, DeleteMode)> {
    let mut targets = Vec::with_capacity(count);

    for file_index in 0..count {
        let directory_index = file_index / FILES_PER_DIRECTORY;
        let file_name_index = file_index % FILES_PER_DIRECTORY;
        let path = root
            .join(format!("dir-{directory_index:02}"))
            .join(format!("file-{file_name_index:02}.bin"));
        targets.push((format!("rule-{file_index:04}"), path, DeleteMode::DryRun));
    }

    targets
}

fn create_fixture(root: &Path) -> ExpectedScan {
    let mut bytes = 0u64;
    let mut files = 0u64;

    for directory_index in 0..DIRECTORY_COUNT {
        let directory = root.join(format!("dir-{directory_index:02}"));
        fs::create_dir_all(&directory).expect("benchmark directory should be created");

        for file_index in 0..FILES_PER_DIRECTORY {
            let file = directory.join(format!("file-{file_index:02}.bin"));
            fs::write(&file, vec![directory_index as u8; BYTES_PER_FILE])
                .expect("benchmark file should be written");
            bytes = bytes.saturating_add(BYTES_PER_FILE as u64);
            files = files.saturating_add(1);
        }
    }

    ExpectedScan {
        bytes,
        files,
        directories: DIRECTORY_COUNT as u64 + 1,
    }
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .sample_size(10)
        .measurement_time(Duration::from_secs(1))
        .warm_up_time(Duration::from_millis(500));
    targets = scan_baseline
}
criterion_main!(benches);

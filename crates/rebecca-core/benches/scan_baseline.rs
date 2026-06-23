use std::fs;
use std::path::Path;

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use rebecca_core::scan::{
    ScanCancellationToken, ScanProgressEvent, measure_path, measure_path_with_progress,
};

const DIRECTORY_COUNT: usize = 32;
const FILES_PER_DIRECTORY: usize = 32;
const BYTES_PER_FILE: usize = 128;

fn scan_baseline(criterion: &mut Criterion) {
    let fixture = tempfile::tempdir().expect("benchmark fixture should be created");
    let expected = create_fixture(fixture.path());

    criterion.bench_function("scan_report_1024_files", |bencher| {
        bencher.iter(|| {
            let report = measure_path(black_box(fixture.path())).expect("scan should succeed");
            assert_eq!(report.files_scanned, expected.files);
            assert_eq!(report.directories_scanned, expected.directories);
            assert_eq!(report.bytes_scanned, expected.bytes);
            black_box(report);
        });
    });

    criterion.bench_function("scan_report_with_file_progress_1024_files", |bencher| {
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
}

#[derive(Debug, Clone, Copy)]
struct ExpectedScan {
    bytes: u64,
    files: u64,
    directories: u64,
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

criterion_group!(benches, scan_baseline);
criterion_main!(benches);

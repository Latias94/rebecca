use std::fs;
use std::path::Path;
use std::time::Duration;

use criterion::{BatchSize, Criterion, black_box, criterion_group, criterion_main};
use rebecca_core::executor::{
    CleanupBackend, ExecutionOutcome, execute_cleanup_plan_parallel_with_policy,
    execute_cleanup_plan_with_policy,
};
use rebecca_core::plan::{CleanupPlan, CleanupTarget, CleanupTargetDeletionStyle};
use rebecca_core::protection::ProtectionPolicy;
use rebecca_core::scan::{ScanCancellationToken, ScanEngine, ScanProgressEvent, ScanTargetRequest};
use rebecca_core::{DeleteMode, TargetStatus};

const DIRECTORY_COUNT: usize = 32;
const FILES_PER_DIRECTORY: usize = 32;
const BYTES_PER_FILE: usize = 128;

fn scan_baseline(criterion: &mut Criterion) {
    criterion.bench_function("scan_report_1024_files", |bencher| {
        let fixture = tempfile::tempdir().expect("benchmark fixture should be created");
        let expected = create_fixture(fixture.path());

        bencher.iter(|| {
            let report = ScanEngine::new()
                .measure_path(black_box(fixture.path()))
                .expect("scan should succeed");
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
            let report = ScanEngine::new()
                .measure_path_with_progress(black_box(fixture.path()), &cancellation, |event| {
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
                let scanned = ScanEngine::new().measure_targets(black_box(targets));

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

    criterion.bench_function("cleanup_delete_serial_1024_files", |bencher| {
        bencher.iter_batched(
            create_cleanup_fixture,
            |mut fixture| {
                execute_cleanup_plan_with_policy(
                    &mut fixture.plan,
                    &fixture.backend,
                    ProtectionPolicy::new(),
                )
                .expect("cleanup should succeed");

                assert_eq!(fixture.plan.summary.completed_targets, DIRECTORY_COUNT);
                assert_eq!(fixture.plan.summary.allowed_targets, 0);
                black_box(fixture);
            },
            BatchSize::SmallInput,
        );
    });

    criterion.bench_function("cleanup_delete_parallel_1024_files", |bencher| {
        bencher.iter_batched(
            create_cleanup_fixture,
            |mut fixture| {
                execute_cleanup_plan_parallel_with_policy(
                    &mut fixture.plan,
                    &fixture.backend,
                    ProtectionPolicy::new(),
                )
                .expect("cleanup should succeed");

                assert_eq!(fixture.plan.summary.completed_targets, DIRECTORY_COUNT);
                assert_eq!(fixture.plan.summary.allowed_targets, 0);
                black_box(fixture);
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

#[derive(Debug, Clone, Copy)]
struct CleanupFixtureBackend;

#[derive(Debug)]
struct CleanupBenchmarkFixture {
    _temp: tempfile::TempDir,
    plan: CleanupPlan,
    backend: CleanupFixtureBackend,
}

impl CleanupBackend for CleanupFixtureBackend {
    fn delete(&self, target: &CleanupTarget) -> rebecca_core::Result<ExecutionOutcome> {
        match target.deletion_style {
            CleanupTargetDeletionStyle::DeleteWholePath => {
                if target.path.is_dir() {
                    fs::remove_dir_all(&target.path).expect("benchmark delete should succeed");
                } else {
                    fs::remove_file(&target.path).expect("benchmark delete should succeed");
                }
            }
            CleanupTargetDeletionStyle::PreserveRootContents => {
                if target.path.is_dir() {
                    for entry in
                        fs::read_dir(&target.path).expect("benchmark directory should be readable")
                    {
                        let entry = entry.expect("benchmark entry should be readable");
                        let entry_path = entry.path();
                        if entry_path.is_dir() {
                            fs::remove_dir_all(&entry_path)
                                .expect("benchmark delete should succeed");
                        } else {
                            fs::remove_file(&entry_path).expect("benchmark delete should succeed");
                        }
                    }
                } else {
                    fs::remove_file(&target.path).expect("benchmark delete should succeed");
                }
            }
        }

        Ok(ExecutionOutcome {
            freed_bytes: 0,
            pending_reclaim_bytes: target.estimated_bytes,
            note: Some("moved to recycle bin".to_string()),
        })
    }
}

fn create_scan_targets_fixture(root: &Path, count: usize) -> Vec<ScanTargetRequest> {
    let mut targets = Vec::with_capacity(count);

    for file_index in 0..count {
        let directory_index = file_index / FILES_PER_DIRECTORY;
        let file_name_index = file_index % FILES_PER_DIRECTORY;
        let path = root
            .join(format!("dir-{directory_index:02}"))
            .join(format!("file-{file_name_index:02}.bin"));
        targets.push(ScanTargetRequest::new(
            format!("rule-{file_index:04}"),
            path,
            DeleteMode::DryRun,
        ));
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

fn create_cleanup_fixture() -> CleanupBenchmarkFixture {
    let temp = tempfile::tempdir().expect("benchmark fixture should be created");
    let root = temp.path();
    let mut plan = CleanupPlan::empty(rebecca_core::PlanRequest::for_platform(
        rebecca_core::Platform::Windows,
        DeleteMode::RecycleBin,
    ));

    for directory_index in 0..DIRECTORY_COUNT {
        let directory = root.join(format!("dir-{directory_index:02}"));
        fs::create_dir_all(&directory).expect("benchmark directory should be created");

        for file_index in 0..FILES_PER_DIRECTORY {
            let file = directory.join(format!("file-{file_index:02}.bin"));
            fs::write(&file, vec![directory_index as u8; BYTES_PER_FILE])
                .expect("benchmark file should be written");
        }

        plan.targets.push(CleanupTarget::allowed(
            format!("rule-{directory_index:04}"),
            directory,
            BYTES_PER_FILE as u64 * FILES_PER_DIRECTORY as u64,
            DeleteMode::RecycleBin,
        ));
    }

    plan.recompute_summary();
    CleanupBenchmarkFixture {
        _temp: temp,
        plan,
        backend: CleanupFixtureBackend,
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

use std::fs;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use rebecca_core::executor::{
    CleanupBackend, ExecutionOutcome, execute_cleanup_plan_parallel_with_policy,
    execute_cleanup_plan_with_policy,
};
use rebecca_core::plan::{CleanupPlan, CleanupTarget, CleanupTargetDeletionStyle};
use rebecca_core::planner::{
    PlanProgressEvent, build_cleanup_plan, build_cleanup_plan_with_progress,
};
use rebecca_core::protection::ProtectionPolicy;
use rebecca_core::scan::{
    ScanBackendKind, ScanCancellationToken, ScanEngine, ScanProgressEvent, ScanTargetRequest,
};
use rebecca_core::scan_cache::{ScanCacheLookup, ScanCacheStore};
use rebecca_core::{
    DeleteMode, PlanRequest, Platform, RuleDefinition, RuleProvenance, RuleSource, RuleTargetSpec,
    SafetyLevel, TargetStatus,
};
use serde::Serialize;

const MANY_SMALL_DIRECTORY_COUNT: usize = 32;
const FILES_PER_DIRECTORY: usize = 32;
const LARGE_DIRECTORY_FILES: usize = 1024;
const DEEP_DIRECTORY_LEVELS: usize = 32;
const FILES_PER_DEEP_DIRECTORY: usize = 4;
const DUPLICATE_TARGET_UNIQUE_FILES: usize = 32;
const DUPLICATE_TARGET_REPEATS: usize = 32;
const BYTES_PER_FILE: usize = 128;

const SCENARIOS: &[ScenarioMetadata] = &[
    ScenarioMetadata::scan(
        "many_small_cold_scan_1024_files",
        "many-small",
        1024,
        33,
        131_072,
        0,
    ),
    ScenarioMetadata::scan_backend(
        "many_small_windows_native_scan_1024_files",
        "windows-native-selected",
        "many-small",
        1024,
        33,
        131_072,
        0,
    ),
    ScenarioMetadata::scan(
        "many_small_progress_scan_1024_files",
        "many-small",
        1024,
        33,
        131_072,
        1024,
    ),
    ScenarioMetadata::scan(
        "large_single_directory_cold_scan_1024_files",
        "large-single-directory",
        1024,
        1,
        131_072,
        0,
    ),
    ScenarioMetadata::scan(
        "deep_tree_cold_scan_128_files",
        "deep-tree",
        128,
        33,
        16_384,
        0,
    ),
    ScenarioMetadata::targets(
        "scan_targets_parallel_1024_files",
        "many-small",
        1024,
        33,
        131_072,
        1024,
    ),
    ScenarioMetadata::targets(
        "scan_targets_duplicate_paths_1024_candidates",
        "duplicate-targets",
        32,
        2,
        131_072,
        1024,
    ),
    ScenarioMetadata::rule_plan(
        "rule_plan_32_dirs_1024_files",
        "many-small-directories",
        1024,
        33,
        131_072,
        32,
    ),
    ScenarioMetadata::rule_plan_progress(
        "rule_plan_target_progress_32_dirs_1024_files",
        "many-small-directories",
        1024,
        33,
        131_072,
        32,
        64,
    ),
    ScenarioMetadata::cache(
        "scan_cache_miss_store_many_small_1024_files",
        "many-small",
        1024,
        33,
        131_072,
        "miss-store",
    ),
    ScenarioMetadata::cache(
        "scan_cache_hit_many_small_1024_files",
        "many-small",
        1024,
        33,
        131_072,
        "hit",
    ),
    ScenarioMetadata::delete(
        "cleanup_delete_serial_32_dirs_1024_files",
        "many-small-directories",
        1024,
        33,
        131_072,
        "serial-recycle",
        32,
    ),
    ScenarioMetadata::delete(
        "cleanup_delete_parallel_32_dirs_1024_files",
        "many-small-directories",
        1024,
        33,
        131_072,
        "parallel-recycle",
        32,
    ),
    ScenarioMetadata::delete(
        "cleanup_delete_batch_32_dirs_1024_files",
        "many-small-directories",
        1024,
        33,
        131_072,
        "batch-recycle",
        32,
    ),
];

fn perf_matrix(criterion: &mut Criterion) {
    write_scenario_manifest();

    let mut group = criterion.benchmark_group("perf_matrix");
    group.sample_size(10);
    group.measurement_time(Duration::from_millis(750));
    group.warm_up_time(Duration::from_millis(250));

    group.bench_function("many_small_cold_scan_1024_files", |bencher| {
        let fixture = tempfile::tempdir().expect("benchmark fixture should be created");
        let expected = create_many_small_fixture(fixture.path());

        bencher.iter(|| {
            let report = ScanEngine::new()
                .measure_path(black_box(fixture.path()))
                .expect("scan should succeed");
            assert_report(report, expected);
            black_box(report);
        });
    });

    group.bench_function("many_small_windows_native_scan_1024_files", |bencher| {
        let fixture = tempfile::tempdir().expect("benchmark fixture should be created");
        let expected = create_many_small_fixture(fixture.path());

        bencher.iter(|| {
            let measured = ScanEngine::new()
                .measure_scan_with_backend(
                    black_box(fixture.path()),
                    &ScanCancellationToken::new(),
                    ScanBackendKind::WindowsNative,
                    |_| {},
                )
                .expect("scan should succeed");
            assert_report(measured.report, expected);
            #[cfg(windows)]
            assert_eq!(measured.backend, ScanBackendKind::WindowsNative);
            #[cfg(not(windows))]
            assert_eq!(measured.backend, ScanBackendKind::PortableRecursive);
            black_box(measured);
        });
    });

    group.bench_function("many_small_progress_scan_1024_files", |bencher| {
        let fixture = tempfile::tempdir().expect("benchmark fixture should be created");
        let expected = create_many_small_fixture(fixture.path());

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

            assert_report(report, expected);
            assert_eq!(progress_events, expected.files);
            black_box((report, progress_events));
        });
    });

    group.bench_function("large_single_directory_cold_scan_1024_files", |bencher| {
        let fixture = tempfile::tempdir().expect("benchmark fixture should be created");
        let expected = create_large_single_directory_fixture(fixture.path());

        bencher.iter(|| {
            let report = ScanEngine::new()
                .measure_path(black_box(fixture.path()))
                .expect("scan should succeed");
            assert_report(report, expected);
            black_box(report);
        });
    });

    group.bench_function("deep_tree_cold_scan_128_files", |bencher| {
        let fixture = tempfile::tempdir().expect("benchmark fixture should be created");
        let expected = create_deep_tree_fixture(fixture.path());

        bencher.iter(|| {
            let report = ScanEngine::new()
                .measure_path(black_box(fixture.path()))
                .expect("scan should succeed");
            assert_report(report, expected);
            black_box(report);
        });
    });

    group.bench_function("scan_targets_parallel_1024_files", |bencher| {
        let fixture = tempfile::tempdir().expect("benchmark fixture should be created");
        let expected = create_many_small_fixture(fixture.path());
        let targets = create_scan_targets_fixture(fixture.path(), expected.files as usize);

        bencher.iter_batched(
            || targets.clone(),
            |targets| {
                let scanned = ScanEngine::new().measure_targets(black_box(targets));
                assert_eq!(scanned.len(), expected.files as usize);
                assert!(
                    scanned
                        .iter()
                        .all(|target| target.status == TargetStatus::Allowed)
                );
                assert_eq!(
                    scanned
                        .iter()
                        .map(|target| target.estimated_bytes)
                        .sum::<u64>(),
                    expected.bytes
                );
                black_box(scanned);
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function("scan_targets_duplicate_paths_1024_candidates", |bencher| {
        let fixture = tempfile::tempdir().expect("benchmark fixture should be created");
        let expected = create_duplicate_target_fixture(fixture.path());
        let targets = create_duplicate_scan_targets_fixture(fixture.path());

        bencher.iter_batched(
            || targets.clone(),
            |targets| {
                let scanned = ScanEngine::new().measure_targets(black_box(targets));
                assert_eq!(
                    scanned.len(),
                    DUPLICATE_TARGET_UNIQUE_FILES * DUPLICATE_TARGET_REPEATS
                );
                assert!(
                    scanned
                        .iter()
                        .all(|target| target.status == TargetStatus::Allowed)
                );
                assert_eq!(
                    scanned
                        .iter()
                        .map(|target| target.estimated_bytes)
                        .sum::<u64>(),
                    expected.bytes * DUPLICATE_TARGET_REPEATS as u64
                );
                black_box(scanned);
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function("rule_plan_32_dirs_1024_files", |bencher| {
        bencher.iter_batched(
            create_rule_plan_fixture,
            |fixture| {
                let request = PlanRequest::for_platform(Platform::Windows, DeleteMode::DryRun);
                let plan = build_cleanup_plan(black_box(&request), black_box(&fixture.rules))
                    .expect("rule plan should build");

                assert_eq!(plan.summary.allowed_targets, MANY_SMALL_DIRECTORY_COUNT);
                assert_eq!(plan.summary.skipped_targets, 0);
                assert_eq!(plan.summary.estimated_bytes, fixture.expected.bytes);
                black_box((fixture, plan));
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function("rule_plan_target_progress_32_dirs_1024_files", |bencher| {
        bencher.iter_batched(
            create_rule_plan_fixture,
            |fixture| {
                let request = PlanRequest::for_platform(Platform::Windows, DeleteMode::DryRun);
                let mut emitted_progress_events = 0_u64;
                let plan = build_cleanup_plan_with_progress(
                    black_box(&request),
                    black_box(&fixture.rules),
                    |event| {
                        if !matches!(event, PlanProgressEvent::FileMeasured { .. }) {
                            emitted_progress_events = emitted_progress_events.saturating_add(1);
                        }
                    },
                )
                .expect("rule plan should build");

                assert_eq!(plan.summary.allowed_targets, MANY_SMALL_DIRECTORY_COUNT);
                assert_eq!(plan.summary.skipped_targets, 0);
                assert_eq!(plan.summary.estimated_bytes, fixture.expected.bytes);
                assert_eq!(
                    emitted_progress_events,
                    (MANY_SMALL_DIRECTORY_COUNT * 2) as u64
                );
                black_box((fixture, plan, emitted_progress_events));
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function("scan_cache_miss_store_many_small_1024_files", |bencher| {
        let fixture = tempfile::tempdir().expect("benchmark fixture should be created");
        let expected = create_many_small_fixture(fixture.path());

        bencher.iter_batched(
            || tempfile::tempdir().expect("cache fixture should be created"),
            |cache_fixture| {
                let store = ScanCacheStore::new(cache_fixture.path().join("scan"));
                assert!(matches!(
                    store.load(fixture.path()),
                    ScanCacheLookup::Miss(_)
                ));
                let report = ScanEngine::new()
                    .measure_path(black_box(fixture.path()))
                    .expect("scan should succeed");
                assert_report(report, expected);
                let record = store
                    .store(fixture.path(), report)
                    .expect("cache write should succeed");
                assert_eq!(record.report, report);
                black_box((cache_fixture, record));
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function("scan_cache_hit_many_small_1024_files", |bencher| {
        let fixture = tempfile::tempdir().expect("benchmark fixture should be created");
        let expected = create_many_small_fixture(fixture.path());
        let report = ScanEngine::new()
            .measure_path(fixture.path())
            .expect("scan should succeed");
        assert_report(report, expected);

        bencher.iter_batched(
            || {
                let cache_fixture = tempfile::tempdir().expect("cache fixture should be created");
                let store = ScanCacheStore::new(cache_fixture.path().join("scan"));
                store
                    .store(fixture.path(), report)
                    .expect("cache write should succeed");
                (cache_fixture, store)
            },
            |(cache_fixture, store)| {
                let ScanCacheLookup::Hit(cached_report) = store.load(black_box(fixture.path()))
                else {
                    panic!("cache lookup should hit");
                };
                assert_report(cached_report, expected);
                black_box((cache_fixture, cached_report));
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function("cleanup_delete_serial_32_dirs_1024_files", |bencher| {
        bencher.iter_batched(
            create_cleanup_fixture,
            |mut fixture| {
                execute_cleanup_plan_with_policy(
                    &mut fixture.plan,
                    &fixture.backend,
                    ProtectionPolicy::new(),
                )
                .expect("cleanup should succeed");

                assert_eq!(
                    fixture.plan.summary.completed_targets,
                    MANY_SMALL_DIRECTORY_COUNT
                );
                assert_eq!(fixture.plan.summary.allowed_targets, 0);
                black_box(fixture);
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function("cleanup_delete_parallel_32_dirs_1024_files", |bencher| {
        bencher.iter_batched(
            create_cleanup_fixture,
            |mut fixture| {
                execute_cleanup_plan_parallel_with_policy(
                    &mut fixture.plan,
                    &fixture.backend,
                    ProtectionPolicy::new(),
                )
                .expect("cleanup should succeed");

                assert_eq!(
                    fixture.plan.summary.completed_targets,
                    MANY_SMALL_DIRECTORY_COUNT
                );
                assert_eq!(fixture.plan.summary.allowed_targets, 0);
                black_box(fixture);
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function("cleanup_delete_batch_32_dirs_1024_files", |bencher| {
        bencher.iter_batched(
            create_batch_cleanup_fixture,
            |mut fixture| {
                execute_cleanup_plan_parallel_with_policy(
                    &mut fixture.plan,
                    &fixture.backend,
                    ProtectionPolicy::new(),
                )
                .expect("cleanup should succeed");

                assert_eq!(
                    fixture.plan.summary.completed_targets,
                    MANY_SMALL_DIRECTORY_COUNT
                );
                assert_eq!(fixture.plan.summary.allowed_targets, 0);
                black_box(fixture);
            },
            BatchSize::SmallInput,
        );
    });

    group.finish();
}

#[derive(Debug, Clone, Copy)]
struct ExpectedScan {
    bytes: u64,
    files: u64,
    directories: u64,
}

#[derive(Debug, Clone, Copy)]
struct CleanupFixtureBackend {
    supports_batch_delete: bool,
}

impl CleanupFixtureBackend {
    const fn single() -> Self {
        Self {
            supports_batch_delete: false,
        }
    }

    const fn batch() -> Self {
        Self {
            supports_batch_delete: true,
        }
    }
}

#[derive(Debug)]
struct CleanupBenchmarkFixture {
    _temp: tempfile::TempDir,
    plan: CleanupPlan,
    backend: CleanupFixtureBackend,
}

#[derive(Debug)]
struct RulePlanBenchmarkFixture {
    _temp: tempfile::TempDir,
    expected: ExpectedScan,
    rules: Vec<RuleDefinition>,
}

#[derive(Debug, Clone, Copy, Serialize)]
struct ScenarioMetadata {
    scenario: &'static str,
    operation: &'static str,
    backend: &'static str,
    fixture: &'static str,
    physical_files: u64,
    physical_directories: u64,
    expected_bytes: u64,
    progress_events: u64,
    target_count: u64,
    cache_mode: &'static str,
    delete_mode: &'static str,
}

impl ScenarioMetadata {
    const fn scan(
        scenario: &'static str,
        fixture: &'static str,
        physical_files: u64,
        physical_directories: u64,
        expected_bytes: u64,
        progress_events: u64,
    ) -> Self {
        Self {
            scenario,
            operation: "scan",
            backend: "portable-recursive",
            fixture,
            physical_files,
            physical_directories,
            expected_bytes,
            progress_events,
            target_count: 1,
            cache_mode: "disabled",
            delete_mode: "none",
        }
    }

    const fn scan_backend(
        scenario: &'static str,
        backend: &'static str,
        fixture: &'static str,
        physical_files: u64,
        physical_directories: u64,
        expected_bytes: u64,
        progress_events: u64,
    ) -> Self {
        Self {
            scenario,
            operation: "scan",
            backend,
            fixture,
            physical_files,
            physical_directories,
            expected_bytes,
            progress_events,
            target_count: 1,
            cache_mode: "disabled",
            delete_mode: "none",
        }
    }

    const fn targets(
        scenario: &'static str,
        fixture: &'static str,
        physical_files: u64,
        physical_directories: u64,
        expected_bytes: u64,
        target_count: u64,
    ) -> Self {
        Self {
            scenario,
            operation: "target-scan",
            backend: "portable-recursive",
            fixture,
            physical_files,
            physical_directories,
            expected_bytes,
            progress_events: 0,
            target_count,
            cache_mode: "disabled",
            delete_mode: "none",
        }
    }

    const fn rule_plan(
        scenario: &'static str,
        fixture: &'static str,
        physical_files: u64,
        physical_directories: u64,
        expected_bytes: u64,
        target_count: u64,
    ) -> Self {
        Self {
            scenario,
            operation: "rule-plan",
            backend: "portable-recursive",
            fixture,
            physical_files,
            physical_directories,
            expected_bytes,
            progress_events: 0,
            target_count,
            cache_mode: "disabled",
            delete_mode: "dry-run",
        }
    }

    const fn rule_plan_progress(
        scenario: &'static str,
        fixture: &'static str,
        physical_files: u64,
        physical_directories: u64,
        expected_bytes: u64,
        target_count: u64,
        progress_events: u64,
    ) -> Self {
        Self {
            scenario,
            operation: "rule-plan-progress",
            backend: "portable-recursive",
            fixture,
            physical_files,
            physical_directories,
            expected_bytes,
            progress_events,
            target_count,
            cache_mode: "disabled",
            delete_mode: "dry-run",
        }
    }

    const fn cache(
        scenario: &'static str,
        fixture: &'static str,
        physical_files: u64,
        physical_directories: u64,
        expected_bytes: u64,
        cache_mode: &'static str,
    ) -> Self {
        Self {
            scenario,
            operation: "scan-cache",
            backend: "portable-recursive",
            fixture,
            physical_files,
            physical_directories,
            expected_bytes,
            progress_events: 0,
            target_count: 1,
            cache_mode,
            delete_mode: "none",
        }
    }

    const fn delete(
        scenario: &'static str,
        fixture: &'static str,
        physical_files: u64,
        physical_directories: u64,
        expected_bytes: u64,
        delete_mode: &'static str,
        target_count: u64,
    ) -> Self {
        Self {
            scenario,
            operation: "delete",
            backend: "fixture-delete",
            fixture,
            physical_files,
            physical_directories,
            expected_bytes,
            progress_events: 0,
            target_count,
            cache_mode: "disabled",
            delete_mode,
        }
    }
}

#[derive(Debug, Serialize)]
struct ScenarioManifest {
    schema_version: u32,
    generated_at_unix_seconds: u64,
    package: &'static str,
    bench: &'static str,
    scenarios: &'static [ScenarioMetadata],
}

impl CleanupBackend for CleanupFixtureBackend {
    fn delete(&self, target: &CleanupTarget) -> rebecca_core::Result<ExecutionOutcome> {
        delete_benchmark_target(target)
    }

    fn supports_batch_delete(&self) -> bool {
        self.supports_batch_delete
    }

    fn delete_batch(
        &self,
        targets: &[&CleanupTarget],
    ) -> Vec<rebecca_core::Result<ExecutionOutcome>> {
        targets
            .iter()
            .map(|target| delete_benchmark_target(target))
            .collect()
    }
}

fn delete_benchmark_target(target: &CleanupTarget) -> rebecca_core::Result<ExecutionOutcome> {
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
                        fs::remove_dir_all(&entry_path).expect("benchmark delete should succeed");
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

fn create_many_small_fixture(root: &Path) -> ExpectedScan {
    let mut bytes = 0u64;
    let mut files = 0u64;

    for directory_index in 0..MANY_SMALL_DIRECTORY_COUNT {
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
        directories: MANY_SMALL_DIRECTORY_COUNT as u64 + 1,
    }
}

fn create_large_single_directory_fixture(root: &Path) -> ExpectedScan {
    let mut bytes = 0u64;

    for file_index in 0..LARGE_DIRECTORY_FILES {
        let file = root.join(format!("file-{file_index:04}.bin"));
        fs::write(&file, vec![file_index as u8; BYTES_PER_FILE])
            .expect("benchmark file should be written");
        bytes = bytes.saturating_add(BYTES_PER_FILE as u64);
    }

    ExpectedScan {
        bytes,
        files: LARGE_DIRECTORY_FILES as u64,
        directories: 1,
    }
}

fn create_deep_tree_fixture(root: &Path) -> ExpectedScan {
    let mut bytes = 0u64;
    let mut files = 0u64;
    let mut directory = root.to_path_buf();

    for level in 0..DEEP_DIRECTORY_LEVELS {
        for file_index in 0..FILES_PER_DEEP_DIRECTORY {
            let file = directory.join(format!("file-{level:02}-{file_index:02}.bin"));
            fs::write(&file, vec![level as u8; BYTES_PER_FILE])
                .expect("benchmark file should be written");
            bytes = bytes.saturating_add(BYTES_PER_FILE as u64);
            files = files.saturating_add(1);
        }

        directory = directory.join(format!("level-{level:02}"));
        fs::create_dir_all(&directory).expect("benchmark directory should be created");
    }

    ExpectedScan {
        bytes,
        files,
        directories: DEEP_DIRECTORY_LEVELS as u64 + 1,
    }
}

fn create_duplicate_target_fixture(root: &Path) -> ExpectedScan {
    let directory = root.join("duplicates");
    fs::create_dir_all(&directory).expect("benchmark directory should be created");

    for file_index in 0..DUPLICATE_TARGET_UNIQUE_FILES {
        let file = directory.join(format!("file-{file_index:02}.bin"));
        fs::write(&file, vec![file_index as u8; BYTES_PER_FILE])
            .expect("benchmark file should be written");
    }

    ExpectedScan {
        bytes: DUPLICATE_TARGET_UNIQUE_FILES as u64 * BYTES_PER_FILE as u64,
        files: DUPLICATE_TARGET_UNIQUE_FILES as u64,
        directories: 2,
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

fn create_duplicate_scan_targets_fixture(root: &Path) -> Vec<ScanTargetRequest> {
    let mut targets = Vec::with_capacity(DUPLICATE_TARGET_UNIQUE_FILES * DUPLICATE_TARGET_REPEATS);

    for repeat in 0..DUPLICATE_TARGET_REPEATS {
        for file_index in 0..DUPLICATE_TARGET_UNIQUE_FILES {
            let path = root
                .join("duplicates")
                .join(format!("file-{file_index:02}.bin"));
            targets.push(ScanTargetRequest::new(
                format!("rule-{repeat:02}-{file_index:02}"),
                path,
                DeleteMode::DryRun,
            ));
        }
    }

    targets
}

fn create_rule_plan_fixture() -> RulePlanBenchmarkFixture {
    let temp = tempfile::tempdir().expect("benchmark fixture should be created");
    let root = temp.path();
    let expected = create_many_small_fixture(root);
    let rules = (0..MANY_SMALL_DIRECTORY_COUNT)
        .map(|directory_index| {
            benchmark_exact_path_rule(
                format!("windows.benchmark-cache-{directory_index:02}"),
                root.join(format!("dir-{directory_index:02}")),
            )
        })
        .collect();

    RulePlanBenchmarkFixture {
        _temp: temp,
        expected,
        rules,
    }
}

fn benchmark_exact_path_rule(id: String, path: PathBuf) -> RuleDefinition {
    RuleDefinition {
        id,
        platform: Platform::Windows,
        category: "benchmark".to_string(),
        name: "Benchmark cache".to_string(),
        safety_level: SafetyLevel::Safe,
        path_templates: vec![RuleTargetSpec::ExactPath(path)],
        restore_hint: Some("benchmark fixture can be rebuilt".to_string()),
        warnings: Vec::new(),
        provenance: RuleProvenance {
            source: RuleSource::Owned,
            license: "test-fixture".to_string(),
            notes: "benchmark-only rule".to_string(),
        },
    }
}

fn create_cleanup_fixture() -> CleanupBenchmarkFixture {
    let temp = tempfile::tempdir().expect("benchmark fixture should be created");
    let root = temp.path();
    let mut plan = CleanupPlan::empty(rebecca_core::PlanRequest::for_platform(
        rebecca_core::Platform::Windows,
        DeleteMode::RecycleBin,
    ));

    for directory_index in 0..MANY_SMALL_DIRECTORY_COUNT {
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
        backend: CleanupFixtureBackend::single(),
    }
}

fn create_batch_cleanup_fixture() -> CleanupBenchmarkFixture {
    let mut fixture = create_cleanup_fixture();
    fixture.backend = CleanupFixtureBackend::batch();
    fixture
}

fn assert_report(actual: rebecca_core::scan::ScanReport, expected: ExpectedScan) {
    assert_eq!(actual.files_scanned, expected.files);
    assert_eq!(actual.directories_scanned, expected.directories);
    assert_eq!(actual.bytes_scanned, expected.bytes);
}

fn write_scenario_manifest() {
    let manifest_path = scenario_manifest_path();
    let Some(parent) = manifest_path.parent() else {
        return;
    };
    fs::create_dir_all(parent).expect("perf manifest directory should be created");

    let manifest = ScenarioManifest {
        schema_version: 1,
        generated_at_unix_seconds: unix_now(),
        package: "rebecca-core",
        bench: "perf_matrix",
        scenarios: SCENARIOS,
    };
    let raw = serde_json::to_vec_pretty(&manifest).expect("perf manifest should serialize");
    fs::write(manifest_path, raw).expect("perf manifest should be written");
}

fn scenario_manifest_path() -> PathBuf {
    if let Some(path) = std::env::var_os("REBECCA_PERF_MATRIX_MANIFEST") {
        return PathBuf::from(path);
    }

    workspace_root()
        .join("target")
        .join("perf")
        .join("perf_matrix-scenarios.json")
}

fn workspace_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .and_then(Path::parent)
        .expect("rebecca-core should live under crates/")
        .to_path_buf()
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

criterion_group!(benches, perf_matrix);
criterion_main!(benches);

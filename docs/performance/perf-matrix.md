# Rebecca Performance Matrix

The performance matrix is the product-level baseline for scan, cache, and cleanup execution work. It is intentionally synthetic and deterministic so later refactors can compare the same shapes before using real-machine dogfood.

Run the compile check first:

```powershell
cargo check -p rebecca-core --benches
cargo check -p rebecca-ntfs --benches
```

Run the matrix and collect JSON, CSV, and Markdown reports:

```powershell
pwsh -File scripts/perf/run-benchmark-matrix.ps1
```

The script runs `cargo bench -p rebecca-core --bench perf_matrix`, reads Criterion estimates from `target/criterion/perf_matrix`, combines them with scenario metadata from `target/perf/perf_matrix-scenarios.json`, and writes `target/perf/rebecca-core-perf_matrix-report.json`, `target/perf/rebecca-core-perf_matrix-report-scenarios.csv`, and `target/perf/rebecca-core-perf_matrix-report-summary.md`.

For verification jobs that should not execute Criterion, use `-SkipRun`. If the manifest or Criterion output is absent, the script still writes a schema-v4 report with `status: "skipped"` or `status: "partial"` and exits successfully; that keeps report generation itself testable without pretending a benchmark was run.

```powershell
pwsh -File scripts/perf/run-benchmark-matrix.ps1 -SkipRun
```

Run the comparison self-test before using a benchmark report as a gate:

```powershell
pwsh -File scripts/perf/compare-benchmark-matrix.ps1 -SelfTest
```

To compare a fresh report against a local baseline, pass the baseline report to
the matrix runner:

```powershell
pwsh -File scripts/perf/run-benchmark-matrix.ps1 -BaselinePath target/perf/baseline.json
```

The report then includes a `comparison` object and writes comparison JSON, CSV,
and Markdown artifacts beside the matrix report. Scenario classifications are
`pass`, `regression`, `improvement`, `skipped`, `missing-baseline`, or
`missing-current`. Regressions are based on mean Criterion nanoseconds and
default to a 10% threshold; missing baseline/current scenarios are classified
explicitly instead of being treated as a pass.

The report records scenario name, operation, requested backend, backend-source expectation, fixture shape, physical files and directories, expected bytes, progress-event count, target count, cache mode, delete mode, expected estimate confidence, scan-cache miss/write expectations, per-scenario status, status reason, Criterion mean/median timing estimates when available, and a nested `evidence` object for backend, traversal, cache, delete, and timing evidence. The default scenarios cover:

- cold recursive scan over many small files
- Windows native directory scan selection over many small files
- experimental NTFS/MFT scan selection with safe fallback over many small files
- recursive scan with file-level progress callbacks
- one large flat directory
- a deep directory tree
- parallel target measurement
- duplicate target candidates
- ordinary rule planning over many directory targets
- target-level rule-planning progress over many directory targets
- Linux ordinary rule planning over many directory targets
- Linux target-level rule-planning progress over many directory targets
- scan-cache miss plus store
- scan-cache hit
- serial cleanup deletion
- parallel cleanup deletion
- batch-backend cleanup deletion

Keep reports under `target/perf/`; they are local measurement artifacts and should not be committed unless a future release process explicitly asks for a checked-in baseline.

For CI evidence, use the manual `Release Gates` workflow instead of committing
benchmark output. A successful `benchmark=full` run uploads a `release-gates`
artifact that contains `perf/rebecca-core-perf_matrix-report.json`. Treat one
successful full run on `main` as the blessed baseline for the next release
candidate, then pass that run id as `baseline_artifact_run_id` on the next full
workflow run. The workflow downloads the prior `release-gates` artifact, finds
the baseline JSON report, and lets the existing comparator classify every
scenario as pass, regression, improvement, skipped, missing-baseline, or
missing-current with the normal 10% threshold.

The default matrix does not read a live NTFS volume because that requires host privileges and can make Criterion results depend on the whole workstation disk.
Use the inspect-map dogfood script for live `windows-ntfs-mft-experimental` evidence, then compare the JSON `estimate_backend`, `estimate_backend_source`, `estimate_fallback_reason`, and `estimate_caveats` fields against the portable and Windows native scenarios. The script enables the `ntfs` Cargo feature only for the experimental backend run.
NTFS parser-core performance work should keep the first-party parser path distinguishable from any future external adapter or oracle path in report labels before adding new speed thresholds.
The script runs one `inspect map --format json` scan per backend/repetition, then derives JSON, CSV, and Markdown artifacts without re-running table mode:

```powershell
pwsh -File scripts/dogfood/run-inspect-map-report.ps1 -Root docs -Backend portable-recursive,windows-native,windows-ntfs-mft-experimental -Repeat 1 -Top 20 -GroupBy extension,depth,age -DiagnosticLimit 0
```

The report is written under `target/inspect-map-dogfood/` and includes raw JSON stdout/stderr, run-level CSV, entry/group row CSV, a Markdown summary, requested versus actual backend fields, normalized `backend_source_kind`, diagnostic summary totals, fallback reasons, caveat code counts, NTFS full-index and mirror evidence fields, NTFS stage timing and build-metric strings, duration, throughput, allocated/unique metric deltas, repeat statistics, and portable-baseline comparison status.
When scanning a root that contains the default report directory, pass an
external `-OutputDirectory` or explicitly opt into `-AllowOutputInsideRoot`.
Backend mismatches or missing portable baselines are non-zero by default; pass
`-AllowMismatch` only for exploratory profiling runs where the report itself is
the artifact being collected.
For a repeatable live NTFS fixture that exercises hardlinks, sparse allocation,
compression, large directories, nested directories, and best-effort fragmentation
candidates before running the same backend comparison report, use:

```powershell
pwsh -File scripts/dogfood/run-ntfs-fixture-dogfood.ps1 -Repeat 1 -LargeFileCount 128 -Top 20 -DiagnosticLimit 0
```

The fixture wrapper writes `ntfs-fixture-manifest.json` beside the generated
files and leaves both fixtures and reports under `target/`.
The experimental backend has its own 20 second live metadata budget before
falling back to a directory scanner; set `REBECCA_NTFS_MFT_INDEX_TIMEOUT_SECONDS`
higher for deep profiling or `0` to disable that guard for one process.
Set `REBECCA_NTFS_MFT_INDEX_TIMINGS=1` for live dogfood when you need stage
timings in timeout fallback reasons or an opt-in `mft-index-build-timing`
caveat on successful experimental runs.
When those diagnostics are present, dogfood normalizes `completed_timings=` into
`ntfs_mft_stage_timings` and `metrics=` into `ntfs_mft_build_metrics` in JSON,
CSV, and Markdown reports. The metrics are read-only performance evidence for
questions such as raw `$MFT` read volume, parsed record count, targeted record
probe count, full-index FSCTL probe count, `$MFTMirr` bytes, and stream-source
read fanout; they do not change cleanup eligibility or reclaim estimates.
Set `REBECCA_NTFS_MFT_FULL_INDEX_FALLBACK=1` only when you intentionally want a
targeted-traversal failure to try the older full-volume MFT index path before
directory-scanner fallback.

`inspect map` defaults to portable recursive inventory. Selecting
`--scan-backend windows-native` exercises the Windows find-data inventory path
and should report `estimate_backend: "windows-native"` plus non-null
`allocated_bytes` on supported local roots where file allocation metadata is
available. On hardlinked fixtures, it should also report non-null
`unique_logical_bytes` and `unique_allocated_bytes` when Windows file identity
metadata is available. Native caveats for compressed, sparse, hardlinked, or
skipped reparse entries are part of the evidence because a faster run that hides
allocation or unique-file semantics is not a valid improvement.
Grouped runs using `--group-by extension --group-by depth --group-by age`
should keep matching totals and emit bounded group summaries from the same
traversal rather than paying for a second scan. Sort variants using
`--sort files` and `--group-sort files` should only reorder bounded output
lists, not change totals or backend provenance.
Selecting `--scan-backend windows-ntfs-mft-experimental` is adaptive only in
binaries compiled with the `ntfs` Cargo feature: scoped roots should normally
report `windows-ntfs-mft-experimental-targeted-fsctl`, while drive-root maps or
explicit full-index diagnostics may report sequential or FSCTL-record full-index
sources. Builds without the feature should report portable fallback provenance
instead of loading the NTFS parser. On the earlier 2026-07-02 elevated E: run,
`target/ntfs-dogfood/20260702-181216-50924/` showed why this split matters:
the old scoped map path tried full-volume construction and exceeded the 20
second internal budget while reading sequential MFT bytes. A follow-up run with
`REBECCA_NTFS_MFT_INDEX_TIMEOUT_SECONDS=0` reached the script-level 180 second
timeout under `target/ntfs-dogfood/20260702-181342-73156/`; full-index disk-map
performance remains a drive-root/diagnostic optimization target rather than a
release threshold for scoped maps.
When full-index diagnostics do return after parsed records are available, the
`mft-index-allocation-budget-exhausted` caveat means Rebecca stopped further
stream-backed `$I30` reads after crossing the live budget and preserved degraded
MFT evidence instead of discarding the entire backend source. On the current
2026-07-04 elevated E: workstation, forced full-index dogfood runs against
`docs\plans` still hit the dogfood script's 180 second process timeout while E:
was under cleanup load, so that host result remains a performance finding rather
than a release threshold.

When collecting drive-root or explicit full-index evidence after `$MFTMirr`
integration, inspect `backend_source_kind`, `ntfs_full_index_source`,
`ntfs_mirror_record_used_count`, `ntfs_mirror_read_failed_count`,
`ntfs_mirror_evidence`, `ntfs_mft_stage_timings`, and
`ntfs_mft_build_metrics` in `inspect-map-report.json` or
`inspect-map-runs.csv`. `mft-mirror-record-used` proves bounded mirror recovery
changed parser output for a reported record; `mft-mirror-read-failed` proves
mirror bytes were unavailable while primary `$MFT` parsing remained
authoritative. Use the timing and metric fields to decide whether a future
persistent volume index cache should target raw full-volume reads, targeted
record resolution, or stream-backed `$I30` expansion first. The live backend now
has an internal typed volume identity, stable volume fingerprint generation, and
an opt-in manifest plus versioned `MftIndex` payload store for configured
builds; the missing production piece is still live USN checkpoint capture and
validation before cross-process reuse.

After the adaptive disk-map refactor, elevated local dogfood under
`target/ntfs-dogfood/20260702-185357-58228/` completed
`inspect-map-windows-ntfs-mft-experimental-E-Rust-rebecca-docs-plans` through
`windows-ntfs-mft-experimental-targeted-fsctl` with no fallback, 624422 logical
bytes, 39 files, and a 910 ms script-measured duration.

The focused NTFS fixture dogfood on 2026-07-04 under
`target/ntfs-dogfood-reports/20260704-044429-77128/` completed without
`-AllowMismatch`: portable, Windows native, and experimental NTFS/MFT matched
logical bytes, file counts, and directory counts for hardlinks, sparse files,
compressed files, large directories, nested files, and fragmentation candidates.
The experimental run used `windows-ntfs-mft-experimental-targeted-fsctl`,
reported three caveats, and completed in 2381 ms. Sparse and compressed
allocated bytes matched Windows native after data-run-backed allocation
accounting; the `mft-data-run-allocated-by-cluster` caveat names cases where
NTFS/MFT data runs count physical clusters and may intentionally differ from
Windows native file allocation APIs.

The portable map implementation should keep memory proportional to traversal
depth plus the requested `--top` bound. It uses post-order aggregation and
pushes completed entries directly into a bounded heap instead of retaining a
full directory-node tree before ranking.
Child-level metadata, directory-read, directory-entry, and reparse failures are
reported as diagnostics with zero bytes for skipped subtrees, so active cache
trees can still produce useful conservative maps instead of failing the entire
command.
Raw diagnostic samples are bounded by `--diagnostic-limit`; the
`diagnostic_summary` counts remain complete so noisy trees cannot turn
diagnostic output into an unbounded memory or JSON payload.

To include an explicit live NTFS source benchmark on a representative Windows machine, opt in for that run:

```powershell
$env:REBECCA_PERF_MATRIX_LIVE_NTFS = "1"
pwsh -File scripts/perf/run-benchmark-matrix.ps1
Remove-Item Env:\REBECCA_PERF_MATRIX_LIVE_NTFS
```

When the live scenario succeeds through the experimental backend, its normal source should be `windows-ntfs-mft-experimental-targeted-fsctl`. Explicit full-index fallback or diagnostic runs may report `windows-ntfs-mft-experimental-sequential` or `windows-ntfs-mft-experimental-fsctl-record`. When the host is unsupported or unelevated, the same scenario must report a directory-scanner fallback with no backend source. Parser caveat volume is part of performance evidence: a faster live run that silently drops attribute-list, sequence, hardlink, runlist, or directory-index uncertainty is not a valid improvement.

For parser-core work, run the NTFS microbench self-check before trusting Criterion numbers:

```powershell
cargo bench -p rebecca-ntfs --bench mft_parser -- --test
```

The NTFS parser bench includes generated MFT records, subtree indexing, stream-backed `$INDEX_ALLOCATION:$I30` expansion, and fragmented runlist reads. It is fixture-backed and does not require elevated live volume access.

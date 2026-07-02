# Rebecca Performance Matrix

The performance matrix is the product-level baseline for scan, cache, and cleanup execution work. It is intentionally synthetic and deterministic so later refactors can compare the same shapes before using real-machine dogfood.

Run the compile check first:

```powershell
cargo check -p rebecca-core --benches
cargo check -p rebecca-ntfs --benches
```

Run the matrix and collect a JSON report:

```powershell
pwsh -File scripts/perf/run-benchmark-matrix.ps1
```

The script runs `cargo bench -p rebecca-core --bench perf_matrix`, reads Criterion estimates from `target/criterion/perf_matrix`, combines them with scenario metadata from `target/perf/perf_matrix-scenarios.json`, and writes `target/perf/rebecca-core-perf_matrix-report.json`.

The report records scenario name, operation, requested backend, backend-source expectation, fixture shape, physical files and directories, expected bytes, progress-event count, target count, cache mode, delete mode, and Criterion mean/median timing estimates. The default scenarios cover:

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
- scan-cache miss plus store
- scan-cache hit
- serial cleanup deletion
- parallel cleanup deletion
- batch-backend cleanup deletion

Keep reports under `target/perf/`; they are local measurement artifacts and should not be committed unless a future release process explicitly asks for a checked-in baseline.

The default matrix does not read a live NTFS volume because that requires host privileges and can make Criterion results depend on the whole workstation disk.
Use the NTFS dogfood script for live `windows-ntfs-mft-experimental` evidence, then compare the JSON `estimate_backend`, `estimate_backend_source`, `estimate_fallback_reason`, and `estimate_caveats` fields against the portable and Windows native scenarios. NTFS parser-core performance work should keep the first-party parser path distinguishable from any future external adapter or oracle path in report labels before adding new speed thresholds.

```powershell
pwsh -File scripts/ntfs/run-live-mft-dogfood.ps1 -Root docs/plans -Mode inspect-space -Top 3 -TimeoutSeconds 45
pwsh -File scripts/ntfs/run-live-mft-dogfood.ps1 -Root docs/plans -Mode inspect-map -Top 3 -TimeoutSeconds 60
```

The dogfood report is written under `target/ntfs-dogfood/` and includes raw CLI output, requested versus actual backend, portable baseline deltas, and timeout status. A timeout from the experimental backend is a valid local finding because live metadata traversal can depend on target size, privilege, and disk health; keep it out of Criterion thresholds until the backend has deterministic fixture coverage for the suspected bottleneck.
The experimental backend has its own 20 second live metadata budget before
falling back to a directory scanner; set `REBECCA_NTFS_MFT_INDEX_TIMEOUT_SECONDS`
higher for deep profiling or `0` to disable that guard for one process.
Set `REBECCA_NTFS_MFT_INDEX_TIMINGS=1` for live dogfood when you need stage
timings in timeout fallback reasons or an opt-in `mft-index-build-timing`
caveat on successful experimental runs.
Set `REBECCA_NTFS_MFT_FULL_INDEX_FALLBACK=1` only when you intentionally want a
targeted-traversal failure to try the older full-volume MFT index path before
directory-scanner fallback.

`inspect map` defaults to portable recursive inventory. Selecting
`--scan-backend windows-ntfs-mft-experimental` is adaptive: scoped roots should
normally report `windows-ntfs-mft-experimental-targeted-fsctl`, while drive-root
maps or explicit full-index diagnostics may report sequential or FSCTL-record
full-index sources. On the earlier 2026-07-02 elevated E: run,
`target/ntfs-dogfood/20260702-181216-50924/` showed why this split matters:
the old scoped map path tried full-volume construction and exceeded the 20
second internal budget while reading sequential MFT bytes. A follow-up run with
`REBECCA_NTFS_MFT_INDEX_TIMEOUT_SECONDS=0` reached the script-level 180 second
timeout under `target/ntfs-dogfood/20260702-181342-73156/`; full-index disk-map
performance remains a drive-root/diagnostic optimization target rather than a
release threshold for scoped maps.

After the adaptive disk-map refactor, elevated local dogfood under
`target/ntfs-dogfood/20260702-185357-58228/` completed
`inspect-map-windows-ntfs-mft-experimental-E-Rust-rebecca-docs-plans` through
`windows-ntfs-mft-experimental-targeted-fsctl` with no fallback, 624422 logical
bytes, 39 files, and a 910 ms script-measured duration.

The portable map implementation should keep memory proportional to traversal
depth plus the requested `--top` bound. It uses post-order aggregation and
pushes completed entries directly into a bounded heap instead of retaining a
full directory-node tree before ranking.
Child-level metadata, directory-read, directory-entry, and reparse failures are
reported as diagnostics with zero bytes for skipped subtrees, so active cache
trees can still produce useful conservative maps instead of failing the entire
command.

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

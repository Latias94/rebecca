# Release Integrity

Rebecca uses one tag-driven release workflow for crates.io publishing and cargo-dist GitHub Releases. The current downloadable target is Windows x86_64 MSVC.

Release handling is split across four workflows:

- `ci.yml` runs formatting, linting, tests, Linux and macOS no-root cleanup smokes, cargo-dist planning, and a Windows release-packaging smoke test on pushes and pull requests;
- `release-gates.yml` is a manual evidence gate that runs the shared release gate wrapper, uploads dogfood/performance artifacts, and can compare full benchmark output against a prior workflow artifact;
- `release-preflight.yml` is a manual gate that validates a chosen source ref and version, checks crate package file lists, dry-runs the first registry-independent crate publish, and exercises the repository PowerShell release archive scripts;
- `release.yml` publishes `rebecca-core`, `rebecca-rules`, `rebecca-windows`, and `rebecca` to crates.io in dependency order, then publishes the tag-driven ZIP, PowerShell installer, and checksum files to GitHub Releases.

## Artifact Names

For tag `v0.2.0`, cargo-dist currently publishes:

```text
rebecca-x86_64-pc-windows-msvc.zip
rebecca-x86_64-pc-windows-msvc.zip.sha256
rebecca-installer.ps1
sha256.sum
source.tar.gz
source.tar.gz.sha256
```

The tag prefix may be `v` or `V`; the cargo-dist release version omits that prefix in generated metadata.

## Install Or Update

Use the cargo-dist PowerShell installer:

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/Latias94/rebecca/releases/download/v0.2.0/rebecca-installer.ps1 | iex"
```

Set `REBECCA_INSTALL_DIR` to override the install directory. Run the installer for a newer tag to update.

Install from crates.io when a Rust toolchain is already available:

```powershell
cargo install rebecca --locked
```

The release workflow dry-runs unpublished crates before publishing, skips crate versions already visible on crates.io, and waits for each dependency crate to become visible before publishing the next dependent crate. GitHub Release hosting waits for crates.io publishing to complete successfully, so a tag has one release status instead of two independent tag-triggered publishers.

## Verify Checksums

When downloading assets manually, verify the ZIP checksum against either the per-asset `.sha256` file or the unified `sha256.sum` file from the same GitHub Release:

```powershell
$asset = "rebecca-x86_64-pc-windows-msvc.zip"
$expected = (Get-Content ".\$asset.sha256").Split(" ", [System.StringSplitOptions]::RemoveEmptyEntries)[0].ToLowerInvariant()
$actual = (Get-FileHash -LiteralPath ".\$asset" -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actual -ne $expected) {
    throw "Checksum mismatch for $asset"
}
```

Checksum verification proves that the downloaded ZIP matches the checksum file published in the release. It does not prove who built either file.

## Local Release Smoke Test

Maintainers can run the repository's PowerShell package and checksum scripts locally. These scripts are also exercised by `ci.yml` and `release-preflight.yml` as an install/archive smoke test; they are not the tag-driven GitHub Release publisher.

```powershell
.\scripts\release\build-release.ps1 -Tag v0.2.0 -OutDir target\release-smoke
.\scripts\release\write-sbom.ps1 -Tag v0.2.0 -DistDir target\release-smoke
.\scripts\release\write-checksums.ps1 -DistDir target\release-smoke
Get-Content target\release-smoke\SHA256SUMS
```

Local smoke artifacts are not official releases.

## Performance And Dogfood Preflight

Before a release-facing merge, run the release gate wrapper. It records command
stdout/stderr, machine JSON payloads, perf artifacts, dogfood artifacts, and a
single `release-gates-report.json` under `target\release-gates\<run-id>\`.

Fast preflight for ordinary branch landing:

```powershell
pwsh -File scripts\release\run-release-gates.ps1
```

Full release evidence with Criterion benchmarks, a local baseline, and the
experimental NTFS/MFT dogfood backend:

```powershell
pwsh -File scripts\release\run-release-gates.ps1 -Benchmark full -BenchmarkBaselinePath target\perf\baseline.json -Dogfood all
```

Use the script's `-SelfTest` mode for a cheap check of the gate harness itself:

```powershell
pwsh -File scripts\release\run-release-gates.ps1 -SelfTest
```

The wrapper runs formatting, clippy, workspace tests, dependency policy,
`catalog validate`, `cache inspect`, a dry-run cleanup preview, the benchmark
comparator self-test, the benchmark matrix smoke/full path, and inspect-map
dogfood according to its parameters. It is the preferred local release-facing
gate; use the lower-level commands below when diagnosing a single surface or
collecting additional evidence.

The same gate can be run from GitHub Actions with the manual `Release Gates`
workflow. Its default `benchmark=smoke` and `dogfood=stable` inputs are the
branch-landing profile. For release-candidate evidence, run it with
`benchmark=full` and `dogfood=all` on a representative Windows runner. Every
run uploads a `release-gates` artifact containing the gate report, raw command
stdout/stderr, benchmark reports, and dogfood reports.

Use a successful full benchmark run on `main` as the blessed baseline for the
next release-candidate comparison. Pass that workflow run id as
`baseline_artifact_run_id`; the workflow downloads its `release-gates` artifact,
locates `rebecca-core-perf_matrix-report.json`, and passes it to
`-BenchmarkBaselinePath` when the current run uses `benchmark=full`. If no
baseline run id is provided, the workflow still records full benchmark evidence
but does not classify performance deltas against a historical run.

Run the performance matrix directly when you need raw benchmark reports. A
matrix run without a baseline is report-only; a run with `-BaselinePath` is a
gate and fails on threshold regressions.

```powershell
pwsh -File scripts\perf\run-benchmark-matrix.ps1
```

When a prior report is available, compare the current report against it:

```powershell
pwsh -File scripts\perf\run-benchmark-matrix.ps1 -BaselinePath target\perf\baseline.json
```

The expected report paths are
`target\perf\rebecca-core-perf_matrix-report.json`,
`target\perf\rebecca-core-perf_matrix-report-scenarios.csv`, and
`target\perf\rebecca-core-perf_matrix-report-summary.md`. Baseline runs also
write comparison JSON, CSV, and Markdown artifacts and classify scenarios as
pass, regression, improvement, skipped, missing-baseline, or missing-current
with a default 10% threshold. Use `-SkipRun` for report-generation smoke checks
that should not execute Criterion; missing benchmark artifacts should produce a
`skipped` or `partial` report instead of a script failure.

Collect live NTFS/MFT evidence with the inspect-map dogfood script. It isolates
Rebecca config, state, cache, and history under
`target\inspect-map-dogfood\<run-id>\` and writes JSON, CSV, Markdown, and raw
command output for each backend run. Use the run-level comparison fields first:
they record portable-baseline match status, requested and actual backend
identity, and byte/file/directory deltas for each backend/repetition.

```powershell
pwsh -File scripts\dogfood\run-inspect-map-report.ps1 -Root docs\plans -Backend portable-recursive,windows-native,windows-ntfs-mft-experimental -Repeat 1 -Top 20 -GroupBy extension,depth,age -DiagnosticLimit 0
```

For a focused NTFS physical-semantics fixture, run:

```powershell
pwsh -File scripts\dogfood\run-ntfs-fixture-dogfood.ps1 -Repeat 1 -LargeFileCount 128 -Top 20 -DiagnosticLimit 0
```

Review the fixture `ntfs-fixture-manifest.json` before treating the report as
evidence. Hardlink, sparse, and compression support can be unavailable on some
hosts or filesystems; those gaps should appear as manifest caveats.

For persistent NTFS/MFT payload and USN replay evidence, run:

```powershell
pwsh -File scripts\dogfood\run-ntfs-usn-replay-dogfood.ps1 -Top 20 -DiagnosticLimit 0 -IndexTimeoutSeconds 60
```

The USN replay script isolates `REBECCA_CACHE_DIR`, explicitly enables
`REBECCA_NTFS_MFT_VOLUME_INDEX_CACHE=1`, mutates unrelated and target fixture
subtrees between phases, and should show
`ntfs-full-index-persistent-cache` for unrelated replay and post-rebuild hits.
Without that env var, ordinary `inspect map` runs must not create persistent
MFT payloads. If a phase rebuilds unexpectedly, inspect
`mft-persistent-cache-miss` and `mft-persistent-cache-write-skipped` caveat code
counts in the dogfood summary before opening raw stdout.

When a stable `persistent-cache` hit is required, prefer the isolated VHD
wrapper over a large active workstation volume:

```powershell
pwsh -File scripts\dogfood\run-ntfs-usn-replay-vhd-dogfood.ps1 -VhdSizeMB 256 -TimeoutSeconds 180 -IndexTimeoutSeconds 30
```

The wrapper creates and formats a new dynamic VHDX, runs the USN replay dogfood
on that scratch NTFS volume with a small USN journal, detaches it by default,
and writes the inner dogfood report under `dogfood\` beside DiskPart and
`fsutil` logs.

Use a smaller `-Root` such as `docs\plans` when the repository root includes large `target\` or `repo-ref\` trees. The script refuses output directories inside the scanned root unless `-AllowOutputInsideRoot` is passed. Backend mismatches, missing portable baselines, parse failures, and timeouts exit non-zero by default; pass `-AllowMismatch` only when collecting exploratory evidence from a changing tree. The backend has an internal 20 second live metadata budget by default; set `REBECCA_NTFS_MFT_INDEX_TIMEOUT_SECONDS` higher for deep diagnosis, or `0` to disable the guard for a single dogfood process. Set `REBECCA_NTFS_MFT_INDEX_TIMINGS=1` to capture active-stage timeout context and successful `mft-index-build-timing` caveats during release dogfood. Set `REBECCA_NTFS_MFT_FULL_INDEX_FALLBACK=1` only when you intentionally want to compare targeted traversal against the older full-volume MFT index path.

Run this dogfood checklist on a representative Windows workstation:

```powershell
cargo run -p rebecca -- catalog validate
cargo run -p rebecca -- inspect space --root . --top 10 --format json
cargo run -p rebecca -- inspect map --root . --top 10 --max-depth 3 --format json
cargo run -p rebecca -- inspect artifacts --root . --format json
cargo run -p rebecca -- inspect lint --root . --top 10 --format json
cargo run -p rebecca -- clean --dry-run --scan-cache --category system
cargo run -p rebecca -- clean --dry-run --scan-cache --category system
cargo run -p rebecca -- clean --dry-run --no-scan-cache --scan-backend windows-native --category system --format json
cargo run -p rebecca -- clean --dry-run --no-scan-cache --scan-backend windows-ntfs-mft-experimental --category system --format json
cargo run -p rebecca -- inspect space --scan-backend windows-ntfs-mft-experimental --root . --top 10 --format json
cargo run -p rebecca -- inspect map --scan-backend windows-native --root docs\plans --top 10 --format json
cargo run -p rebecca -- inspect map --scan-backend windows-ntfs-mft-experimental --root docs\plans --top 10 --format json
```

Prefer the script for repeatable backend comparison; use the raw commands above when diagnosing a single CLI behavior, dry-run safety, or reproducing a script-captured failure.

For delete smoke, use a dry-run against disposable user-temp data and verify the
default plan remains recoverable before any real `--yes` run:

```powershell
$root = Join-Path $env:TEMP "rebecca-delete-smoke"
New-Item -ItemType Directory -Force -Path $root | Out-Null
Set-Content -LiteralPath (Join-Path $root "delete-me.tmp") -Value "smoke"
cargo run -p rebecca -- clean --dry-run --rule windows.user-temp
```

Record JSON `estimate_source`, `estimate_backend`, `estimate_backend_source`,
`estimate_confidence`, `estimate_fallback_reason`, and `estimate_caveats` values
for the backend dogfood runs. Also record any `diagnostic_summary` totals and
the dogfood report's `comparisons.status`, allocated/unique comparison status,
throughput, repeat-stat fields, `backend_source_kind`, `caveat_code_counts`,
and NTFS mirror evidence fields. The Windows native map run
should show `estimate_backend: "windows-native"` and non-null `allocated_bytes`
on supported local roots. When hardlinks or other repeated file ids are present,
the same run should keep path-ranked bytes unchanged while reporting non-null
`unique_logical_bytes` and `unique_allocated_bytes` if file identity metadata is
available. Windows native runs may also report `compressed-file`, `sparse-file`,
`hardlink-file`, or `reparse-skipped` caveats when those filesystem semantics
are present. A grouped map run with `--group-by extension --group-by depth --group-by age --sort files --group-sort files`
should emit bounded `groups` with logical bytes and file counts from the same
traversal, and should reorder top entries/groups by file count. The experimental NTFS/MFT run should either show
`estimate_backend: "windows-ntfs-mft-experimental"` with
`estimate_backend_source: "windows-ntfs-mft-experimental-targeted-fsctl"` on a
supported elevated local NTFS volume, including scoped `inspect map` roots.
Explicit full-index fallback diagnostics and drive-root disk maps may instead
show `"windows-ntfs-mft-experimental-sequential"` or
`"windows-ntfs-mft-experimental-fsctl-record"`; those runs may also report a
budget or command timeout on large live volumes, which should be captured as
backend performance evidence rather than treated as a dogfood script failure.
Explicit persistent-cache diagnostics may instead show
`"windows-ntfs-mft-experimental-persistent-cache"` after the cache has been
warmed and USN replay proves the target subtree is unchanged.
Unsupported hosts should report a clear fallback reason, no backend source, and
`experimental-ntfs-mft-fallback` caveat. Successful NTFS/MFT dogfood should also
review any parser caveats for sequence mismatches, hardlink path candidates,
attribute-list handling, directory-index fallback, unsupported nonresident
streams, `mft-index-allocation-budget-exhausted` full-index budget degradation,
`mft-data-run-allocated-by-cluster` allocation-semantics notes, or bounded
parse-error summaries. Sequential full-index diagnostics may also report
`mft-mirror-record-used` or `mft-mirror-read-failed`; mirror caveats are
summarized by `ntfs_mirror_record_used_count`,
`ntfs_mirror_read_failed_count`, and `ntfs_mirror_evidence` in the dogfood
report. Mirror caveats are read-only recovery evidence and must not be
interpreted as cleanup authority.
The data-run allocation caveat is expected when NTFS/MFT reports
cluster-allocation evidence that intentionally differs from a Windows native
file allocation API value. Focused Windows backend tests and the performance
matrix remain the authoritative evidence for native and experimental backend
fallback behavior.

## Current Limitations

- The first supported downloadable target is Windows x86_64 MSVC.
- GitHub artifact attestations are not currently emitted by the cargo-dist release workflow.
- Winget, Scoop, MSI, MSIX, and in-CLI update commands are not implemented.

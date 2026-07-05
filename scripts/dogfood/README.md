# Dogfood Scripts

## Inspect Map Report

`run-inspect-map-report.ps1` runs `rebecca inspect map --format json` once per
requested backend and repetition, then derives JSON, CSV, and Markdown report
files from those parsed payloads. It does not run a second table-mode scan for
formatting.

Example:

```powershell
pwsh -File scripts/dogfood/run-inspect-map-report.ps1 `
  -Root docs `
  -Backend portable-recursive,windows-native,windows-ntfs-mft-experimental `
  -Repeat 1 `
  -Top 20 `
  -GroupBy extension,depth,age `
  -DiagnosticLimit 0
```

Outputs are written under `target/inspect-map-dogfood/<timestamp-pid>/` by
default:

- `inspect-map-report.json`
- `inspect-map-runs.csv`
- `inspect-map-rows.csv`
- `inspect-map-summary.md`
- `raw/*.stdout.json`
- `raw/*.stderr.txt`

Run summaries include duration, derived throughput, requested backend, actual
backend/source, normalized `backend_source_kind`, fallback reason, caveat count,
`caveat_code_counts`, NTFS full-index and mirror evidence fields, diagnostic
counts, logical/allocated/unique totals, portable-baseline deltas, allocated and
unique comparison status, and repeat duration statistics. Unknown allocated or
unique metrics are recorded as unknown rather than zero.

Drive roots are refused unless `-AllowDriveRoot` is passed. The script isolates
Rebecca config, state, cache, and history paths under the report directory.
Report directories inside the scanned root are refused unless
`-AllowOutputInsideRoot` is passed, because generated raw output can pollute
later backend comparisons. Backend mismatches or missing portable baselines make
the script exit non-zero unless `-AllowMismatch` is passed; run failures and
timeouts remain failures.

Sequential full-index NTFS/MFT runs may report
`ntfs_mirror_record_used_count`, `ntfs_mirror_read_failed_count`, and
`ntfs_mirror_evidence` values derived from `mft-mirror-record-used` and
`mft-mirror-read-failed` caveats. These fields are release evidence only; they
do not authorize cleanup deletion.

Run the pure parser/report tests without invoking Cargo:

```powershell
pwsh -File scripts/dogfood/run-inspect-map-report.ps1 -SelfTest
```

## NTFS Fixture Dogfood

`run-ntfs-fixture-dogfood.ps1` creates a local fixture tree under
`target/ntfs-dogfood-fixtures/<timestamp-pid>/`, writes
`ntfs-fixture-manifest.json`, and then calls `run-inspect-map-report.ps1` against
that fixture. The fixture includes hardlink aliases, a sparse file, a compressed
file, a large flat directory, a nested directory, and best-effort fragmentation
candidates. Unsupported host features are recorded as manifest caveats rather
than silently treated as evidence.

```powershell
pwsh -File scripts/dogfood/run-ntfs-fixture-dogfood.ps1 `
  -Repeat 1 `
  -LargeFileCount 128 `
  -Top 20 `
  -DiagnosticLimit 0
```

Outputs are local artifacts:

- fixture: `target/ntfs-dogfood-fixtures/<timestamp-pid>/`
- report: `target/ntfs-dogfood-reports/<timestamp-pid>/`

Run the fixture builder self-test without invoking the CLI report:

```powershell
pwsh -File scripts/dogfood/run-ntfs-fixture-dogfood.ps1 -SelfTest
```

## NTFS USN Replay Dogfood

`run-ntfs-usn-replay-dogfood.ps1` creates a target subtree and an unrelated
same-volume subtree, then runs the real `inspect map` CLI path through four
cache phases:

- `warm-build`: build a persistent full-index payload from an isolated cache.
- `unrelated-replay`: mutate the unrelated subtree and expect a
  `persistent-cache` hit with unchanged target bytes.
- `target-invalidates`: mutate the target subtree and expect a rebuild plus
  increased target bytes.
- `post-rebuild-hit`: repeat without more mutations and expect another
  `persistent-cache` hit.

The script sets `REBECCA_NTFS_MFT_VOLUME_INDEX_CACHE=1`,
`REBECCA_NTFS_MFT_FULL_INDEX_FALLBACK=1`,
`REBECCA_NTFS_MFT_INDEX_TIMINGS=1`, and an isolated `REBECCA_CACHE_DIR` for the
dogfood process. Default CLI scans do not enable this persistent MFT payload
store. Persistent writes require a stable USN checkpoint before and after the
full-index build, so large or busy volumes can produce useful rebuild/timeout
evidence without producing a `persistent-cache` hit.

```powershell
pwsh -File scripts/dogfood/run-ntfs-usn-replay-dogfood.ps1 `
  -Top 20 `
  -DiagnosticLimit 0 `
  -IndexTimeoutSeconds 60
```

Outputs are local artifacts:

- fixture: `target/ntfs-usn-replay-fixtures/<timestamp-pid>/`
- report: `target/ntfs-usn-replay-dogfood/<timestamp-pid>/`
- `ntfs-usn-replay-report.json`
- `ntfs-usn-replay-summary.md`
- `raw/*.stdout.json`
- `raw/*.stderr.txt`

Run the parser and expectation self-test without invoking Cargo:

```powershell
pwsh -File scripts/dogfood/run-ntfs-usn-replay-dogfood.ps1 -SelfTest
```

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

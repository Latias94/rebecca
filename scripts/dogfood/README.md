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

Drive roots are refused unless `-AllowDriveRoot` is passed. The script isolates
Rebecca config, state, cache, and history paths under the report directory.
Report directories inside the scanned root are refused unless
`-AllowOutputInsideRoot` is passed, because generated raw output can pollute
later backend comparisons. Backend mismatches or missing portable baselines make
the script exit non-zero unless `-AllowMismatch` is passed; run failures and
timeouts remain failures.

Run the pure parser/report tests without invoking Cargo:

```powershell
pwsh -File scripts/dogfood/run-inspect-map-report.ps1 -SelfTest
```

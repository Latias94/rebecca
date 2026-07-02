# Release Integrity

Rebecca uses one tag-driven release workflow for crates.io publishing and cargo-dist GitHub Releases. The current downloadable target is Windows x86_64 MSVC.

Release handling is split across three workflows:

- `ci.yml` runs formatting, linting, tests, cargo-dist planning, and a Windows release-packaging smoke test on pushes and pull requests;
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

Before a release-facing merge, run the performance matrix or record why it was
skipped. The matrix is report-only until Rebecca has enough stable baseline
history for hard thresholds.

```powershell
pwsh -File scripts\perf\run-benchmark-matrix.ps1
```

The expected report path is
`target\perf\rebecca-core-perf_matrix-report.json`.

Run this dogfood checklist on a representative Windows workstation:

```powershell
cargo run -p rebecca -- catalog validate
cargo run -p rebecca -- inspect space --root . --top 10 --format json
cargo run -p rebecca -- inspect artifacts --root . --format json
cargo run -p rebecca -- inspect lint --root . --top 10 --format json
cargo run -p rebecca -- clean --dry-run --scan-cache --category system
cargo run -p rebecca -- clean --dry-run --scan-cache --category system
cargo run -p rebecca -- clean --dry-run --no-scan-cache --scan-backend windows-native --category system --format json
cargo run -p rebecca -- clean --dry-run --no-scan-cache --scan-backend windows-ntfs-mft-experimental --category system --format json
cargo run -p rebecca -- inspect space --scan-backend windows-ntfs-mft-experimental --root . --top 10 --format json
```

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
for the backend dogfood runs. The experimental NTFS/MFT run should either show
`estimate_backend: "windows-ntfs-mft-experimental"` with
`estimate_backend_source: "windows-ntfs-mft-experimental-sequential"` or
`"windows-ntfs-mft-experimental-fsctl-record"` on a supported elevated local
NTFS volume, or a clear fallback reason, no backend source, and
`experimental-ntfs-mft-fallback` caveat. Successful NTFS/MFT dogfood should also
review any parser caveats for sequence mismatches, hardlink path candidates,
attribute-list handling, directory-index fallback, unsupported nonresident index
allocation, or bounded parse-error summaries. Focused Windows backend tests and
the performance matrix remain the authoritative evidence for native and
experimental backend fallback behavior.

## Current Limitations

- The first supported downloadable target is Windows x86_64 MSVC.
- GitHub artifact attestations are not currently emitted by the cargo-dist release workflow.
- Winget, Scoop, MSI, MSIX, and in-CLI update commands are not implemented.

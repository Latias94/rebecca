# Release Integrity

Rebecca tracks cargo-dist-style release metadata in `dist-workspace.toml` and
uses GitHub Actions to keep the release surface explicit. The current
distribution target is a Windows x86_64 MSVC ZIP archive, an SPDX SBOM, and a
`SHA256SUMS` file.

Release handling is split into three layers:

- `ci.yml` runs formatting, linting, tests, and a Windows release-packaging
  smoke test on pushes and pull requests;
- `release-preflight.yml` is a manual gate that validates a chosen source ref
  and version, then exercises the release archive and installer flow;
- `release.yml` publishes the tag-driven ZIP, SBOM, checksums, and GitHub
  build-provenance attestations.

## Artifact Names

For tag `v0.1.0`, the current artifact names are:

```text
rebecca-0.1.0-windows-x86_64-msvc.zip
rebecca-0.1.0-windows-x86_64-msvc.spdx
```

The tag prefix may be `v` or `V`; the artifact version omits that prefix.

## Verify The Checksum

Download the ZIP, SBOM, and `SHA256SUMS` from the same GitHub Release, then
verify the asset hashes in PowerShell:

```powershell
foreach ($asset in @(
    "rebecca-0.1.0-windows-x86_64-msvc.zip",
    "rebecca-0.1.0-windows-x86_64-msvc.spdx"
)) {
    $expected = (Select-String -LiteralPath .\SHA256SUMS -Pattern "  $asset$").Line.Split(" ", [System.StringSplitOptions]::RemoveEmptyEntries)[0]
    $actual = (Get-FileHash -LiteralPath ".\$asset" -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $expected) {
        throw "Checksum mismatch for $asset"
    }
}
```

Checksum verification proves that the downloaded ZIP matches the checksum file
published in the release. It does not prove who built either file.

## Verify Build Provenance

When the GitHub CLI is installed and authenticated, verify the artifact
attestation:

```powershell
gh attestation verify .\rebecca-0.1.0-windows-x86_64-msvc.zip --repo OWNER/REPO --deny-self-hosted-runners
gh attestation verify .\rebecca-0.1.0-windows-x86_64-msvc.spdx --repo OWNER/REPO --deny-self-hosted-runners
gh attestation verify .\SHA256SUMS --repo OWNER/REPO --deny-self-hosted-runners
```

Replace `OWNER/REPO` with the GitHub repository that published the release.

The `--deny-self-hosted-runners` flag rejects attestations produced by
self-hosted runners. Rebecca's release workflow is expected to use GitHub-hosted
runners for the published artifacts.

## Local Release Smoke Test

Maintainers can run the same package and checksum scripts locally:

```powershell
.\scripts\release\build-release.ps1 -Tag v0.1.0 -OutDir target\release-smoke
.\scripts\release\write-sbom.ps1 -Tag v0.1.0 -DistDir target\release-smoke
.\scripts\release\write-checksums.ps1 -DistDir target\release-smoke
Get-Content target\release-smoke\SHA256SUMS
```

Local smoke artifacts are not official releases and do not have GitHub build
provenance.

## Install Or Update

Use the PowerShell installer to download a release, verify `SHA256SUMS`, and
install `rebecca.exe` under `%LOCALAPPDATA%\Programs\Rebecca` by default:

```powershell
.\scripts\install.ps1 -Repository OWNER/REPO
.\scripts\install.ps1 -Repository OWNER/REPO -Tag v0.1.0
```

Run the same command with a newer tag to update. The install directory can be
overridden:

```powershell
.\scripts\install.ps1 -Repository OWNER/REPO -Tag v0.1.0 -InstallDir D:\Tools\Rebecca
```

To fail closed unless GitHub build provenance verifies, require attestation:

```powershell
.\scripts\install.ps1 -Repository OWNER/REPO -Tag v0.1.0 -RequireAttestation
```

`-RequireAttestation` requires an installed and authenticated GitHub CLI. Without
that flag, the installer still verifies the checksum and reports whether
attestation was skipped, verified, or failed as an optional check.

The release ZIP also includes `scripts\install.ps1` and `VERSION.txt`. When
running from an extracted release package, the installer can read the repository
from `VERSION.txt`; otherwise pass `-Repository OWNER/REPO` or set
`REBECCA_REPOSITORY`.

The installer does not edit PATH automatically. Add the install directory to
your user PATH if you want to run `rebecca` from any terminal.

## Current Limitations

- The first supported downloadable target is Windows x86_64 MSVC.
- Package-manager publishing is not implemented.
- MSI/MSIX and in-CLI update commands are not implemented.
- The checked-in release pipeline still uses the repository's PowerShell
  packaging scripts instead of invoking cargo-dist directly.

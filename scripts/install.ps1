param(
    [string]$Repository = $env:REBECCA_REPOSITORY,
    [string]$Tag = $env:REBECCA_VERSION,
    [string]$InstallDir = "",
    [string]$AssetPath = "",
    [string]$ChecksumsPath = "",
    [switch]$RequireAttestation,
    [switch]$SkipAttestation
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$UserAgent = "Rebecca installer"
$Target = "windows-x86_64-msvc"

function Get-ScriptRoot {
    return (Split-Path -Parent $PSCommandPath)
}

function Get-RepoRoot {
    $scriptRoot = Get-ScriptRoot
    return (Resolve-Path -LiteralPath (Join-Path $scriptRoot "..")).ProviderPath
}

function Resolve-InstallDir {
    param([string]$RequestedInstallDir)

    if (-not [string]::IsNullOrWhiteSpace($RequestedInstallDir)) {
        return [System.IO.Path]::GetFullPath($RequestedInstallDir)
    }

    if ([string]::IsNullOrWhiteSpace($env:LOCALAPPDATA)) {
        throw "LOCALAPPDATA is not set; pass -InstallDir explicitly."
    }

    return [System.IO.Path]::GetFullPath((Join-Path $env:LOCALAPPDATA "Programs\Rebecca"))
}

function Convert-GitRemoteToRepository {
    param([string]$RemoteUrl)

    if ([string]::IsNullOrWhiteSpace($RemoteUrl)) {
        return ""
    }

    $trimmed = $RemoteUrl.Trim()
    $patterns = @(
        '^https://github\.com/([^/]+/[^/]+?)(?:\.git)?$',
        '^git@github\.com:([^/]+/[^/]+?)(?:\.git)?$',
        '^ssh://git@github\.com/([^/]+/[^/]+?)(?:\.git)?$'
    )

    foreach ($pattern in $patterns) {
        if ($trimmed -match $pattern) {
            return $Matches[1]
        }
    }

    return ""
}

function Resolve-Repository {
    param([string]$RequestedRepository)

    if (-not [string]::IsNullOrWhiteSpace($RequestedRepository)) {
        return $RequestedRepository.Trim()
    }

    $repoRoot = Get-RepoRoot
    $versionFile = Join-Path $repoRoot "VERSION.txt"
    if (Test-Path -LiteralPath $versionFile -PathType Leaf) {
        foreach ($line in Get-Content -LiteralPath $versionFile) {
            if ($line -match '^repository=(.+)$') {
                $fromVersionFile = $Matches[1].Trim()
                if (-not [string]::IsNullOrWhiteSpace($fromVersionFile)) {
                    return $fromVersionFile
                }
            }
        }
    }

    $remote = ""
    try {
        $remote = (& git -C $repoRoot config --get remote.origin.url 2>$null)
    }
    catch {
        $remote = ""
    }

    return Convert-GitRemoteToRepository -RemoteUrl $remote
}

function Assert-Repository {
    param([string]$ResolvedRepository)

    if ($ResolvedRepository -notmatch '^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$') {
        throw "Could not resolve GitHub repository. Pass -Repository owner/repo or set REBECCA_REPOSITORY."
    }
}

function Normalize-Tag {
    param([string]$RequestedTag)

    if ([string]::IsNullOrWhiteSpace($RequestedTag)) {
        return ""
    }

    $trimmed = $RequestedTag.Trim()
    if ($trimmed -match '^[vV]\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$') {
        return $trimmed
    }
    if ($trimmed -match '^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$') {
        return "v$trimmed"
    }
    if ($trimmed -ieq "latest") {
        return ""
    }

    throw "Unsupported release tag '$RequestedTag'. Use latest, vX.Y.Z, or X.Y.Z."
}

function Get-LatestReleaseTag {
    param([string]$ResolvedRepository)

    $url = "https://api.github.com/repos/$ResolvedRepository/releases/latest"
    $release = Invoke-RestMethod -Uri $url -Headers @{ "User-Agent" = $UserAgent }

    if ([string]::IsNullOrWhiteSpace($release.tag_name)) {
        throw "GitHub latest release response did not include tag_name for $ResolvedRepository."
    }

    return [string]$release.tag_name
}

function Get-VersionFromTag {
    param([string]$ResolvedTag)

    if ($ResolvedTag -match '^[vV](.+)$') {
        return $Matches[1]
    }

    return $ResolvedTag
}

function Get-VersionFromAssetName {
    param([string]$AssetName)

    if ($AssetName -match '^rebecca-(.+)-windows-x86_64-msvc\.zip$') {
        return $Matches[1]
    }
    if ($AssetName -match '^rebecca-cli-x86_64-pc-windows-msvc\.zip$') {
        return ""
    }

    throw "Unsupported Rebecca release artifact name: $AssetName"
}

function New-InstallerTempRoot {
    $root = Join-Path ([System.IO.Path]::GetTempPath()) ("rebecca-install-" + [System.Guid]::NewGuid().ToString("N"))
    New-Item -ItemType Directory -Force -Path $root | Out-Null
    return $root
}

function Assert-ChildPath {
    param(
        [string]$Parent,
        [string]$Child
    )

    $parentFull = [System.IO.Path]::GetFullPath($Parent).TrimEnd([System.IO.Path]::DirectorySeparatorChar, [System.IO.Path]::AltDirectorySeparatorChar)
    $childFull = [System.IO.Path]::GetFullPath($Child)
    $prefix = $parentFull + [System.IO.Path]::DirectorySeparatorChar

    if (-not $childFull.StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing to modify path outside expected parent: $childFull"
    }
}

function Download-ReleaseFile {
    param(
        [string]$ResolvedRepository,
        [string]$ResolvedTag,
        [string]$FileName,
        [string]$Destination
    )

    $tagSegment = [System.Uri]::EscapeDataString($ResolvedTag)
    $fileSegment = [System.Uri]::EscapeDataString($FileName)
    $url = "https://github.com/$ResolvedRepository/releases/download/$tagSegment/$fileSegment"
    Invoke-WebRequest -Uri $url -OutFile $Destination -Headers @{ "User-Agent" = $UserAgent }
}

function Get-ChecksumEntry {
    param(
        [string]$ChecksumFile,
        [string]$AssetName
    )

    foreach ($line in Get-Content -LiteralPath $ChecksumFile) {
        $parts = $line -split '\s+'
        if ($parts.Length -ge 2 -and $parts[1] -eq $AssetName) {
            $hash = $parts[0].ToLowerInvariant()
            if ($hash -notmatch '^[0-9a-f]{64}$') {
                throw "Invalid SHA-256 entry for $AssetName in $ChecksumFile."
            }
            return $hash
        }
    }

    throw "Checksum file does not contain an entry for $AssetName."
}

function Assert-Checksum {
    param(
        [string]$AssetFile,
        [string]$ChecksumFile,
        [string]$AssetName
    )

    $expected = Get-ChecksumEntry -ChecksumFile $ChecksumFile -AssetName $AssetName
    $actual = (Get-FileHash -LiteralPath $AssetFile -Algorithm SHA256).Hash.ToLowerInvariant()

    if ($expected -ne $actual) {
        throw "Checksum mismatch for $AssetName. Expected $expected but got $actual."
    }

    return $actual
}

function Test-CommandAvailable {
    param([string]$CommandName)

    return $null -ne (Get-Command $CommandName -ErrorAction SilentlyContinue)
}

function Invoke-AttestationVerification {
    param(
        [string]$AssetFile,
        [string]$ChecksumFile,
        [string]$ResolvedRepository,
        [bool]$Required,
        [bool]$Skip
    )

    if ($Skip) {
        return "skipped"
    }

    if ([string]::IsNullOrWhiteSpace($ResolvedRepository)) {
        if ($Required) {
            throw "Attestation verification requires -Repository owner/repo."
        }
        Write-Warning "Skipping attestation verification because repository could not be resolved."
        return "skipped"
    }

    if (-not (Test-CommandAvailable -CommandName "gh")) {
        if ($Required) {
            throw "GitHub CLI is required for -RequireAttestation but gh was not found."
        }
        Write-Warning "GitHub CLI was not found; continuing after checksum verification only."
        return "skipped"
    }

    & gh auth status *> $null
    if ($LASTEXITCODE -ne 0) {
        if ($Required) {
            throw "GitHub CLI is not authenticated; cannot satisfy -RequireAttestation."
        }
        Write-Warning "GitHub CLI is not authenticated; continuing after checksum verification only."
        return "skipped"
    }

    $assetsToVerify = @($AssetFile, $ChecksumFile)
    foreach ($asset in $assetsToVerify) {
        & gh attestation verify $asset --repo $ResolvedRepository --deny-self-hosted-runners
        if ($LASTEXITCODE -ne 0) {
            if ($Required) {
                throw "Attestation verification failed for $asset."
            }
            Write-Warning "Attestation verification failed for $asset; continuing after checksum verification only."
            return "failed-optional"
        }
    }

    return "verified"
}

function Resolve-InstallInputs {
    param(
        [string]$ResolvedRepository,
        [string]$ResolvedTag,
        [string]$LocalAssetPath,
        [string]$LocalChecksumsPath,
        [string]$TempRoot
    )

    if (-not [string]::IsNullOrWhiteSpace($LocalAssetPath) -or -not [string]::IsNullOrWhiteSpace($LocalChecksumsPath)) {
        if ([string]::IsNullOrWhiteSpace($LocalAssetPath) -or [string]::IsNullOrWhiteSpace($LocalChecksumsPath)) {
            throw "Pass both -AssetPath and -ChecksumsPath for local install smoke mode."
        }

        $assetFull = (Resolve-Path -LiteralPath $LocalAssetPath).ProviderPath
        $checksumFull = (Resolve-Path -LiteralPath $LocalChecksumsPath).ProviderPath
        $assetName = [System.IO.Path]::GetFileName($assetFull)
        $version = Get-VersionFromAssetName -AssetName $assetName
        if ([string]::IsNullOrWhiteSpace($version)) {
            $version = Get-VersionFromTag -ResolvedTag $ResolvedTag
        }

        return [pscustomobject]@{
            AssetFile = $assetFull
            ChecksumFile = $checksumFull
            AssetName = $assetName
            Version = $version
            Tag = $ResolvedTag
        }
    }

    Assert-Repository -ResolvedRepository $ResolvedRepository

    $effectiveTag = Normalize-Tag -RequestedTag $ResolvedTag
    if ([string]::IsNullOrWhiteSpace($effectiveTag)) {
        $effectiveTag = Get-LatestReleaseTag -ResolvedRepository $ResolvedRepository
    }

    $version = Get-VersionFromTag -ResolvedTag $effectiveTag
    $assetName = "rebecca-cli-x86_64-pc-windows-msvc.zip"
    $assetFile = Join-Path $TempRoot $assetName
    $checksumFile = Join-Path $TempRoot "sha256.sum"

    Download-ReleaseFile -ResolvedRepository $ResolvedRepository -ResolvedTag $effectiveTag -FileName $assetName -Destination $assetFile
    Download-ReleaseFile -ResolvedRepository $ResolvedRepository -ResolvedTag $effectiveTag -FileName "sha256.sum" -Destination $checksumFile

    return [pscustomobject]@{
        AssetFile = $assetFile
        ChecksumFile = $checksumFile
        AssetName = $assetName
        Version = $version
        Tag = $effectiveTag
    }
}

function Install-FromArchive {
    param(
        [string]$AssetFile,
        [string]$AssetName,
        [string]$InstallDirFull,
        [string]$Version,
        [string]$Repository,
        [string]$Tag,
        [string]$Checksum,
        [string]$AttestationStatus,
        [string]$TempRoot
    )

    $extractDir = Join-Path $TempRoot "extract"
    New-Item -ItemType Directory -Force -Path $extractDir | Out-Null
    Expand-Archive -LiteralPath $AssetFile -DestinationPath $extractDir -Force

    $payloadDir = Join-Path $extractDir ([System.IO.Path]::GetFileNameWithoutExtension($AssetName))
    if (-not (Test-Path -LiteralPath $payloadDir -PathType Container)) {
        $payloadDir = $extractDir
    }

    $binary = Join-Path $payloadDir "rebecca.exe"
    if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) {
        throw "Release archive did not contain rebecca.exe."
    }

    New-Item -ItemType Directory -Force -Path $InstallDirFull | Out-Null
    $installDirResolved = (Resolve-Path -LiteralPath $InstallDirFull).ProviderPath

    $newBinary = Join-Path $installDirResolved "rebecca.exe.new"
    $finalBinary = Join-Path $installDirResolved "rebecca.exe"
    Assert-ChildPath -Parent $installDirResolved -Child $newBinary
    Assert-ChildPath -Parent $installDirResolved -Child $finalBinary
    Copy-Item -LiteralPath $binary -Destination $newBinary -Force
    Move-Item -LiteralPath $newBinary -Destination $finalBinary -Force

    foreach ($fileName in @("README.md", "SECURITY.md", "VERSION.txt")) {
        $source = Join-Path $payloadDir $fileName
        if (Test-Path -LiteralPath $source -PathType Leaf) {
            Copy-Item -LiteralPath $source -Destination $installDirResolved -Force
        }
    }

    $sourceDocs = Join-Path $payloadDir "docs"
    if (Test-Path -LiteralPath $sourceDocs -PathType Container) {
        $targetDocs = Join-Path $installDirResolved "docs"
        New-Item -ItemType Directory -Force -Path $targetDocs | Out-Null
        Get-ChildItem -LiteralPath $sourceDocs -File | ForEach-Object {
            Copy-Item -LiteralPath $_.FullName -Destination $targetDocs -Force
        }
    }

    $installRecord = [ordered]@{
        name = "rebecca"
        version = $Version
        repository = $Repository
        tag = $Tag
        artifact = $AssetName
        sha256 = $Checksum
        attestation = $AttestationStatus
        installed_at_utc = (Get-Date).ToUniversalTime().ToString("o")
    }
    $installRecord | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $installDirResolved "install.json") -Encoding utf8

    return $finalBinary
}

function Write-PathHint {
    param([string]$InstallDirFull)

    $pathEntries = ($env:Path -split ';') | Where-Object { -not [string]::IsNullOrWhiteSpace($_) }
    $alreadyOnPath = $false
    foreach ($entry in $pathEntries) {
        try {
            if ([System.IO.Path]::GetFullPath($entry.Trim('"')).TrimEnd('\') -ieq $InstallDirFull.TrimEnd('\')) {
                $alreadyOnPath = $true
                break
            }
        }
        catch {
            continue
        }
    }

    if (-not $alreadyOnPath) {
        Write-Host "Install directory is not on PATH: $InstallDirFull"
        Write-Host "Run directly: `"$InstallDirFull\rebecca.exe`" --version"
        Write-Host "Add this directory to your user PATH if you want to run rebecca from any terminal."
    }
}

if ($RequireAttestation -and $SkipAttestation) {
    throw "Use either -RequireAttestation or -SkipAttestation, not both."
}

$resolvedRepository = Resolve-Repository -RequestedRepository $Repository
$installDirFull = Resolve-InstallDir -RequestedInstallDir $InstallDir
$tempRoot = New-InstallerTempRoot

try {
    $inputs = Resolve-InstallInputs `
        -ResolvedRepository $resolvedRepository `
        -ResolvedTag $Tag `
        -LocalAssetPath $AssetPath `
        -LocalChecksumsPath $ChecksumsPath `
        -TempRoot $tempRoot

    $checksum = Assert-Checksum -AssetFile $inputs.AssetFile -ChecksumFile $inputs.ChecksumFile -AssetName $inputs.AssetName
    Write-Host "Verified checksum for $($inputs.AssetName)"

    $attestationStatus = Invoke-AttestationVerification `
        -AssetFile $inputs.AssetFile `
        -ChecksumFile $inputs.ChecksumFile `
        -ResolvedRepository $resolvedRepository `
        -Required ([bool]$RequireAttestation) `
        -Skip ([bool]$SkipAttestation)

    if ($attestationStatus -eq "verified") {
        Write-Host "Verified GitHub build provenance."
    }
    elseif ($attestationStatus -eq "skipped") {
        Write-Host "Installed after checksum verification only; attestation was skipped."
    }
    elseif ($attestationStatus -eq "failed-optional") {
        Write-Host "Installed after checksum verification only; optional attestation failed."
    }

    $binary = Install-FromArchive `
        -AssetFile $inputs.AssetFile `
        -AssetName $inputs.AssetName `
        -InstallDirFull $installDirFull `
        -Version $inputs.Version `
        -Repository $resolvedRepository `
        -Tag $inputs.Tag `
        -Checksum $checksum `
        -AttestationStatus $attestationStatus `
        -TempRoot $tempRoot

    Write-Host "Installed Rebecca to $binary"
    & $binary --version
    Write-PathHint -InstallDirFull (Split-Path -Parent $binary)
}
finally {
    if (-not [string]::IsNullOrWhiteSpace($tempRoot) -and (Test-Path -LiteralPath $tempRoot)) {
        Assert-ChildPath -Parent ([System.IO.Path]::GetTempPath()) -Child $tempRoot
        Remove-Item -LiteralPath $tempRoot -Recurse -Force
    }
}

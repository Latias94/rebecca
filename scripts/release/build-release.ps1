param(
    [string]$Tag = $env:GITHUB_REF_NAME,
    [string]$Repository = $env:GITHUB_REPOSITORY,
    [string]$Target = "x86_64-pc-windows-msvc",
    [string]$OutDir = "dist",
    [switch]$SkipBuild
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Get-RepoRoot {
    $scriptRoot = Split-Path -Parent $PSCommandPath
    return (Resolve-Path -LiteralPath (Join-Path $scriptRoot "..\..")).ProviderPath
}

function Get-WorkspaceVersion {
    param([string]$RepoRoot)

    $cargoToml = Join-Path $RepoRoot "Cargo.toml"
    $versionLine = Get-Content -LiteralPath $cargoToml |
        Where-Object { $_ -match '^\s*version\s*=\s*"([^"]+)"' } |
        Select-Object -First 1

    if (-not $versionLine) {
        throw "Could not find workspace package version in Cargo.toml"
    }

    if ($versionLine -notmatch '^\s*version\s*=\s*"([^"]+)"') {
        throw "Could not parse workspace package version from Cargo.toml"
    }

    return $Matches[1]
}

function Resolve-ReleaseVersion {
    param(
        [string]$RepoRoot,
        [string]$Tag
    )

    if ([string]::IsNullOrWhiteSpace($Tag)) {
        return Get-WorkspaceVersion -RepoRoot $RepoRoot
    }

    $normalizedTag = $Tag.Trim()
    if ($normalizedTag.StartsWith("refs/tags/")) {
        $normalizedTag = $normalizedTag.Substring("refs/tags/".Length)
    }

    if ($normalizedTag -match '^[vV](\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?)$') {
        return $Matches[1]
    }

    if ($normalizedTag -match '^(\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?)$') {
        return $Matches[1]
    }

    return Get-WorkspaceVersion -RepoRoot $RepoRoot
}

function Resolve-TargetLabel {
    param([string]$Target)

    switch ($Target) {
        "x86_64-pc-windows-msvc" { return "windows-x86_64-msvc" }
        default { throw "Unsupported release target: $Target" }
    }
}

function Resolve-OutputDirectory {
    param(
        [string]$RepoRoot,
        [string]$OutDir
    )

    if ([System.IO.Path]::IsPathRooted($OutDir)) {
        return [System.IO.Path]::GetFullPath($OutDir)
    }

    return [System.IO.Path]::GetFullPath((Join-Path $RepoRoot $OutDir))
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
        throw "Refusing to modify path outside output directory: $childFull"
    }
}

$repoRoot = Get-RepoRoot
$version = Resolve-ReleaseVersion -RepoRoot $repoRoot -Tag $Tag
$targetLabel = Resolve-TargetLabel -Target $Target
$outDirFull = Resolve-OutputDirectory -RepoRoot $repoRoot -OutDir $OutDir
$artifactName = "rebecca-$version-$targetLabel"
$stageDir = Join-Path $outDirFull $artifactName
$archivePath = Join-Path $outDirFull "$artifactName.zip"

Push-Location $repoRoot
try {
    if (-not $SkipBuild) {
        cargo build -p rebecca-cli --bin rebecca --release --locked --target $Target
    }

    $binaryPath = Join-Path $repoRoot "target\$Target\release\rebecca.exe"
    if (-not (Test-Path -LiteralPath $binaryPath -PathType Leaf)) {
        throw "Expected release binary was not found: $binaryPath"
    }

    New-Item -ItemType Directory -Force -Path $outDirFull | Out-Null

    if (Test-Path -LiteralPath $stageDir) {
        Assert-ChildPath -Parent $outDirFull -Child $stageDir
        Remove-Item -LiteralPath $stageDir -Recurse -Force
    }
    if (Test-Path -LiteralPath $archivePath) {
        Assert-ChildPath -Parent $outDirFull -Child $archivePath
        Remove-Item -LiteralPath $archivePath -Force
    }

    New-Item -ItemType Directory -Force -Path $stageDir | Out-Null
    New-Item -ItemType Directory -Force -Path (Join-Path $stageDir "docs") | Out-Null
    New-Item -ItemType Directory -Force -Path (Join-Path $stageDir "scripts") | Out-Null

    Copy-Item -LiteralPath $binaryPath -Destination (Join-Path $stageDir "rebecca.exe")
    Copy-Item -LiteralPath (Join-Path $repoRoot "README.md") -Destination $stageDir
    Copy-Item -LiteralPath (Join-Path $repoRoot "CHANGELOG.md") -Destination $stageDir
    Copy-Item -LiteralPath (Join-Path $repoRoot "LICENSE") -Destination $stageDir
    Copy-Item -LiteralPath (Join-Path $repoRoot "LICENSE-APACHE") -Destination $stageDir
    Copy-Item -LiteralPath (Join-Path $repoRoot "LICENSE-MIT") -Destination $stageDir
    Copy-Item -LiteralPath (Join-Path $repoRoot "SECURITY.md") -Destination $stageDir
    Copy-Item -LiteralPath (Join-Path $repoRoot "docs\security-audit.md") -Destination (Join-Path $stageDir "docs")
    Copy-Item -LiteralPath (Join-Path $repoRoot "docs\release.md") -Destination (Join-Path $stageDir "docs")
    Copy-Item -LiteralPath (Join-Path $repoRoot "scripts\install.ps1") -Destination (Join-Path $stageDir "scripts")

    $metadata = @(
        "name=rebecca",
        "version=$version",
        "target=$Target",
        "tag=$Tag",
        "repository=$Repository",
        "commit=$env:GITHUB_SHA"
    )
    Set-Content -LiteralPath (Join-Path $stageDir "VERSION.txt") -Value $metadata -Encoding utf8

    Compress-Archive -LiteralPath $stageDir -DestinationPath $archivePath -Force

    if ($env:GITHUB_OUTPUT) {
        "artifact_path=$archivePath" | Out-File -LiteralPath $env:GITHUB_OUTPUT -Append -Encoding utf8
        "artifact_name=$([System.IO.Path]::GetFileName($archivePath))" | Out-File -LiteralPath $env:GITHUB_OUTPUT -Append -Encoding utf8
    }

    Write-Host "Created release artifact: $archivePath"
}
finally {
    Pop-Location
}

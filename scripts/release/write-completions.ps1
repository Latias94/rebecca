param(
    [string]$BinaryPath = "",
    [string]$OutDir = "dist\completions"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Get-RepoRoot {
    $scriptRoot = Split-Path -Parent $PSCommandPath
    return (Resolve-Path -LiteralPath (Join-Path $scriptRoot "..\..")).ProviderPath
}

function Resolve-PathForRepo {
    param(
        [string]$RepoRoot,
        [string]$Path
    )

    if ([System.IO.Path]::IsPathRooted($Path)) {
        return [System.IO.Path]::GetFullPath($Path)
    }

    return [System.IO.Path]::GetFullPath((Join-Path $RepoRoot $Path))
}

function Resolve-BinaryPath {
    param(
        [string]$RepoRoot,
        [string]$RequestedBinaryPath
    )

    if (-not [string]::IsNullOrWhiteSpace($RequestedBinaryPath)) {
        return Resolve-PathForRepo -RepoRoot $RepoRoot -Path $RequestedBinaryPath
    }

    $defaultBinary = Join-Path $RepoRoot "target\x86_64-pc-windows-msvc\release\rebecca.exe"
    return [System.IO.Path]::GetFullPath($defaultBinary)
}

function Write-CompletionFile {
    param(
        [string]$BinaryPath,
        [string]$Shell,
        [string]$Destination
    )

    $output = & $BinaryPath completion $Shell
    if ($LASTEXITCODE -ne 0) {
        throw "Failed to generate $Shell completion with $BinaryPath"
    }

    if (-not $output -or [string]::IsNullOrWhiteSpace(($output -join "`n"))) {
        throw "Generated $Shell completion was empty."
    }

    Set-Content -LiteralPath $Destination -Value $output -Encoding utf8
}

$repoRoot = Get-RepoRoot
$binaryFull = Resolve-BinaryPath -RepoRoot $repoRoot -RequestedBinaryPath $BinaryPath
$outDirFull = Resolve-PathForRepo -RepoRoot $repoRoot -Path $OutDir

if (-not (Test-Path -LiteralPath $binaryFull -PathType Leaf)) {
    throw "Rebecca binary was not found: $binaryFull"
}

New-Item -ItemType Directory -Force -Path $outDirFull | Out-Null

$completionFiles = @(
    @{ Shell = "bash"; File = "rebecca.bash" },
    @{ Shell = "zsh"; File = "_rebecca" },
    @{ Shell = "fish"; File = "rebecca.fish" },
    @{ Shell = "powershell"; File = "rebecca.ps1" },
    @{ Shell = "elvish"; File = "rebecca.elv" }
)

foreach ($completion in $completionFiles) {
    $destination = Join-Path $outDirFull $completion.File
    Write-CompletionFile -BinaryPath $binaryFull -Shell $completion.Shell -Destination $destination
}

Write-Host "Wrote shell completions: $outDirFull"

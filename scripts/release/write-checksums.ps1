param(
    [string]$DistDir = "dist",
    [string]$OutputFile = ""
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

$repoRoot = Get-RepoRoot
$distDirFull = Resolve-PathForRepo -RepoRoot $repoRoot -Path $DistDir

if (-not (Test-Path -LiteralPath $distDirFull -PathType Container)) {
    throw "Distribution directory does not exist: $distDirFull"
}

if ([string]::IsNullOrWhiteSpace($OutputFile)) {
    $outputFileFull = Join-Path $distDirFull "SHA256SUMS"
}
else {
    $outputFileFull = Resolve-PathForRepo -RepoRoot $repoRoot -Path $OutputFile
}

$outputName = [System.IO.Path]::GetFileName($outputFileFull)
$files = Get-ChildItem -LiteralPath $distDirFull -File |
    Where-Object { $_.Name -ne $outputName } |
    Sort-Object Name

if (-not $files) {
    throw "No release files found in $distDirFull"
}

$lines = foreach ($file in $files) {
    $hash = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    "$hash  $($file.Name)"
}

Set-Content -LiteralPath $outputFileFull -Value $lines -Encoding ascii
Write-Host "Wrote checksums: $outputFileFull"

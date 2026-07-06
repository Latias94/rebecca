param(
    [string]$Target = "x86_64-unknown-linux-gnu",
    [string]$ZigTarget = "x86_64-linux-gnu",
    [string[]]$CargoArgs = @("--workspace", "--all-targets")
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Get-RepoRoot {
    return (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "..\..")).ProviderPath
}

function Require-Command {
    param(
        [string]$Name,
        [string]$InstallHint
    )

    if ($null -eq (Get-Command $Name -ErrorAction SilentlyContinue)) {
        throw "$Name was not found on PATH. $InstallHint"
    }
}

function Require-RustTarget {
    param([string]$Target)

    $installedTargets = @(rustup target list --installed)
    if ($installedTargets -notcontains $Target) {
        throw "Rust target '$Target' is not installed. Run: rustup target add $Target"
    }
}

$repoRoot = Get-RepoRoot
Require-Command -Name "zig" -InstallHint "Install Zig or use a Linux runner for this check."
Require-Command -Name "rustup" -InstallHint "Install rustup and the Rust toolchain."
Require-RustTarget -Target $Target

$ccEnvName = "CC_$($Target -replace "-", "_")"
$previousCc = [Environment]::GetEnvironmentVariable($ccEnvName, "Process")
$previousNoDefaults = [Environment]::GetEnvironmentVariable("CRATE_CC_NO_DEFAULTS", "Process")

try {
    [Environment]::SetEnvironmentVariable($ccEnvName, "zig cc -target $ZigTarget", "Process")
    [Environment]::SetEnvironmentVariable("CRATE_CC_NO_DEFAULTS", "1", "Process")

    Push-Location $repoRoot
    try {
        cargo clippy @CargoArgs --target $Target -- -D warnings
    } finally {
        Pop-Location
    }
} finally {
    [Environment]::SetEnvironmentVariable($ccEnvName, $previousCc, "Process")
    [Environment]::SetEnvironmentVariable("CRATE_CC_NO_DEFAULTS", $previousNoDefaults, "Process")
}

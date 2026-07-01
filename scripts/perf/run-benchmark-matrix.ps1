param(
    [string]$Package = "rebecca-core",
    [string]$Bench = "perf_matrix",
    [string]$OutputPath = "",
    [switch]$SkipRun
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Get-RepoRoot {
    return (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "..\..")).ProviderPath
}

function Get-UnixTimeSeconds {
    return [int64]([DateTimeOffset]::UtcNow.ToUnixTimeSeconds())
}

function Invoke-CheckedCommand {
    param([string[]]$Command)

    & $Command[0] @($Command | Select-Object -Skip 1)
    if ($LASTEXITCODE -ne 0) {
        throw "Command failed with exit code ${LASTEXITCODE}: $($Command -join ' ')"
    }
}

function Get-GitCommit {
    param([string]$RepoRoot)

    $commit = (& git -C $RepoRoot rev-parse --short HEAD 2>$null)
    if ($LASTEXITCODE -ne 0) {
        return ""
    }
    return [string]$commit
}

function Convert-Estimate {
    param(
        [object]$Scenario,
        [object]$Estimate
    )

    return [pscustomobject]@{
        scenario = $Scenario.scenario
        operation = $Scenario.operation
        backend = $Scenario.backend
        fixture = $Scenario.fixture
        physical_files = $Scenario.physical_files
        physical_directories = $Scenario.physical_directories
        expected_bytes = $Scenario.expected_bytes
        progress_events = $Scenario.progress_events
        target_count = $Scenario.target_count
        cache_mode = $Scenario.cache_mode
        delete_mode = $Scenario.delete_mode
        mean_ns = $Estimate.mean.point_estimate
        median_ns = $Estimate.median.point_estimate
        mean_confidence_interval_ns = [pscustomobject]@{
            lower_bound = $Estimate.mean.confidence_interval.lower_bound
            upper_bound = $Estimate.mean.confidence_interval.upper_bound
        }
    }
}

$repoRoot = Get-RepoRoot
$manifestPath = Join-Path $repoRoot "target\perf\$Bench-scenarios.json"
if ([string]::IsNullOrWhiteSpace($OutputPath)) {
    $OutputPath = Join-Path $repoRoot "target\perf\$Package-$Bench-report.json"
}

Push-Location $repoRoot
try {
    $env:REBECCA_PERF_MATRIX_MANIFEST = $manifestPath

    if (-not $SkipRun) {
        Invoke-CheckedCommand -Command @("cargo", "bench", "-p", $Package, "--bench", $Bench)
    }

    if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
        throw "Benchmark scenario manifest was not found at $manifestPath. Run without -SkipRun first."
    }

    $manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
    $criterionRoot = Join-Path $repoRoot "target\criterion\$Bench"
    if (-not (Test-Path -LiteralPath $criterionRoot -PathType Container)) {
        throw "Criterion output was not found at $criterionRoot. Run without -SkipRun first."
    }

    $estimatesByScenario = @{}
    Get-ChildItem -LiteralPath $criterionRoot -Recurse -Filter estimates.json |
        Where-Object { $_.Directory.Name -eq "new" } |
        ForEach-Object {
            $scenario = Split-Path -Leaf (Split-Path -Parent $_.DirectoryName)
            $estimatesByScenario[$scenario] = Get-Content -LiteralPath $_.FullName -Raw | ConvertFrom-Json
        }

    $scenarios = @()
    foreach ($scenario in $manifest.scenarios) {
        if (-not $estimatesByScenario.ContainsKey($scenario.scenario)) {
            throw "Missing Criterion estimate for scenario '$($scenario.scenario)'."
        }
        $scenarios += Convert-Estimate -Scenario $scenario -Estimate $estimatesByScenario[$scenario.scenario]
    }

    $report = [pscustomobject]@{
        schema_version = 1
        generated_at_unix_seconds = Get-UnixTimeSeconds
        package = $Package
        bench = $Bench
        git_commit = Get-GitCommit -RepoRoot $repoRoot
        scenario_manifest = $manifestPath
        criterion_root = $criterionRoot
        scenarios = $scenarios
    }

    $outputParent = Split-Path -Parent $OutputPath
    New-Item -ItemType Directory -Force -Path $outputParent | Out-Null
    $report | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $OutputPath -Encoding utf8
    Write-Host "Wrote benchmark matrix report to $OutputPath"
}
finally {
    Pop-Location
    if (Test-Path Env:\REBECCA_PERF_MATRIX_MANIFEST) {
        Remove-Item Env:\REBECCA_PERF_MATRIX_MANIFEST
    }
}

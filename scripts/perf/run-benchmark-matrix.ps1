param(
    [string]$Package = "rebecca-core",
    [string]$Bench = "perf_matrix",
    [string]$OutputPath = "",
    [string]$OutputDirectory = "",
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

function Get-OutputPaths {
    param(
        [string]$RepoRoot,
        [string]$Package,
        [string]$Bench,
        [string]$OutputPath,
        [string]$OutputDirectory
    )

    if ([string]::IsNullOrWhiteSpace($OutputPath)) {
        if ([string]::IsNullOrWhiteSpace($OutputDirectory)) {
            $OutputDirectory = Join-Path $RepoRoot "target\perf"
        }
        $OutputPath = Join-Path $OutputDirectory "$Package-$Bench-report.json"
    }

    $jsonPath = [IO.Path]::GetFullPath($OutputPath)
    $outputParent = Split-Path -Parent $jsonPath
    $stem = [IO.Path]::GetFileNameWithoutExtension($jsonPath)

    return [pscustomobject]@{
        json = $jsonPath
        csv = Join-Path $outputParent "$stem-scenarios.csv"
        markdown = Join-Path $outputParent "$stem-summary.md"
    }
}

function Get-ScenarioText {
    param(
        [object]$Scenario,
        [string]$PropertyName
    )

    if ($null -eq $Scenario) {
        return ""
    }
    if ($null -eq $Scenario.PSObject.Properties[$PropertyName]) {
        return ""
    }
    if ($null -eq $Scenario.$PropertyName) {
        return ""
    }
    return [string]$Scenario.$PropertyName
}

function Get-ScenarioNumber {
    param(
        [object]$Scenario,
        [string]$PropertyName
    )

    if ($null -eq $Scenario) {
        return $null
    }
    if ($null -eq $Scenario.PSObject.Properties[$PropertyName]) {
        return $null
    }
    if ($null -eq $Scenario.$PropertyName) {
        return $null
    }
    return [int64]$Scenario.$PropertyName
}

function Get-EstimatePoint {
    param(
        [object]$Estimate,
        [string]$PropertyName
    )

    if ($null -eq $Estimate) {
        return $null
    }
    if ($null -eq $Estimate.PSObject.Properties[$PropertyName]) {
        return $null
    }
    if ($null -eq $Estimate.$PropertyName) {
        return $null
    }
    if ($null -eq $Estimate.$PropertyName.PSObject.Properties["point_estimate"]) {
        return $null
    }
    if ($null -eq $Estimate.$PropertyName.point_estimate) {
        return $null
    }
    return [double]$Estimate.$PropertyName.point_estimate
}

function Get-EstimateConfidenceInterval {
    param(
        [object]$Estimate,
        [string]$PropertyName
    )

    if ($null -eq $Estimate) {
        return [pscustomobject]@{
            lower_bound = $null
            upper_bound = $null
        }
    }
    if ($null -eq $Estimate.PSObject.Properties[$PropertyName]) {
        return [pscustomobject]@{
            lower_bound = $null
            upper_bound = $null
        }
    }
    if ($null -eq $Estimate.$PropertyName) {
        return [pscustomobject]@{
            lower_bound = $null
            upper_bound = $null
        }
    }
    if ($null -eq $Estimate.$PropertyName.PSObject.Properties["confidence_interval"]) {
        return [pscustomobject]@{
            lower_bound = $null
            upper_bound = $null
        }
    }
    $interval = $Estimate.$PropertyName.confidence_interval
    return [pscustomobject]@{
        lower_bound = $interval.lower_bound
        upper_bound = $interval.upper_bound
    }
}

function Convert-Estimate {
    param(
        [object]$Scenario,
        [object]$Estimate,
        [string]$Status,
        [string]$StatusReason
    )

    return [pscustomobject]@{
        scenario = Get-ScenarioText -Scenario $Scenario -PropertyName "scenario"
        operation = Get-ScenarioText -Scenario $Scenario -PropertyName "operation"
        backend = Get-ScenarioText -Scenario $Scenario -PropertyName "backend"
        backend_source_expectation = Get-ScenarioText -Scenario $Scenario -PropertyName "backend_source_expectation"
        fixture = Get-ScenarioText -Scenario $Scenario -PropertyName "fixture"
        physical_files = Get-ScenarioNumber -Scenario $Scenario -PropertyName "physical_files"
        physical_directories = Get-ScenarioNumber -Scenario $Scenario -PropertyName "physical_directories"
        expected_bytes = Get-ScenarioNumber -Scenario $Scenario -PropertyName "expected_bytes"
        progress_events = Get-ScenarioNumber -Scenario $Scenario -PropertyName "progress_events"
        target_count = Get-ScenarioNumber -Scenario $Scenario -PropertyName "target_count"
        cache_mode = Get-ScenarioText -Scenario $Scenario -PropertyName "cache_mode"
        delete_mode = Get-ScenarioText -Scenario $Scenario -PropertyName "delete_mode"
        status = $Status
        status_reason = $StatusReason
        mean_ns = Get-EstimatePoint -Estimate $Estimate -PropertyName "mean"
        median_ns = Get-EstimatePoint -Estimate $Estimate -PropertyName "median"
        mean_confidence_interval_ns = Get-EstimateConfidenceInterval -Estimate $Estimate -PropertyName "mean"
    }
}

function Convert-ScenarioForCsv {
    param([object]$Scenario)

    return [pscustomobject]@{
        scenario = $Scenario.scenario
        operation = $Scenario.operation
        backend = $Scenario.backend
        backend_source_expectation = $Scenario.backend_source_expectation
        fixture = $Scenario.fixture
        physical_files = $Scenario.physical_files
        physical_directories = $Scenario.physical_directories
        expected_bytes = $Scenario.expected_bytes
        progress_events = $Scenario.progress_events
        target_count = $Scenario.target_count
        cache_mode = $Scenario.cache_mode
        delete_mode = $Scenario.delete_mode
        status = $Scenario.status
        status_reason = $Scenario.status_reason
        mean_ns = $Scenario.mean_ns
        median_ns = $Scenario.median_ns
        mean_ci_lower_ns = $Scenario.mean_confidence_interval_ns.lower_bound
        mean_ci_upper_ns = $Scenario.mean_confidence_interval_ns.upper_bound
    }
}

function Convert-NsToMs {
    param([object]$Value)

    if ($null -eq $Value) {
        return ""
    }
    return "{0:n3}" -f ([double]$Value / 1000000.0)
}

function New-MarkdownReport {
    param([object]$Report)

    $lines = @()
    $lines += "# Rebecca Performance Matrix"
    $lines += ""
    $lines += "- Status: $($Report.status)"
    $lines += "- Package: $($Report.package)"
    $lines += "- Bench: $($Report.bench)"
    $lines += "- Git commit: $($Report.git_commit)"
    $lines += "- Scenario manifest: $($Report.scenario_manifest)"
    $lines += "- Criterion root: $($Report.criterion_root)"
    if (-not [string]::IsNullOrWhiteSpace($Report.status_reason)) {
        $lines += "- Status reason: $($Report.status_reason)"
    }
    $lines += ""
    $lines += "| Scenario | Operation | Backend | Status | Mean ms | Median ms |"
    $lines += "| --- | --- | --- | --- | ---: | ---: |"
    foreach ($scenario in $Report.scenarios) {
        $lines += "| $($scenario.scenario) | $($scenario.operation) | $($scenario.backend) | $($scenario.status) | $(Convert-NsToMs -Value $scenario.mean_ns) | $(Convert-NsToMs -Value $scenario.median_ns) |"
    }
    if ($Report.scenarios.Count -eq 0) {
        $lines += "| _(none)_ |  |  | $($Report.status) |  |  |"
    }
    return ($lines -join [Environment]::NewLine)
}

function Write-ReportArtifacts {
    param(
        [object]$Report,
        [object]$Paths
    )

    $outputParent = Split-Path -Parent $Paths.json
    New-Item -ItemType Directory -Force -Path $outputParent | Out-Null

    $Report | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $Paths.json -Encoding utf8

    $csvRows = @($Report.scenarios | ForEach-Object { Convert-ScenarioForCsv -Scenario $_ })
    if ($csvRows.Count -gt 0) {
        $csvRows | Export-Csv -LiteralPath $Paths.csv -NoTypeInformation -Encoding utf8
    }
    else {
        "scenario,operation,backend,backend_source_expectation,fixture,physical_files,physical_directories,expected_bytes,progress_events,target_count,cache_mode,delete_mode,status,status_reason,mean_ns,median_ns,mean_ci_lower_ns,mean_ci_upper_ns" |
            Set-Content -LiteralPath $Paths.csv -Encoding utf8
    }

    New-MarkdownReport -Report $Report | Set-Content -LiteralPath $Paths.markdown -Encoding utf8
}

function Get-ReportStatus {
    param([object[]]$Scenarios)

    if ($Scenarios.Count -eq 0) {
        return "skipped"
    }
    $missing = @($Scenarios | Where-Object { $_.status -ne "completed" })
    if ($missing.Count -gt 0) {
        return "partial"
    }
    return "completed"
}

$repoRoot = Get-RepoRoot
$manifestPath = Join-Path $repoRoot "target\perf\$Bench-scenarios.json"
$outputPaths = Get-OutputPaths -RepoRoot $repoRoot -Package $Package -Bench $Bench -OutputPath $OutputPath -OutputDirectory $OutputDirectory

Push-Location $repoRoot
try {
    $env:REBECCA_PERF_MATRIX_MANIFEST = $manifestPath

    if (-not $SkipRun) {
        Invoke-CheckedCommand -Command @("cargo", "bench", "-p", $Package, "--bench", $Bench)
    }

    $manifestExists = Test-Path -LiteralPath $manifestPath -PathType Leaf
    $criterionRoot = Join-Path $repoRoot "target\criterion\$Bench"
    $criterionExists = Test-Path -LiteralPath $criterionRoot -PathType Container

    if (-not $manifestExists -and -not $SkipRun) {
        throw "Benchmark scenario manifest was not found at $manifestPath. Run without -SkipRun first."
    }
    if (-not $criterionExists -and -not $SkipRun) {
        throw "Criterion output was not found at $criterionRoot. Run without -SkipRun first."
    }

    $statusReason = ""
    $manifest = $null
    if ($manifestExists) {
        $manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
    }
    elseif ($SkipRun) {
        $statusReason = "skip-run requested and benchmark scenario manifest was not found"
    }

    $estimatesByScenario = @{}
    if ($criterionExists) {
        Get-ChildItem -LiteralPath $criterionRoot -Recurse -Filter estimates.json |
            Where-Object { $_.Directory.Name -eq "new" } |
            ForEach-Object {
                $scenario = Split-Path -Leaf (Split-Path -Parent $_.DirectoryName)
                $estimatesByScenario[$scenario] = Get-Content -LiteralPath $_.FullName -Raw | ConvertFrom-Json
            }
    }
    elseif ($SkipRun -and [string]::IsNullOrWhiteSpace($statusReason)) {
        $statusReason = "skip-run requested and Criterion output was not found"
    }

    $scenarios = @()
    if ($null -ne $manifest) {
        foreach ($scenario in $manifest.scenarios) {
            if ($estimatesByScenario.ContainsKey($scenario.scenario)) {
                $scenarios += Convert-Estimate -Scenario $scenario -Estimate $estimatesByScenario[$scenario.scenario] -Status "completed" -StatusReason ""
            }
            elseif ($SkipRun) {
                $scenarios += Convert-Estimate -Scenario $scenario -Estimate $null -Status "missing-estimate" -StatusReason "Criterion estimate was not found during skip-run report generation"
            }
            else {
                throw "Missing Criterion estimate for scenario '$($scenario.scenario)'."
            }
        }
    }

    $report = [pscustomobject]@{
        schema_version = 3
        generated_at_unix_seconds = Get-UnixTimeSeconds
        status = Get-ReportStatus -Scenarios @($scenarios)
        status_reason = $statusReason
        run_attempted = -not $SkipRun
        package = $Package
        bench = $Bench
        git_commit = Get-GitCommit -RepoRoot $repoRoot
        scenario_manifest = $manifestPath
        criterion_root = $criterionRoot
        report_artifacts = [pscustomobject]@{
            json = $outputPaths.json
            csv = $outputPaths.csv
            markdown = $outputPaths.markdown
        }
        scenarios = @($scenarios)
    }

    Write-ReportArtifacts -Report $report -Paths $outputPaths
    Write-Host "Wrote benchmark matrix JSON report to $($outputPaths.json)"
    Write-Host "Wrote benchmark matrix CSV report to $($outputPaths.csv)"
    Write-Host "Wrote benchmark matrix Markdown report to $($outputPaths.markdown)"
}
finally {
    Pop-Location
    if (Test-Path Env:\REBECCA_PERF_MATRIX_MANIFEST) {
        Remove-Item Env:\REBECCA_PERF_MATRIX_MANIFEST
    }
}

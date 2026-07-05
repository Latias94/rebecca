param(
    [string]$BaselinePath = "",
    [string]$CurrentPath = "",
    [string]$OutputPath = "",
    [string]$OutputDirectory = "",
    [double]$RegressionThresholdPercent = 15.0,
    [double]$ImprovementThresholdPercent = 15.0,
    [switch]$SelfTest
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Get-RepoRoot {
    return (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "..\..")).ProviderPath
}

function Get-UnixTimeSeconds {
    return [int64]([DateTimeOffset]::UtcNow.ToUnixTimeSeconds())
}

function Get-ObjectProperty {
    param([object]$Object, [string]$Name)

    if ($null -eq $Object -or $null -eq $Object.PSObject.Properties[$Name]) {
        return $null
    }
    return $Object.$Name
}

function Get-ScenarioName {
    param([object]$Scenario)

    $name = Get-ObjectProperty -Object $Scenario -Name "scenario"
    if ([string]::IsNullOrWhiteSpace([string]$name)) {
        return ""
    }
    return [string]$name
}

function Get-ScenarioStatus {
    param([object]$Scenario)

    $status = Get-ObjectProperty -Object $Scenario -Name "status"
    if ([string]::IsNullOrWhiteSpace([string]$status)) {
        return "unknown"
    }
    return [string]$status
}

function Get-ScenarioNumber {
    param([object]$Scenario, [string]$Name)

    $value = Get-ObjectProperty -Object $Scenario -Name $Name
    if ($null -eq $value -or [string]::IsNullOrWhiteSpace([string]$value)) {
        return $null
    }
    return [double]$value
}

function Read-JsonReport {
    param([string]$Path)

    if ([string]::IsNullOrWhiteSpace($Path) -or -not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        return $null
    }
    return Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json
}

function New-ScenarioMap {
    param([object]$Report)

    $map = @{}
    if ($null -eq $Report) {
        return $map
    }
    foreach ($scenario in @($Report.scenarios)) {
        $name = Get-ScenarioName -Scenario $scenario
        if (-not [string]::IsNullOrWhiteSpace($name)) {
            $map[$name] = $scenario
        }
    }
    return $map
}

function Get-ComparisonStatus {
    param(
        [object]$Baseline,
        [object]$Current,
        [object]$DeltaPercent,
        [double]$RegressionThreshold,
        [double]$ImprovementThreshold
    )

    if ($null -eq $Baseline) {
        return "missing-baseline"
    }
    if ($null -eq $Current) {
        return "missing-current"
    }
    if ((Get-ScenarioStatus -Scenario $Baseline) -ne "completed" -or (Get-ScenarioStatus -Scenario $Current) -ne "completed") {
        return "skipped"
    }

    $baselineMean = Get-ScenarioNumber -Scenario $Baseline -Name "mean_ns"
    $currentMean = Get-ScenarioNumber -Scenario $Current -Name "mean_ns"
    if ($null -eq $baselineMean -or $null -eq $currentMean -or $baselineMean -le 0 -or $null -eq $DeltaPercent) {
        return "skipped"
    }
    if ($DeltaPercent -gt $RegressionThreshold) {
        return "regression"
    }
    if ($DeltaPercent -lt (-1.0 * $ImprovementThreshold)) {
        return "improvement"
    }
    return "pass"
}

function Get-StatusReason {
    param([string]$Status, [object]$Baseline, [object]$Current)

    switch ($Status) {
        "missing-baseline" { return "scenario is present only in current report" }
        "missing-current" { return "scenario is present only in baseline report" }
        "skipped" {
            $baselineStatus = Get-ScenarioStatus -Scenario $Baseline
            $currentStatus = Get-ScenarioStatus -Scenario $Current
            $baselineMean = Get-ScenarioNumber -Scenario $Baseline -Name "mean_ns"
            $currentMean = Get-ScenarioNumber -Scenario $Current -Name "mean_ns"
            return "baseline_status=$baselineStatus current_status=$currentStatus baseline_mean_ns=$baselineMean current_mean_ns=$currentMean"
        }
        "regression" { return "current mean exceeded baseline threshold" }
        "improvement" { return "current mean improved beyond threshold" }
        default { return "" }
    }
}

function New-ScenarioComparison {
    param(
        [string]$Name,
        [object]$Baseline,
        [object]$Current,
        [double]$RegressionThreshold,
        [double]$ImprovementThreshold
    )

    $representative = if ($null -ne $Current) { $Current } else { $Baseline }
    $baselineMean = Get-ScenarioNumber -Scenario $Baseline -Name "mean_ns"
    $currentMean = Get-ScenarioNumber -Scenario $Current -Name "mean_ns"
    $deltaNs = if ($null -ne $baselineMean -and $null -ne $currentMean) { $currentMean - $baselineMean } else { $null }
    $deltaPercent = if ($null -ne $deltaNs -and $null -ne $baselineMean -and $baselineMean -gt 0) {
        ($deltaNs / $baselineMean) * 100.0
    }
    else {
        $null
    }
    $status = Get-ComparisonStatus `
        -Baseline $Baseline `
        -Current $Current `
        -DeltaPercent $deltaPercent `
        -RegressionThreshold $RegressionThreshold `
        -ImprovementThreshold $ImprovementThreshold

    return [pscustomobject]@{
        scenario = $Name
        operation = [string](Get-ObjectProperty -Object $representative -Name "operation")
        backend = [string](Get-ObjectProperty -Object $representative -Name "backend")
        fixture = [string](Get-ObjectProperty -Object $representative -Name "fixture")
        status = $status
        status_reason = Get-StatusReason -Status $status -Baseline $Baseline -Current $Current
        baseline_status = Get-ScenarioStatus -Scenario $Baseline
        current_status = Get-ScenarioStatus -Scenario $Current
        baseline_mean_ns = $baselineMean
        current_mean_ns = $currentMean
        baseline_median_ns = Get-ScenarioNumber -Scenario $Baseline -Name "median_ns"
        current_median_ns = Get-ScenarioNumber -Scenario $Current -Name "median_ns"
        delta_ns = $deltaNs
        delta_percent = $deltaPercent
        regression_threshold_percent = $RegressionThreshold
        improvement_threshold_percent = $ImprovementThreshold
        baseline_evidence = Get-ObjectProperty -Object $Baseline -Name "evidence"
        current_evidence = Get-ObjectProperty -Object $Current -Name "evidence"
    }
}

function New-StatusCounts {
    param([object[]]$Comparisons)

    $counts = [ordered]@{
        pass = 0
        regression = 0
        improvement = 0
        skipped = 0
        "missing-baseline" = 0
        "missing-current" = 0
    }
    foreach ($comparison in @($Comparisons)) {
        if (-not $counts.Contains($comparison.status)) {
            $counts[$comparison.status] = 0
        }
        $counts[$comparison.status] = [int]$counts[$comparison.status] + 1
    }
    return [pscustomobject]$counts
}

function Get-ReportStatus {
    param([object[]]$Comparisons, [bool]$BaselineMissing, [bool]$CurrentMissing)

    if ($BaselineMissing -and $CurrentMissing) {
        return "missing-baseline-and-current"
    }
    if ($BaselineMissing) {
        return "missing-baseline"
    }
    if ($CurrentMissing) {
        return "missing-current"
    }
    if ($Comparisons.Count -eq 0) {
        return "skipped"
    }
    if (@($Comparisons | Where-Object { $_.status -eq "regression" }).Count -gt 0) {
        return "regression"
    }
    if (@($Comparisons | Where-Object { $_.status -in @("missing-baseline", "missing-current") }).Count -gt 0) {
        return "partial"
    }
    if (@($Comparisons | Where-Object { $_.status -eq "skipped" }).Count -eq $Comparisons.Count) {
        return "skipped"
    }
    return "passed"
}

function New-ComparisonReport {
    param([string]$BaselinePathValue, [string]$CurrentPathValue)

    $baseline = Read-JsonReport -Path $BaselinePathValue
    $current = Read-JsonReport -Path $CurrentPathValue
    $baselineMissing = $null -eq $baseline
    $currentMissing = $null -eq $current
    $baselineMap = New-ScenarioMap -Report $baseline
    $currentMap = New-ScenarioMap -Report $current
    $scenarioNames = [System.Collections.Generic.SortedSet[string]]::new()
    foreach ($name in $baselineMap.Keys) {
        [void]$scenarioNames.Add([string]$name)
    }
    foreach ($name in $currentMap.Keys) {
        [void]$scenarioNames.Add([string]$name)
    }

    $comparisons = @()
    foreach ($name in $scenarioNames) {
        $baselineScenario = if ($baselineMap.ContainsKey($name)) { $baselineMap[$name] } else { $null }
        $currentScenario = if ($currentMap.ContainsKey($name)) { $currentMap[$name] } else { $null }
        $comparisons += New-ScenarioComparison `
            -Name $name `
            -Baseline $baselineScenario `
            -Current $currentScenario `
            -RegressionThreshold $RegressionThresholdPercent `
            -ImprovementThreshold $ImprovementThresholdPercent
    }

    return [pscustomobject]@{
        schema_version = 1
        generated_at_unix_seconds = Get-UnixTimeSeconds
        status = Get-ReportStatus -Comparisons @($comparisons) -BaselineMissing $baselineMissing -CurrentMissing $currentMissing
        baseline_path = if ([string]::IsNullOrWhiteSpace($BaselinePathValue)) { "" } else { [IO.Path]::GetFullPath($BaselinePathValue) }
        current_path = if ([string]::IsNullOrWhiteSpace($CurrentPathValue)) { "" } else { [IO.Path]::GetFullPath($CurrentPathValue) }
        baseline_report_status = if ($baselineMissing) { "missing" } else { [string]$baseline.status }
        current_report_status = if ($currentMissing) { "missing" } else { [string]$current.status }
        regression_threshold_percent = $RegressionThresholdPercent
        improvement_threshold_percent = $ImprovementThresholdPercent
        status_counts = New-StatusCounts -Comparisons @($comparisons)
        scenarios = @($comparisons)
    }
}

function Get-OutputPaths {
    param([string]$OutputPathValue, [string]$OutputDirectoryValue)

    if ([string]::IsNullOrWhiteSpace($OutputPathValue)) {
        if ([string]::IsNullOrWhiteSpace($OutputDirectoryValue)) {
            if (-not [string]::IsNullOrWhiteSpace($CurrentPath)) {
                $OutputDirectoryValue = Split-Path -Parent ([IO.Path]::GetFullPath($CurrentPath))
            }
            else {
                $OutputDirectoryValue = Join-Path (Get-RepoRoot) "target\perf"
            }
        }
        $OutputPathValue = Join-Path $OutputDirectoryValue "benchmark-matrix-comparison.json"
    }

    $jsonPath = [IO.Path]::GetFullPath($OutputPathValue)
    $parent = Split-Path -Parent $jsonPath
    $stem = [IO.Path]::GetFileNameWithoutExtension($jsonPath)
    return [pscustomobject]@{
        json = $jsonPath
        csv = Join-Path $parent "$stem-scenarios.csv"
        markdown = Join-Path $parent "$stem-summary.md"
    }
}

function Convert-ComparisonForCsv {
    param([object]$Comparison)

    return [pscustomobject]@{
        scenario = $Comparison.scenario
        operation = $Comparison.operation
        backend = $Comparison.backend
        fixture = $Comparison.fixture
        status = $Comparison.status
        status_reason = $Comparison.status_reason
        baseline_status = $Comparison.baseline_status
        current_status = $Comparison.current_status
        baseline_mean_ns = $Comparison.baseline_mean_ns
        current_mean_ns = $Comparison.current_mean_ns
        delta_ns = $Comparison.delta_ns
        delta_percent = $Comparison.delta_percent
    }
}

function Convert-NsToMs {
    param([object]$Value)

    if ($null -eq $Value) {
        return ""
    }
    return "{0:n3}" -f ([double]$Value / 1000000.0)
}

function Convert-Percent {
    param([object]$Value)

    if ($null -eq $Value) {
        return ""
    }
    return "{0:n2}" -f ([double]$Value)
}

function New-MarkdownReport {
    param([object]$Report)

    $lines = @()
    $lines += "# Rebecca Benchmark Matrix Comparison"
    $lines += ""
    $lines += "- Status: $($Report.status)"
    $lines += "- Baseline: $($Report.baseline_path)"
    $lines += "- Current: $($Report.current_path)"
    $lines += "- Regression threshold: $($Report.regression_threshold_percent)%"
    $lines += "- Improvement threshold: $($Report.improvement_threshold_percent)%"
    $lines += ""
    $lines += "| Scenario | Status | Baseline ms | Current ms | Delta % |"
    $lines += "| --- | --- | ---: | ---: | ---: |"
    foreach ($comparison in @($Report.scenarios)) {
        $lines += "| $($comparison.scenario) | $($comparison.status) | $(Convert-NsToMs -Value $comparison.baseline_mean_ns) | $(Convert-NsToMs -Value $comparison.current_mean_ns) | $(Convert-Percent -Value $comparison.delta_percent) |"
    }
    if ($Report.scenarios.Count -eq 0) {
        $lines += "| _(none)_ | $($Report.status) |  |  |  |"
    }
    return ($lines -join [Environment]::NewLine)
}

function Write-ReportArtifacts {
    param([object]$Report, [object]$Paths)

    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $Paths.json) | Out-Null
    $Report | ConvertTo-Json -Depth 32 | Set-Content -LiteralPath $Paths.json -Encoding utf8
    $csvRows = @($Report.scenarios | ForEach-Object { Convert-ComparisonForCsv -Comparison $_ })
    if ($csvRows.Count -gt 0) {
        $csvRows | Export-Csv -LiteralPath $Paths.csv -NoTypeInformation -Encoding utf8
    }
    else {
        "scenario,operation,backend,fixture,status,status_reason,baseline_status,current_status,baseline_mean_ns,current_mean_ns,delta_ns,delta_percent" |
            Set-Content -LiteralPath $Paths.csv -Encoding utf8
    }
    New-MarkdownReport -Report $Report | Set-Content -LiteralPath $Paths.markdown -Encoding utf8
}

function Assert-Equal {
    param([object]$Actual, [object]$Expected, [string]$Message)

    if ($Actual -ne $Expected) {
        throw "$Message expected '$Expected' but got '$Actual'"
    }
}

function New-SelfTestScenario {
    param([string]$Name, [string]$Status, [object]$MeanNs)

    return [pscustomobject]@{
        scenario = $Name
        operation = "scan"
        backend = "portable-recursive"
        fixture = "self-test"
        status = $Status
        mean_ns = $MeanNs
        median_ns = $MeanNs
        evidence = [pscustomobject]@{
            timing = [pscustomobject]@{ measurement = "synthetic"; mean_ns = $MeanNs }
        }
    }
}

function Write-SelfTestReport {
    param([string]$Path, [object[]]$Scenarios)

    [pscustomobject]@{
        schema_version = 4
        generated_at_unix_seconds = 1
        status = "completed"
        package = "rebecca-core"
        bench = "perf_matrix"
        scenarios = @($Scenarios)
    } | ConvertTo-Json -Depth 32 | Set-Content -LiteralPath $Path -Encoding utf8
}

function Invoke-SelfTest {
    $temp = New-Item -ItemType Directory -Path (Join-Path ([IO.Path]::GetTempPath()) ("rebecca-perf-compare-" + [Guid]::NewGuid()))
    try {
        $baseline = Join-Path $temp.FullName "baseline.json"
        $current = Join-Path $temp.FullName "current.json"
        Write-SelfTestReport -Path $baseline -Scenarios @(
            New-SelfTestScenario -Name "pass_case" -Status "completed" -MeanNs 1000
            New-SelfTestScenario -Name "regression_case" -Status "completed" -MeanNs 1000
            New-SelfTestScenario -Name "improvement_case" -Status "completed" -MeanNs 1000
            New-SelfTestScenario -Name "skipped_case" -Status "missing-estimate" -MeanNs $null
            New-SelfTestScenario -Name "missing_current_case" -Status "completed" -MeanNs 1000
        )
        Write-SelfTestReport -Path $current -Scenarios @(
            New-SelfTestScenario -Name "pass_case" -Status "completed" -MeanNs 1050
            New-SelfTestScenario -Name "regression_case" -Status "completed" -MeanNs 1250
            New-SelfTestScenario -Name "improvement_case" -Status "completed" -MeanNs 800
            New-SelfTestScenario -Name "skipped_case" -Status "missing-estimate" -MeanNs $null
            New-SelfTestScenario -Name "missing_baseline_case" -Status "completed" -MeanNs 1000
        )

        $report = New-ComparisonReport -BaselinePathValue $baseline -CurrentPathValue $current
        Assert-Equal -Actual $report.status -Expected "regression" -Message "top-level status"
        $byName = @{}
        foreach ($scenario in @($report.scenarios)) {
            $byName[$scenario.scenario] = $scenario
        }
        Assert-Equal -Actual $byName["pass_case"].status -Expected "pass" -Message "pass scenario"
        Assert-Equal -Actual $byName["regression_case"].status -Expected "regression" -Message "regression scenario"
        Assert-Equal -Actual $byName["improvement_case"].status -Expected "improvement" -Message "improvement scenario"
        Assert-Equal -Actual $byName["skipped_case"].status -Expected "skipped" -Message "skipped scenario"
        Assert-Equal -Actual $byName["missing_baseline_case"].status -Expected "missing-baseline" -Message "missing baseline scenario"
        Assert-Equal -Actual $byName["missing_current_case"].status -Expected "missing-current" -Message "missing current scenario"

        $missingBaseline = New-ComparisonReport -BaselinePathValue (Join-Path $temp.FullName "missing-baseline.json") -CurrentPathValue $current
        Assert-Equal -Actual $missingBaseline.status -Expected "missing-baseline" -Message "missing baseline report"
        $missingCurrent = New-ComparisonReport -BaselinePathValue $baseline -CurrentPathValue (Join-Path $temp.FullName "missing-current.json")
        Assert-Equal -Actual $missingCurrent.status -Expected "missing-current" -Message "missing current report"
        Write-Host "Benchmark matrix comparator self-test passed."
    }
    finally {
        Remove-Item -LiteralPath $temp.FullName -Recurse -Force
    }
}

if ($SelfTest) {
    Invoke-SelfTest
    exit 0
}

if ([string]::IsNullOrWhiteSpace($BaselinePath) -or [string]::IsNullOrWhiteSpace($CurrentPath)) {
    throw "BaselinePath and CurrentPath are required unless -SelfTest is used."
}

$paths = Get-OutputPaths -OutputPathValue $OutputPath -OutputDirectoryValue $OutputDirectory
$report = New-ComparisonReport -BaselinePathValue $BaselinePath -CurrentPathValue $CurrentPath
$report | Add-Member -NotePropertyName "report_artifacts" -NotePropertyValue ([pscustomobject]@{
    json = $paths.json
    csv = $paths.csv
    markdown = $paths.markdown
})
Write-ReportArtifacts -Report $report -Paths $paths
Write-Host "Wrote benchmark matrix comparison JSON report to $($paths.json)"
Write-Host "Wrote benchmark matrix comparison CSV report to $($paths.csv)"
Write-Host "Wrote benchmark matrix comparison Markdown report to $($paths.markdown)"

if ($report.status -eq "regression") {
    exit 2
}

param(
    [string]$Root = "",
    [string[]]$Backend = @("portable-recursive", "windows-native", "windows-ntfs-mft-experimental"),
    [string[]]$GroupBy = @("extension", "depth", "age"),
    [int]$Repeat = 1,
    [int]$Top = 20,
    [int]$DiagnosticLimit = 0,
    [int]$TimeoutSeconds = 180,
    [string]$OutputDirectory = "",
    [switch]$CleanupAdvice,
    [switch]$AllowDriveRoot,
    [switch]$SelfTest
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Get-RepoRoot {
    return (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "..\..")).ProviderPath
}

function Get-TimestampId {
    return (Get-Date).ToUniversalTime().ToString("yyyyMMdd-HHmmss") + "-$PID"
}

function Normalize-ValidatedList {
    param(
        [string[]]$Values,
        [string[]]$Allowed,
        [string]$Name
    )

    $items = @()
    foreach ($value in $Values) {
        if ([string]::IsNullOrWhiteSpace($value)) {
            continue
        }
        $items += @($value -split "," | ForEach-Object { $_.Trim() } | Where-Object { $_ -ne "" })
    }
    if ($items.Count -eq 0) {
        throw "$Name must include at least one value."
    }
    foreach ($item in $items) {
        if ($Allowed -notcontains $item) {
            throw "Invalid $Name value '$item'. Allowed values: $($Allowed -join ', ')"
        }
    }
    return @($items)
}

function Resolve-ReportOutputDirectory {
    param([string]$Path)

    if (-not [string]::IsNullOrWhiteSpace($Path)) {
        if ([System.IO.Path]::IsPathRooted($Path)) {
            return $Path
        }
        return (Join-Path (Get-Location).ProviderPath $Path)
    }

    return Join-Path (Get-RepoRoot) (Join-Path "target\inspect-map-dogfood" (Get-TimestampId))
}

function Resolve-RequiredRoot {
    param(
        [string]$Path,
        [bool]$AllowDriveRootValue
    )

    if ([string]::IsNullOrWhiteSpace($Path)) {
        throw "Pass -Root explicitly. This script intentionally avoids implicit drive or repo scans."
    }

    $resolved = (Resolve-Path -LiteralPath $Path).ProviderPath
    $full = [System.IO.Path]::GetFullPath($resolved).TrimEnd('\', '/')
    $driveRoot = [System.IO.Path]::GetPathRoot($full)
    if (-not [string]::IsNullOrWhiteSpace($driveRoot)) {
        $driveRoot = $driveRoot.TrimEnd('\', '/')
    }
    if (-not $AllowDriveRootValue -and $full -eq $driveRoot) {
        throw "Refusing to dogfood a drive root without -AllowDriveRoot: $resolved"
    }

    return $resolved
}

function Get-ObjectProperty {
    param(
        [object]$Object,
        [string]$Name
    )

    if ($null -eq $Object) {
        return $null
    }
    $property = $Object.PSObject.Properties[$Name]
    if ($null -eq $property) {
        return $null
    }
    return $property.Value
}

function Find-JsonValues {
    param(
        [object]$Node,
        [string[]]$Names
    )

    $values = [System.Collections.Generic.List[string]]::new()
    Add-JsonValues -Node $Node -Names $Names -Values $values
    return @($values | Select-Object -Unique)
}

function Add-JsonValues {
    param(
        [object]$Node,
        [string[]]$Names,
        [System.Collections.Generic.List[string]]$Values
    )

    if ($null -eq $Node -or $Node -is [string] -or $Node -is [System.ValueType]) {
        return
    }
    if ($Node -is [System.Collections.IEnumerable]) {
        foreach ($item in $Node) {
            Add-JsonValues -Node $item -Names $Names -Values $Values
        }
        return
    }

    foreach ($property in $Node.PSObject.Properties) {
        if ($Names -contains $property.Name -and $null -ne $property.Value) {
            if ($property.Value -is [System.Collections.IEnumerable] -and $property.Value -isnot [string]) {
                foreach ($item in $property.Value) {
                    $Values.Add(($item | ConvertTo-Json -Compress -Depth 32))
                }
            }
            else {
                $Values.Add([string]$property.Value)
            }
        }
        Add-JsonValues -Node $property.Value -Names $Names -Values $Values
    }
}

function Join-ReportValues {
    param([object[]]$Values)

    if ($null -eq $Values) {
        return ""
    }
    return (@($Values) | Where-Object { $null -ne $_ -and [string]$_ -ne "" }) -join ";"
}

function Convert-InspectMapJson {
    param([string]$Raw)

    if ([string]::IsNullOrWhiteSpace($Raw)) {
        return [pscustomobject]@{
            parsed = $false
            parse_error = "empty stdout"
            json = $null
            data = $null
            actual_backends = @()
            backend_sources = @()
            fallback_reasons = @()
            caveats = @()
            totals = $null
            diagnostic_summary = $null
        }
    }

    try {
        $json = $Raw | ConvertFrom-Json -Depth 100
        $data = Get-ObjectProperty -Object $json -Name "data"
        return [pscustomobject]@{
            parsed = $true
            parse_error = $null
            json = $json
            data = $data
            actual_backends = @(Find-JsonValues -Node $json -Names @("estimate_backend"))
            backend_sources = @(Find-JsonValues -Node $json -Names @("estimate_backend_source"))
            fallback_reasons = @(Find-JsonValues -Node $json -Names @("estimate_fallback_reason"))
            caveats = @(Find-JsonValues -Node $json -Names @("estimate_caveats"))
            totals = Get-ObjectProperty -Object $data -Name "totals"
            diagnostic_summary = Get-ObjectProperty -Object $data -Name "diagnostic_summary"
        }
    }
    catch {
        return [pscustomobject]@{
            parsed = $false
            parse_error = $_.Exception.Message
            json = $null
            data = $null
            actual_backends = @()
            backend_sources = @()
            fallback_reasons = @()
            caveats = @()
            totals = $null
            diagnostic_summary = $null
        }
    }
}

function Get-MetricValue {
    param(
        [object]$Object,
        [string]$Name
    )

    $value = Get-ObjectProperty -Object $Object -Name $Name
    if ($null -eq $value) {
        return $null
    }
    return [int64]$value
}

function New-RunSummary {
    param(
        [string]$RunId,
        [int]$RepeatIndex,
        [string]$RootPath,
        [string]$RequestedBackend,
        [int]$ExitCode,
        [bool]$TimedOut,
        [int64]$DurationMs,
        [string]$StdoutPath,
        [string]$StderrPath,
        [object]$Probe
    )

    $status = if ($TimedOut) {
        "timeout"
    }
    elseif ($ExitCode -eq 0 -and $Probe.parsed) {
        "passed"
    }
    else {
        "failed"
    }
    $actualBackends = @($Probe.actual_backends)
    $actualBackend = if ($actualBackends.Count -eq 1) { $actualBackends[0] } else { "" }
    $totals = $Probe.totals
    $diag = $Probe.diagnostic_summary

    return [pscustomobject]@{
        run_id = $RunId
        repeat = $RepeatIndex
        root = $RootPath
        requested_backend = $RequestedBackend
        actual_backend = $actualBackend
        actual_backends = $actualBackends
        backend_sources = @($Probe.backend_sources)
        fallback_reasons = @($Probe.fallback_reasons)
        caveats = @($Probe.caveats)
        status = $status
        exit_code = $ExitCode
        timed_out = $TimedOut
        duration_ms = $DurationMs
        logical_bytes = Get-MetricValue -Object $totals -Name "logical_bytes"
        allocated_bytes = Get-MetricValue -Object $totals -Name "allocated_bytes"
        unique_logical_bytes = Get-MetricValue -Object $totals -Name "unique_logical_bytes"
        unique_allocated_bytes = Get-MetricValue -Object $totals -Name "unique_allocated_bytes"
        files = Get-MetricValue -Object $totals -Name "files"
        directories = Get-MetricValue -Object $totals -Name "directories"
        diagnostic_total = Get-MetricValue -Object $diag -Name "total"
        diagnostic_retained = Get-MetricValue -Object $diag -Name "retained"
        diagnostic_truncated = Get-MetricValue -Object $diag -Name "truncated"
        stdout_path = $StdoutPath
        stderr_path = $StderrPath
        parse_error = $Probe.parse_error
        comparison_status = ""
        logical_delta = $null
        file_delta = $null
        directory_delta = $null
    }
}

function Expand-InspectMapRows {
    param(
        [string]$RunId,
        [object]$Data
    )

    $rows = @()
    $entries = @(Get-ObjectProperty -Object $Data -Name "top_entries")
    for ($i = 0; $i -lt $entries.Count; $i++) {
        $entry = $entries[$i]
        $advice = Get-ObjectProperty -Object $entry -Name "cleanup_advice"
        $rows += [pscustomobject]@{
            run_id = $RunId
            row_kind = "entry"
            rank = $i + 1
            path = [string](Get-ObjectProperty -Object $entry -Name "path")
            entry_kind = [string](Get-ObjectProperty -Object $entry -Name "kind")
            group_kind = ""
            group_key = ""
            group_label = ""
            logical_bytes = Get-MetricValue -Object $entry -Name "logical_bytes"
            allocated_bytes = Get-MetricValue -Object $entry -Name "allocated_bytes"
            unique_logical_bytes = Get-MetricValue -Object $entry -Name "unique_logical_bytes"
            unique_allocated_bytes = Get-MetricValue -Object $entry -Name "unique_allocated_bytes"
            files = Get-MetricValue -Object $entry -Name "files"
            directories = Get-MetricValue -Object $entry -Name "directories"
            cleanup_status = [string](Get-ObjectProperty -Object $advice -Name "status")
            cleanup_source = [string](Get-ObjectProperty -Object $advice -Name "source")
            cleanup_rule_id = [string](Get-ObjectProperty -Object $advice -Name "rule_id")
        }
    }

    $groups = @(Get-ObjectProperty -Object $Data -Name "groups")
    for ($i = 0; $i -lt $groups.Count; $i++) {
        $group = $groups[$i]
        $metrics = Get-ObjectProperty -Object $group -Name "metrics"
        $rows += [pscustomobject]@{
            run_id = $RunId
            row_kind = "group"
            rank = $i + 1
            path = ""
            entry_kind = ""
            group_kind = [string](Get-ObjectProperty -Object $group -Name "kind")
            group_key = [string](Get-ObjectProperty -Object $group -Name "key")
            group_label = [string](Get-ObjectProperty -Object $group -Name "label")
            logical_bytes = Get-MetricValue -Object $metrics -Name "logical_bytes"
            allocated_bytes = Get-MetricValue -Object $metrics -Name "allocated_bytes"
            unique_logical_bytes = Get-MetricValue -Object $metrics -Name "unique_logical_bytes"
            unique_allocated_bytes = Get-MetricValue -Object $metrics -Name "unique_allocated_bytes"
            files = Get-MetricValue -Object $metrics -Name "files"
            directories = Get-MetricValue -Object $metrics -Name "directories"
            cleanup_status = ""
            cleanup_source = ""
            cleanup_rule_id = ""
        }
    }

    return $rows
}

function Add-RunComparisons {
    param([object[]]$Runs)

    foreach ($group in ($Runs | Group-Object repeat)) {
        $baseline = $group.Group |
            Where-Object { $_.requested_backend -eq "portable-recursive" -and $_.status -eq "passed" } |
            Select-Object -First 1
        foreach ($run in $group.Group) {
            if ($null -eq $baseline) {
                $run.comparison_status = "baseline-missing"
                continue
            }
            if ($run.status -ne "passed") {
                $run.comparison_status = "run-not-passed"
                continue
            }

            $run.logical_delta = $run.logical_bytes - $baseline.logical_bytes
            $run.file_delta = $run.files - $baseline.files
            $run.directory_delta = $run.directories - $baseline.directories
            if ($run.logical_delta -eq 0 -and $run.file_delta -eq 0 -and $run.directory_delta -eq 0) {
                $run.comparison_status = "matched"
            }
            else {
                $run.comparison_status = "mismatched"
            }
        }
    }

    return $Runs
}

function Convert-RunsForCsv {
    param([object[]]$Runs)

    return @($Runs | ForEach-Object {
        [pscustomobject]@{
            run_id = $_.run_id
            repeat = $_.repeat
            root = $_.root
            requested_backend = $_.requested_backend
            actual_backend = $_.actual_backend
            actual_backends = Join-ReportValues $_.actual_backends
            backend_sources = Join-ReportValues $_.backend_sources
            fallback_reasons = Join-ReportValues $_.fallback_reasons
            status = $_.status
            comparison_status = $_.comparison_status
            exit_code = $_.exit_code
            timed_out = $_.timed_out
            duration_ms = $_.duration_ms
            logical_bytes = $_.logical_bytes
            allocated_bytes = $_.allocated_bytes
            unique_logical_bytes = $_.unique_logical_bytes
            unique_allocated_bytes = $_.unique_allocated_bytes
            files = $_.files
            directories = $_.directories
            logical_delta = $_.logical_delta
            file_delta = $_.file_delta
            directory_delta = $_.directory_delta
            diagnostic_total = $_.diagnostic_total
            diagnostic_retained = $_.diagnostic_retained
            diagnostic_truncated = $_.diagnostic_truncated
            stdout_path = $_.stdout_path
            stderr_path = $_.stderr_path
            parse_error = $_.parse_error
        }
    })
}

function New-MarkdownSummary {
    param(
        [object]$Report,
        [object[]]$Runs
    )

    $lines = [System.Collections.Generic.List[string]]::new()
    $lines.Add("# Inspect Map Dogfood")
    $lines.Add("")
    $lines.Add("- Root: $($Report.root)")
    $lines.Add("- Generated: $($Report.generated_at_utc)")
    $lines.Add("- Commit: $($Report.git_commit)")
    $lines.Add("- Cleanup advice: $($Report.cleanup_advice)")
    $lines.Add("")
    $lines.Add("| Run | Backend | Actual | Status | Compare | ms | Logical | Files | Dirs |")
    $lines.Add("| --- | --- | --- | --- | --- | ---: | ---: | ---: | ---: |")
    foreach ($run in $Runs) {
        $lines.Add("| $($run.run_id) | $($run.requested_backend) | $($run.actual_backend) | $($run.status) | $($run.comparison_status) | $($run.duration_ms) | $($run.logical_bytes) | $($run.files) | $($run.directories) |")
    }
    $lines.Add("")
    return ($lines -join [Environment]::NewLine)
}

function Invoke-InspectMapRun {
    param(
        [string]$RepoRoot,
        [string]$RootPath,
        [string]$RequestedBackend,
        [int]$RepeatIndex,
        [int]$TopLimit,
        [string[]]$GroupKinds,
        [int]$DiagnosticLimitValue,
        [int]$TimeoutSecondsValue,
        [string]$RawDirectory,
        [bool]$CleanupAdviceValue,
        [hashtable]$EnvironmentOverrides
    )

    $runId = "r$RepeatIndex-$RequestedBackend"
    $stdoutPath = Join-Path $RawDirectory "$runId.stdout.json"
    $stderrPath = Join-Path $RawDirectory "$runId.stderr.txt"
    $arguments = [System.Collections.Generic.List[string]]::new()
    foreach ($arg in @("run", "-q", "-p", "rebecca", "--", "inspect", "map", "--format", "json", "--root", $RootPath, "--top", [string]$TopLimit, "--diagnostic-limit", [string]$DiagnosticLimitValue, "--scan-backend", $RequestedBackend)) {
        $arguments.Add($arg)
    }
    foreach ($group in $GroupKinds) {
        $arguments.Add("--group-by")
        $arguments.Add($group)
    }
    if ($CleanupAdviceValue) {
        $arguments.Add("--cleanup-advice")
    }

    $psi = [System.Diagnostics.ProcessStartInfo]::new()
    $psi.FileName = "cargo"
    foreach ($arg in $arguments) {
        [void]$psi.ArgumentList.Add($arg)
    }
    $psi.WorkingDirectory = $RepoRoot
    $psi.UseShellExecute = $false
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.CreateNoWindow = $true
    foreach ($key in $EnvironmentOverrides.Keys) {
        $psi.Environment[$key] = [string]$EnvironmentOverrides[$key]
    }

    $started = [DateTimeOffset]::UtcNow
    $process = [System.Diagnostics.Process]::Start($psi)
    $stdoutTask = $process.StandardOutput.ReadToEndAsync()
    $stderrTask = $process.StandardError.ReadToEndAsync()
    $timedOut = -not $process.WaitForExit($TimeoutSecondsValue * 1000)
    if ($timedOut) {
        try {
            $process.Kill($true)
        }
        catch {
        }
    }
    $stdout = $stdoutTask.GetAwaiter().GetResult()
    $stderr = $stderrTask.GetAwaiter().GetResult()
    [System.IO.File]::WriteAllText($stdoutPath, $stdout)
    [System.IO.File]::WriteAllText($stderrPath, $stderr)
    $durationMs = [int64]([DateTimeOffset]::UtcNow - $started).TotalMilliseconds
    $exitCode = if ($timedOut) { -1 } else { $process.ExitCode }
    $probe = Convert-InspectMapJson -Raw $stdout
    $summary = New-RunSummary -RunId $runId -RepeatIndex $RepeatIndex -RootPath $RootPath -RequestedBackend $RequestedBackend -ExitCode $exitCode -TimedOut $timedOut -DurationMs $durationMs -StdoutPath $stdoutPath -StderrPath $stderrPath -Probe $probe
    $rows = if ($probe.parsed) { @(Expand-InspectMapRows -RunId $runId -Data $probe.data) } else { @() }
    return [pscustomobject]@{
        summary = $summary
        rows = $rows
    }
}

function Invoke-SelfTest {
    $sample = @{
        api_version = "rebecca.cli.v1"
        kind = "success"
        command = "inspect map"
        payload_kind = "inspect-map"
        generated_at_unix_seconds = 1
        data = @{
            roots = @()
            totals = @{
                logical_bytes = 10
                allocated_bytes = $null
                unique_logical_bytes = $null
                unique_allocated_bytes = $null
                files = 2
                directories = 1
            }
            top_entries = @(
                @{
                    path = "C:\tmp\a"
                    root = "C:\tmp"
                    kind = "directory"
                    depth = 1
                    logical_bytes = 10
                    allocated_bytes = $null
                    unique_logical_bytes = $null
                    unique_allocated_bytes = $null
                    files = 2
                    directories = 1
                    estimate_source = "fresh-scan"
                    estimate_backend = "portable-recursive"
                    estimate_confidence = "exact"
                    cleanup_advice = @{
                        status = "cleanable"
                        source = "project-artifact"
                        rule_id = "windows.project-artifact-node-modules"
                        reason = "test"
                    }
                }
            )
            groups = @(
                @{
                    kind = "extension"
                    key = ".bin"
                    label = ".bin"
                    metrics = @{
                        logical_bytes = 10
                        allocated_bytes = $null
                        unique_logical_bytes = $null
                        unique_allocated_bytes = $null
                        files = 2
                        directories = 0
                    }
                }
            )
            diagnostic_summary = @{
                total = 0
                retained = 0
                truncated = 0
                by_kind = @()
            }
            diagnostics = @()
        }
    } | ConvertTo-Json -Depth 64

    $probe = Convert-InspectMapJson -Raw $sample
    if (-not $probe.parsed) { throw "self-test parse failed" }
    if ([int64]$probe.totals.logical_bytes -ne 10) { throw "self-test totals failed" }
    $rows = @(Expand-InspectMapRows -RunId "r1" -Data $probe.data)
    if ($rows.Count -ne 2) { throw "self-test row expansion failed" }
    if ($rows[0].cleanup_status -ne "cleanable") { throw "self-test cleanup status failed" }

    $portable = New-RunSummary -RunId "r1-portable" -RepeatIndex 1 -RootPath "C:\tmp" -RequestedBackend "portable-recursive" -ExitCode 0 -TimedOut $false -DurationMs 10 -StdoutPath "out" -StderrPath "err" -Probe $probe
    $native = New-RunSummary -RunId "r1-native" -RepeatIndex 1 -RootPath "C:\tmp" -RequestedBackend "windows-native" -ExitCode 0 -TimedOut $false -DurationMs 5 -StdoutPath "out" -StderrPath "err" -Probe $probe
    $runs = @(Add-RunComparisons -Runs @($portable, $native))
    if (($runs | Where-Object { $_.requested_backend -eq "windows-native" }).comparison_status -ne "matched") {
        throw "self-test comparison failed"
    }

    $temp = Join-Path ([System.IO.Path]::GetTempPath()) ("rebecca-inspect-map-dogfood-selftest-" + [Guid]::NewGuid().ToString("N"))
    New-Item -ItemType Directory -Force -Path $temp | Out-Null
    try {
        Convert-RunsForCsv -Runs $runs | Export-Csv -LiteralPath (Join-Path $temp "runs.csv") -NoTypeInformation
        $rows | Export-Csv -LiteralPath (Join-Path $temp "rows.csv") -NoTypeInformation
        if (-not (Test-Path -LiteralPath (Join-Path $temp "runs.csv"))) { throw "self-test csv failed" }
        if (-not (Test-Path -LiteralPath (Join-Path $temp "rows.csv"))) { throw "self-test rows csv failed" }
    }
    finally {
        Remove-Item -LiteralPath $temp -Recurse -Force
    }

    Write-Host "Self-test passed."
}

if ($SelfTest) {
    Invoke-SelfTest
    exit 0
}

$Backend = Normalize-ValidatedList -Values $Backend -Allowed @("portable-recursive", "windows-native", "windows-ntfs-mft-experimental") -Name "Backend"
$GroupBy = Normalize-ValidatedList -Values $GroupBy -Allowed @("extension", "depth", "age") -Name "GroupBy"

$repoRoot = Get-RepoRoot
$resolvedRoot = Resolve-RequiredRoot -Path $Root -AllowDriveRootValue ([bool]$AllowDriveRoot)
$outputRoot = Resolve-ReportOutputDirectory -Path $OutputDirectory
$rawDirectory = Join-Path $outputRoot "raw"
New-Item -ItemType Directory -Force -Path $rawDirectory | Out-Null

$runtimeRoot = Join-Path $outputRoot "runtime"
$environmentOverrides = @{
    REBECCA_CONFIG_DIR = Join-Path $runtimeRoot "config"
    REBECCA_STATE_DIR = Join-Path $runtimeRoot "state"
    REBECCA_CACHE_DIR = Join-Path $runtimeRoot "cache"
    REBECCA_HISTORY_FILE = Join-Path (Join-Path $runtimeRoot "state") "history.jsonl"
}
foreach ($value in $environmentOverrides.Values) {
    $parent = if ([System.IO.Path]::HasExtension($value)) { Split-Path -Parent $value } else { $value }
    if (-not [string]::IsNullOrWhiteSpace($parent)) {
        New-Item -ItemType Directory -Force -Path $parent | Out-Null
    }
}

$runs = @()
$rows = @()
for ($repeatIndex = 1; $repeatIndex -le $Repeat; $repeatIndex++) {
    foreach ($requestedBackend in $Backend) {
        $result = Invoke-InspectMapRun -RepoRoot $repoRoot -RootPath $resolvedRoot -RequestedBackend $requestedBackend -RepeatIndex $repeatIndex -TopLimit $Top -GroupKinds $GroupBy -DiagnosticLimitValue $DiagnosticLimit -TimeoutSecondsValue $TimeoutSeconds -RawDirectory $rawDirectory -CleanupAdviceValue ([bool]$CleanupAdvice) -EnvironmentOverrides $environmentOverrides
        $runs += $result.summary
        $rows += $result.rows
    }
}

$runs = @(Add-RunComparisons -Runs $runs)
$report = [pscustomobject]@{
    schema_version = 1
    generated_at_utc = [DateTimeOffset]::UtcNow.ToString("O")
    git_commit = (& git -C $repoRoot rev-parse --short HEAD 2>$null)
    repo_root = $repoRoot
    root = $resolvedRoot
    cleanup_advice = [bool]$CleanupAdvice
    backends = @($Backend)
    repeat = $Repeat
    top = $Top
    group_by = @($GroupBy)
    diagnostic_limit = $DiagnosticLimit
    timeout_seconds = $TimeoutSeconds
    output_directory = $outputRoot
    runs = @($runs)
}

$reportPath = Join-Path $outputRoot "inspect-map-report.json"
$runsCsvPath = Join-Path $outputRoot "inspect-map-runs.csv"
$rowsCsvPath = Join-Path $outputRoot "inspect-map-rows.csv"
$summaryPath = Join-Path $outputRoot "inspect-map-summary.md"

$report | ConvertTo-Json -Depth 64 | Set-Content -LiteralPath $reportPath -Encoding utf8
Convert-RunsForCsv -Runs $runs | Export-Csv -LiteralPath $runsCsvPath -NoTypeInformation
$rows | Export-Csv -LiteralPath $rowsCsvPath -NoTypeInformation
New-MarkdownSummary -Report $report -Runs $runs | Set-Content -LiteralPath $summaryPath -Encoding utf8

Write-Host "Inspect-map dogfood report written to $outputRoot"
Write-Host "  JSON: $reportPath"
Write-Host "  Runs: $runsCsvPath"
Write-Host "  Rows: $rowsCsvPath"
Write-Host "  Summary: $summaryPath"

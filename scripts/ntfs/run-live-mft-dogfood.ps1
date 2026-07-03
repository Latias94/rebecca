param(
    [string[]]$Root = @(),
    [ValidateSet("inspect-space", "inspect-map", "clean-dry-run", "both")]
    [string]$Mode = "inspect-space",
    [ValidateSet("portable-recursive", "windows-native", "windows-ntfs-mft-experimental")]
    [string[]]$Backend = @("portable-recursive", "windows-native", "windows-ntfs-mft-experimental"),
    [int]$Top = 10,
    [int]$TimeoutSeconds = 180,
    [string]$OutputPath = "",
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

function Get-GitCommit {
    param([string]$RepoRoot)

    $commit = (& git -C $RepoRoot rev-parse --short HEAD 2>$null)
    if ($LASTEXITCODE -ne 0) {
        return ""
    }
    return [string]$commit
}

function Test-IsElevated {
    if (-not $IsWindows) {
        return $false
    }

    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = [Security.Principal.WindowsPrincipal]::new($identity)
    return $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

function Get-HostSummary {
    return [pscustomobject]@{
        os = [System.Runtime.InteropServices.RuntimeInformation]::OSDescription
        architecture = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
        computer_name = $env:COMPUTERNAME
        user_domain = $env:USERDOMAIN
        is_windows = $IsWindows
        is_elevated = Test-IsElevated
        pwsh_version = $PSVersionTable.PSVersion.ToString()
    }
}

function Get-RootSummary {
    param([string]$Path)

    $resolved = (Resolve-Path -LiteralPath $Path).ProviderPath
    $drive = [System.IO.Path]::GetPathRoot($resolved)
    $volume = $null
    if ($IsWindows -and -not [string]::IsNullOrWhiteSpace($drive)) {
        $driveLetter = $drive.Substring(0, 1)
        try {
            $volumeInfo = Get-Volume -DriveLetter $driveLetter -ErrorAction Stop
            $volume = [pscustomobject]@{
                drive_letter = $driveLetter
                file_system = $volumeInfo.FileSystem
                drive_type = $volumeInfo.DriveType
                size_bytes = [int64]$volumeInfo.Size
                size_remaining_bytes = [int64]$volumeInfo.SizeRemaining
            }
        }
        catch {
            $volume = [pscustomobject]@{
                drive_letter = $driveLetter
                probe_error = $_.Exception.Message
            }
        }
    }

    return [pscustomobject]@{
        requested_path = $Path
        resolved_path = $resolved
        drive_root = $drive
        volume = $volume
    }
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

function Add-JsonValues {
    param(
        [object]$Node,
        [string[]]$Names,
        [System.Collections.Generic.List[string]]$Values
    )

    if ($null -eq $Node) {
        return
    }
    if ($Node -is [string]) {
        return
    }
    if ($Node -is [System.ValueType]) {
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
            $Values.Add([string]$property.Value)
        }
        Add-JsonValues -Node $property.Value -Names $Names -Values $Values
    }
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

function Get-NestedProperty {
    param(
        [object]$Object,
        [string[]]$Path
    )

    $current = $Object
    foreach ($segment in $Path) {
        $current = Get-ObjectProperty -Object $current -Name $segment
        if ($null -eq $current) {
            return $null
        }
    }
    return $current
}

function Get-JsonMetric {
    param([object]$Json)

    $data = Get-ObjectProperty -Object $Json -Name "data"
    $estimatedBytes = Get-NestedProperty -Object $data -Path @("totals", "estimated_bytes")
    if ($null -eq $estimatedBytes) {
        $estimatedBytes = Get-NestedProperty -Object $data -Path @("totals", "logical_bytes")
    }
    if ($null -eq $estimatedBytes) {
        $estimatedBytes = Get-NestedProperty -Object $data -Path @("summary", "estimated_bytes")
    }
    if ($null -eq $estimatedBytes) {
        $estimatedBytes = Get-NestedProperty -Object $data -Path @("summary", "pending_reclaim_bytes")
    }

    $files = Get-NestedProperty -Object $data -Path @("totals", "files")
    $directories = Get-NestedProperty -Object $data -Path @("totals", "directories")
    $targets = Get-NestedProperty -Object $data -Path @("summary", "total_targets")

    return [pscustomobject]@{
        estimated_bytes = if ($null -eq $estimatedBytes) { $null } else { [int64]$estimatedBytes }
        files = if ($null -eq $files) { $null } else { [int64]$files }
        directories = if ($null -eq $directories) { $null } else { [int64]$directories }
        total_targets = if ($null -eq $targets) { $null } else { [int64]$targets }
    }
}

function Get-JsonDiagnosticSummary {
    param([object]$Json)

    $summary = Get-NestedProperty -Object (Get-ObjectProperty -Object $Json -Name "data") -Path @("diagnostic_summary")
    if ($null -eq $summary) {
        return $null
    }

    $total = Get-ObjectProperty -Object $summary -Name "total"
    $retained = Get-ObjectProperty -Object $summary -Name "retained"
    $truncated = Get-ObjectProperty -Object $summary -Name "truncated"

    return [pscustomobject]@{
        total = if ($null -eq $total) { $null } else { [int64]$total }
        retained = if ($null -eq $retained) { $null } else { [int64]$retained }
        truncated = if ($null -eq $truncated) { $null } else { [int64]$truncated }
    }
}

function Convert-JsonProbe {
    param([string]$Raw)

    if ([string]::IsNullOrWhiteSpace($Raw)) {
        return [pscustomobject]@{
            parsed = $false
            parse_error = "empty stdout"
            actual_backends = @()
            backend_sources = @()
            fallback_reasons = @()
            caveats = @()
            diagnostic_summary = $null
            metric = $null
        }
    }

    try {
        $json = $Raw | ConvertFrom-Json
        $estimateBackends = @(Find-JsonValues -Node $json -Names @("estimate_backend"))
        $backends = if ($estimateBackends.Count -gt 0) {
            @($estimateBackends)
        }
        else {
            @(Find-JsonValues -Node $json -Names @("backend"))
        }

        return [pscustomobject]@{
            parsed = $true
            parse_error = $null
            actual_backends = @($backends)
            backend_sources = @(Find-JsonValues -Node $json -Names @("estimate_backend_source", "backend_source"))
            fallback_reasons = @(Find-JsonValues -Node $json -Names @("estimate_fallback_reason", "fallback_reason"))
            caveats = @(Find-JsonValues -Node $json -Names @("estimate_caveats", "caveats", "diagnostics"))
            diagnostic_summary = Get-JsonDiagnosticSummary -Json $json
            metric = Get-JsonMetric -Json $json
        }
    }
    catch {
        return [pscustomobject]@{
            parsed = $false
            parse_error = $_.Exception.Message
            actual_backends = @()
            backend_sources = @()
            fallback_reasons = @()
            caveats = @()
            diagnostic_summary = $null
            metric = $null
        }
    }
}

function Read-TextFile {
    param([string]$Path)

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        return ""
    }
    $content = Get-Content -LiteralPath $Path -Raw
    if ($null -eq $content) {
        return ""
    }
    return [string]$content
}

function Resolve-OutputPath {
    param([string]$Path)

    if ([System.IO.Path]::IsPathRooted($Path)) {
        return $Path
    }
    return (Join-Path (Get-Location).ProviderPath $Path)
}

function Ensure-OutputParent {
    param([string]$Path)

    $parent = Split-Path -Parent $Path
    if ([string]::IsNullOrWhiteSpace($parent)) {
        $parent = (Get-Location).ProviderPath
    }
    New-Item -ItemType Directory -Force -Path $parent | Out-Null
}

function Get-RebeccaCommand {
    param(
        [string]$ModeName,
        [string]$RequestedBackend,
        [string]$RootPath,
        [int]$TopLimit
    )

    if ($ModeName -eq "inspect-space") {
        return @(
            "cargo", "run", "-q", "-p", "rebecca", "--",
            "inspect", "space",
            "--format", "json",
            "--root", $RootPath,
            "--top", ([string]$TopLimit),
            "--scan-backend", $RequestedBackend
        )
    }
    if ($ModeName -eq "inspect-map") {
        return @(
            "cargo", "run", "-q", "-p", "rebecca", "--",
            "inspect", "map",
            "--format", "json",
            "--root", $RootPath,
            "--top", ([string]$TopLimit),
            "--scan-backend", $RequestedBackend
        )
    }

    return @(
        "cargo", "run", "-q", "-p", "rebecca", "--",
        "clean",
        "--dry-run",
        "--no-scan-cache",
        "--format", "json",
        "--category", "system",
        "--scan-backend", $RequestedBackend
    )
}

function Invoke-DogfoodCommand {
    param(
        [string]$ModeName,
        [string]$RequestedBackend,
        [string]$RootPath,
        [int]$TopLimit,
        [string]$OutputDirectory,
        [string]$RunId,
        [int]$CommandTimeoutSeconds,
        [string]$WorkingDirectory
    )

    $command = Get-RebeccaCommand -ModeName $ModeName -RequestedBackend $RequestedBackend -RootPath $RootPath -TopLimit $TopLimit
    $stdoutPath = Join-Path $OutputDirectory "$RunId.stdout.json"
    $stderrPath = Join-Path $OutputDirectory "$RunId.stderr.txt"
    $started = [DateTimeOffset]::UtcNow

    $process = Start-Process `
        -FilePath $command[0] `
        -ArgumentList @($command | Select-Object -Skip 1) `
        -WorkingDirectory $WorkingDirectory `
        -RedirectStandardOutput $stdoutPath `
        -RedirectStandardError $stderrPath `
        -NoNewWindow `
        -PassThru
    $timedOut = $false
    $exitCode = $null
    if (-not $process.WaitForExit($CommandTimeoutSeconds * 1000)) {
        $timedOut = $true
        try {
            $process.Kill($true)
        }
        catch {
            $stderrPathForTimeout = Join-Path $OutputDirectory "$RunId.timeout.txt"
            $_.Exception.Message | Set-Content -LiteralPath $stderrPathForTimeout -Encoding utf8
        }
    }
    else {
        $exitCode = $process.ExitCode
    }
    $durationMs = [int64]([DateTimeOffset]::UtcNow - $started).TotalMilliseconds
    $stdout = Read-TextFile -Path $stdoutPath
    $stderr = Read-TextFile -Path $stderrPath
    $probe = Convert-JsonProbe -Raw $stdout
    $status = if ($timedOut) {
        "timeout"
    }
    elseif ($exitCode -eq 0 -and $probe.parsed) {
        "passed"
    }
    else {
        "failed"
    }
    $actualBackends = @($probe.actual_backends)
    $actualBackend = if ($actualBackends.Count -eq 1) { $actualBackends[0] } else { $null }

    return [pscustomobject]@{
        run_id = $RunId
        root = $RootPath
        mode = $ModeName
        requested_backend = $RequestedBackend
        actual_backend = $actualBackend
        actual_backends = @($actualBackends)
        backend_sources = @($probe.backend_sources)
        fallback_reasons = @($probe.fallback_reasons)
        caveats = @($probe.caveats)
        diagnostic_summary = $probe.diagnostic_summary
        status = $status
        exit_code = $exitCode
        timed_out = $timedOut
        timeout_seconds = $CommandTimeoutSeconds
        duration_ms = $durationMs
        command = @($command)
        raw_output_path = $stdoutPath
        raw_stderr_path = $stderrPath
        stderr_preview = if ($stderr.Length -gt 500) { $stderr.Substring(0, 500) } else { $stderr }
        json_parse_error = $probe.parse_error
        metric = $probe.metric
        safety = [pscustomobject]@{
            dry_run_only = $ModeName -ne "clean-dry-run" -or ($command -contains "--dry-run")
            scan_cache_disabled = $ModeName -ne "clean-dry-run" -or ($command -contains "--no-scan-cache")
        }
        comparison = $null
    }
}

function Add-Comparisons {
    param([object[]]$Runs)

    $summaries = @()
    foreach ($modeGroup in ($Runs | Group-Object mode)) {
        foreach ($rootGroup in ($modeGroup.Group | Group-Object root)) {
            $portable = $rootGroup.Group |
                Where-Object { $_.requested_backend -eq "portable-recursive" -and $null -ne $_.metric } |
                Select-Object -First 1
            if ($null -eq $portable -or $null -eq $portable.metric.estimated_bytes) {
                $summaries += [pscustomobject]@{
                    mode = $modeGroup.Name
                    root = $rootGroup.Name
                    baseline_backend = "portable-recursive"
                    baseline_run_id = $null
                    status = "baseline-missing"
                    fastest_run_id = $null
                    fastest_requested_backend = $null
                    fastest_actual_backend = $null
                    fastest_backend_sources = @()
                    fastest_duration_ms = $null
                    runs = @()
                }
                continue
            }

            $runComparisons = @()
            foreach ($run in $rootGroup.Group) {
                if ($null -eq $run.metric -or $null -eq $run.metric.estimated_bytes) {
                    $run.comparison = [pscustomobject]@{
                        baseline_backend = "portable-recursive"
                        baseline_run_id = $portable.run_id
                        status = "metric-missing"
                        estimated_bytes_delta = $null
                        estimated_bytes_delta_percent = $null
                        files_delta = $null
                        directories_delta = $null
                        duration_ms_delta = $null
                        duration_ratio = $null
                        matches = [pscustomobject]@{
                            estimated_bytes = $false
                            files = $false
                            directories = $false
                        }
                    }
                    $runComparisons += [pscustomobject]@{
                        run_id = $run.run_id
                        requested_backend = $run.requested_backend
                        actual_backend = $run.actual_backend
                        status = $run.comparison.status
                        duration_ms = $run.duration_ms
                        estimated_bytes_delta = $null
                        files_delta = $null
                        directories_delta = $null
                    }
                    continue
                }
                $estimatedBytesDelta = [int64]$run.metric.estimated_bytes - [int64]$portable.metric.estimated_bytes
                $filesDelta = if ($null -eq $run.metric.files -or $null -eq $portable.metric.files) {
                    $null
                }
                else {
                    [int64]$run.metric.files - [int64]$portable.metric.files
                }
                $directoriesDelta = if ($null -eq $run.metric.directories -or $null -eq $portable.metric.directories) {
                    $null
                }
                else {
                    [int64]$run.metric.directories - [int64]$portable.metric.directories
                }
                $durationDelta = if ($null -eq $run.duration_ms -or $null -eq $portable.duration_ms) {
                    $null
                }
                else {
                    [int64]$run.duration_ms - [int64]$portable.duration_ms
                }
                $durationRatio = if ($null -eq $run.duration_ms -or $null -eq $portable.duration_ms -or [int64]$portable.duration_ms -eq 0) {
                    $null
                }
                else {
                    [math]::Round(([double]$run.duration_ms / [double]$portable.duration_ms), 4)
                }
                $estimatedBytesDeltaPercent = if ([int64]$portable.metric.estimated_bytes -eq 0) {
                    if ($estimatedBytesDelta -eq 0) { 0.0 } else { $null }
                }
                else {
                    [math]::Round((100.0 * [double]$estimatedBytesDelta / [double]$portable.metric.estimated_bytes), 4)
                }
                $bytesMatch = $estimatedBytesDelta -eq 0
                $filesMatch = $null -ne $filesDelta -and $filesDelta -eq 0
                $directoriesMatch = $null -ne $directoriesDelta -and $directoriesDelta -eq 0
                $comparisonStatus = if ($run.status -ne "passed") {
                    "run-not-passed"
                }
                elseif ($bytesMatch -and $filesMatch -and $directoriesMatch) {
                    "matched"
                }
                else {
                    "mismatched"
                }
                $run.comparison = [pscustomobject]@{
                    baseline_backend = "portable-recursive"
                    baseline_run_id = $portable.run_id
                    status = $comparisonStatus
                    estimated_bytes_delta = $estimatedBytesDelta
                    estimated_bytes_delta_percent = $estimatedBytesDeltaPercent
                    files_delta = $filesDelta
                    directories_delta = $directoriesDelta
                    duration_ms_delta = $durationDelta
                    duration_ratio = $durationRatio
                    matches = [pscustomobject]@{
                        estimated_bytes = $bytesMatch
                        files = $filesMatch
                        directories = $directoriesMatch
                    }
                }
                $runComparisons += [pscustomobject]@{
                    run_id = $run.run_id
                    requested_backend = $run.requested_backend
                    actual_backend = $run.actual_backend
                    status = $run.comparison.status
                    duration_ms = $run.duration_ms
                    duration_ratio = $durationRatio
                    estimated_bytes_delta = $estimatedBytesDelta
                    estimated_bytes_delta_percent = $estimatedBytesDeltaPercent
                    files_delta = $filesDelta
                    directories_delta = $directoriesDelta
                }
            }

            $passedRuns = @($rootGroup.Group | Where-Object { $_.status -eq "passed" -and $null -ne $_.duration_ms } | Sort-Object duration_ms)
            $fastest = if ($passedRuns.Count -gt 0) { $passedRuns[0] } else { $null }
            $mismatchedComparisons = @($runComparisons | Where-Object { $_.status -eq "mismatched" })
            $incompleteComparisons = @($runComparisons | Where-Object { $_.status -ne "matched" })
            $summaryStatus = if ($mismatchedComparisons.Count -gt 0) {
                "mismatched"
            }
            elseif ($incompleteComparisons.Count -gt 0) {
                "incomplete"
            }
            else {
                "matched"
            }
            $fastestRunId = $null
            $fastestRequestedBackend = $null
            $fastestActualBackend = $null
            $fastestBackendSources = @()
            $fastestDurationMs = $null
            if ($null -ne $fastest) {
                $fastestRunId = $fastest.run_id
                $fastestRequestedBackend = $fastest.requested_backend
                $fastestActualBackend = $fastest.actual_backend
                $fastestBackendSources = @($fastest.backend_sources)
                $fastestDurationMs = $fastest.duration_ms
            }
            $summaries += [pscustomobject]@{
                mode = $modeGroup.Name
                root = $rootGroup.Name
                baseline_backend = "portable-recursive"
                baseline_run_id = $portable.run_id
                baseline_metric = $portable.metric
                status = $summaryStatus
                fastest_run_id = $fastestRunId
                fastest_requested_backend = $fastestRequestedBackend
                fastest_actual_backend = $fastestActualBackend
                fastest_backend_sources = @($fastestBackendSources)
                fastest_duration_ms = $fastestDurationMs
                runs = @($runComparisons)
            }
        }
    }
    return @($summaries)
}

function Test-Self {
    param(
        [string]$RepoRoot,
        [string]$OutputPath
    )

    $fake = @'
{
  "api_version": "rebecca.cli.v1",
  "kind": "success",
  "command": "inspect space",
  "payload_kind": "inspect-space",
  "generated_at_unix_seconds": 1,
  "data": {
    "totals": {
      "estimated_bytes": 123,
      "files": 2,
      "directories": 1
    },
    "top_entries": [
      {
        "estimate_backend": "windows-ntfs-mft-experimental",
        "estimate_backend_source": "windows-ntfs-mft-experimental-targeted-fsctl"
      }
    ]
  }
}
'@
    $probe = Convert-JsonProbe -Raw $fake
    if (-not $probe.parsed) {
        throw "SelfTest failed: fake JSON did not parse: $($probe.parse_error)"
    }
    if ($probe.actual_backends -notcontains "windows-ntfs-mft-experimental") {
        throw "SelfTest failed: backend extraction did not find windows-ntfs-mft-experimental."
    }
    if ($probe.metric.estimated_bytes -ne 123) {
        throw "SelfTest failed: metric extraction returned $($probe.metric.estimated_bytes)."
    }
    if ($null -ne $probe.diagnostic_summary) {
        throw "SelfTest failed: fake inspect-space JSON should not have a diagnostic summary."
    }

    $fakeMap = @'
{
  "api_version": "rebecca.cli.v1",
  "kind": "success",
  "command": "inspect map",
  "payload_kind": "inspect-map",
  "generated_at_unix_seconds": 1,
  "data": {
    "diagnostic_summary": {
      "total": 1,
      "retained": 1,
      "truncated": 0,
      "by_kind": [
        {
          "kind": "fallback",
          "count": 1
        }
      ]
    },
    "totals": {
      "logical_bytes": 456,
      "allocated_bytes": null,
      "files": 3,
      "directories": 2
    },
    "top_entries": [
      {
        "estimate_backend": "windows-ntfs-mft-experimental",
        "estimate_backend_source": "windows-ntfs-mft-experimental-targeted-fsctl"
      }
    ]
  }
}
'@
    $mapProbe = Convert-JsonProbe -Raw $fakeMap
    if (-not $mapProbe.parsed) {
        throw "SelfTest failed: fake inspect-map JSON did not parse: $($mapProbe.parse_error)"
    }
    if ($mapProbe.metric.estimated_bytes -ne 456) {
        throw "SelfTest failed: inspect-map logical metric extraction returned $($mapProbe.metric.estimated_bytes)."
    }
    if ($mapProbe.backend_sources -notcontains "windows-ntfs-mft-experimental-targeted-fsctl") {
        throw "SelfTest failed: inspect-map backend source extraction did not find targeted-fsctl."
    }
    if ($mapProbe.diagnostic_summary.total -ne 1) {
        throw "SelfTest failed: inspect-map diagnostic summary extraction returned $($mapProbe.diagnostic_summary.total)."
    }

    $selfTestRuns = @(
        [pscustomobject]@{
            run_id = "self-test-portable"
            root = $RepoRoot
            mode = "inspect-map"
            requested_backend = "portable-recursive"
            actual_backend = "portable-recursive"
            actual_backends = @("portable-recursive")
            backend_sources = @()
            fallback_reasons = @()
            caveats = @()
            diagnostic_summary = $null
            status = "passed"
            exit_code = 0
            timed_out = $false
            timeout_seconds = 0
            duration_ms = 50
            command = @()
            raw_output_path = $null
            raw_stderr_path = $null
            stderr_preview = ""
            json_parse_error = $null
            metric = [pscustomobject]@{
                estimated_bytes = 456
                files = 3
                directories = 2
                total_targets = $null
            }
            safety = [pscustomobject]@{
                dry_run_only = $true
                scan_cache_disabled = $true
            }
            comparison = $null
        },
        [pscustomobject]@{
            run_id = "self-test-ntfs"
            root = $RepoRoot
            mode = "inspect-map"
            requested_backend = "windows-ntfs-mft-experimental"
            actual_backend = "windows-ntfs-mft-experimental"
            actual_backends = @($mapProbe.actual_backends)
            backend_sources = @($mapProbe.backend_sources)
            fallback_reasons = @($mapProbe.fallback_reasons)
            caveats = @($mapProbe.caveats)
            diagnostic_summary = $mapProbe.diagnostic_summary
            status = "passed"
            exit_code = 0
            timed_out = $false
            timeout_seconds = 0
            duration_ms = 100
            command = @()
            raw_output_path = $null
            raw_stderr_path = $null
            stderr_preview = ""
            json_parse_error = $null
            metric = $mapProbe.metric
            safety = [pscustomobject]@{
                dry_run_only = $true
                scan_cache_disabled = $true
            }
            comparison = $null
        }
    )
    $comparisonSummaries = Add-Comparisons -Runs $selfTestRuns
    if ($comparisonSummaries.Count -ne 1 -or $comparisonSummaries[0].status -ne "matched") {
        throw "SelfTest failed: comparison summary did not report a matched backend comparison."
    }
    if ($comparisonSummaries[0].fastest_run_id -ne "self-test-portable") {
        throw "SelfTest failed: fastest run id was $($comparisonSummaries[0].fastest_run_id)."
    }
    if ($comparisonSummaries[0].fastest_requested_backend -ne "portable-recursive") {
        throw "SelfTest failed: fastest requested backend was $($comparisonSummaries[0].fastest_requested_backend)."
    }
    if ($comparisonSummaries[0].fastest_actual_backend -ne "portable-recursive") {
        throw "SelfTest failed: fastest actual backend was $($comparisonSummaries[0].fastest_actual_backend)."
    }
    if ($null -eq $comparisonSummaries[0].fastest_backend_sources) {
        throw "SelfTest failed: fastest backend sources should be an empty array, not null."
    }
    if (@($comparisonSummaries[0].fastest_backend_sources).Count -ne 0) {
        throw "SelfTest failed: fastest backend sources should be empty for portable-recursive."
    }
    if ($selfTestRuns[1].comparison.duration_ratio -ne 2.0) {
        throw "SelfTest failed: comparison duration ratio was $($selfTestRuns[1].comparison.duration_ratio)."
    }

    $report = [pscustomobject]@{
        schema_version = 1
        generated_at_unix_seconds = Get-UnixTimeSeconds
        repo_root = $RepoRoot
        git_commit = Get-GitCommit -RepoRoot $RepoRoot
        script = $PSCommandPath
        host = Get-HostSummary
        output_path = $OutputPath
        self_test = $true
        comparisons = @($comparisonSummaries)
        runs = @($selfTestRuns)
    }
    Ensure-OutputParent -Path $OutputPath
    $report | ConvertTo-Json -Depth 16 | Set-Content -LiteralPath $OutputPath -Encoding utf8
    Write-Host "Wrote NTFS dogfood self-test report to $OutputPath"
}

$repoRoot = Get-RepoRoot
$runStamp = [DateTimeOffset]::UtcNow.ToString("yyyyMMdd-HHmmss")
$runRoot = Join-Path $repoRoot "target\ntfs-dogfood\$runStamp-$PID"
$rawRoot = Join-Path $runRoot "raw"
$configRoot = Join-Path $runRoot "config"
$stateRoot = Join-Path $runRoot "state"
$cacheRoot = Join-Path $runRoot "cache"
$historyFile = Join-Path $stateRoot "history.jsonl"

if ([string]::IsNullOrWhiteSpace($OutputPath)) {
    $OutputPath = Join-Path $runRoot "ntfs-dogfood-report.json"
}

if ($SelfTest) {
    $OutputPath = Resolve-OutputPath -Path $OutputPath
    Test-Self -RepoRoot $repoRoot -OutputPath $OutputPath
    exit 0
}

if ($Root.Count -eq 0) {
    $Root = @($repoRoot)
}

$OutputPath = Resolve-OutputPath -Path $OutputPath

New-Item -ItemType Directory -Force -Path $rawRoot, $configRoot, $stateRoot, $cacheRoot | Out-Null

$previousEnv = @{
    REBECCA_CONFIG_DIR = [Environment]::GetEnvironmentVariable("REBECCA_CONFIG_DIR", "Process")
    REBECCA_STATE_DIR = [Environment]::GetEnvironmentVariable("REBECCA_STATE_DIR", "Process")
    REBECCA_CACHE_DIR = [Environment]::GetEnvironmentVariable("REBECCA_CACHE_DIR", "Process")
    REBECCA_HISTORY_FILE = [Environment]::GetEnvironmentVariable("REBECCA_HISTORY_FILE", "Process")
}

try {
    [Environment]::SetEnvironmentVariable("REBECCA_CONFIG_DIR", $configRoot, "Process")
    [Environment]::SetEnvironmentVariable("REBECCA_STATE_DIR", $stateRoot, "Process")
    [Environment]::SetEnvironmentVariable("REBECCA_CACHE_DIR", $cacheRoot, "Process")
    [Environment]::SetEnvironmentVariable("REBECCA_HISTORY_FILE", $historyFile, "Process")

    $modes = if ($Mode -eq "both") {
        @("inspect-space", "clean-dry-run")
    }
    else {
        @($Mode)
    }

    $runs = @()
    foreach ($modeName in $modes) {
        foreach ($backendName in $Backend) {
            if ($modeName -eq "clean-dry-run") {
                $runId = "$modeName-$backendName"
                $run = Invoke-DogfoodCommand -ModeName $modeName -RequestedBackend $backendName -RootPath $repoRoot -TopLimit $Top -OutputDirectory $rawRoot -RunId $runId -CommandTimeoutSeconds $TimeoutSeconds -WorkingDirectory $repoRoot
                $historyWrote = Test-Path -LiteralPath $historyFile -PathType Leaf
                $historyLength = if ($historyWrote) { (Get-Item -LiteralPath $historyFile).Length } else { 0 }
                $run.safety | Add-Member -NotePropertyName history_wrote_during_dry_run -NotePropertyValue ($historyLength -gt 0)
                if ($historyLength -gt 0) {
                    $run.status = "safety-violation"
                }
                $runs += $run
                continue
            }

            foreach ($rootPath in $Root) {
                $rootSlug = ((Resolve-Path -LiteralPath $rootPath).ProviderPath -replace "[:\\\/\s]+", "-").Trim("-")
                $runId = "$modeName-$backendName-$rootSlug"
                $runs += Invoke-DogfoodCommand -ModeName $modeName -RequestedBackend $backendName -RootPath $rootPath -TopLimit $Top -OutputDirectory $rawRoot -RunId $runId -CommandTimeoutSeconds $TimeoutSeconds -WorkingDirectory $repoRoot
            }
        }
    }

    $comparisonSummaries = Add-Comparisons -Runs $runs

    $report = [pscustomobject]@{
        schema_version = 1
        generated_at_unix_seconds = Get-UnixTimeSeconds
        repo_root = $repoRoot
        git_commit = Get-GitCommit -RepoRoot $repoRoot
        script = $PSCommandPath
        host = Get-HostSummary
        output_path = $OutputPath
        state = [pscustomobject]@{
            run_root = $runRoot
            config_dir = $configRoot
            state_dir = $stateRoot
            cache_dir = $cacheRoot
            history_file = $historyFile
        }
        roots = @($Root | ForEach-Object { Get-RootSummary -Path $_ })
        comparisons = @($comparisonSummaries)
        runs = @($runs)
    }

    Ensure-OutputParent -Path $OutputPath
    $report | ConvertTo-Json -Depth 18 | Set-Content -LiteralPath $OutputPath -Encoding utf8
    Write-Host "Wrote NTFS dogfood report to $OutputPath"
}
finally {
    foreach ($key in $previousEnv.Keys) {
        [Environment]::SetEnvironmentVariable($key, $previousEnv[$key], "Process")
    }
}

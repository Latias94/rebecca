param(
    [string]$FixtureRoot = "",
    [string]$OutputDirectory = "",
    [int]$Top = 20,
    [int]$DiagnosticLimit = 0,
    [int]$TimeoutSeconds = 240,
    [int]$IndexTimeoutSeconds = 60,
    [switch]$AllowMismatch,
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

function Resolve-UnderRepoPath {
    param(
        [string]$Path,
        [string]$DefaultRelativeRoot
    )

    $repoRoot = Get-RepoRoot
    if ([string]::IsNullOrWhiteSpace($Path)) {
        return [IO.Path]::GetFullPath((Join-Path $repoRoot (Join-Path $DefaultRelativeRoot (Get-TimestampId))))
    }
    if ([IO.Path]::IsPathRooted($Path)) {
        return [IO.Path]::GetFullPath($Path)
    }
    return [IO.Path]::GetFullPath((Join-Path (Get-Location).ProviderPath $Path))
}

function Assert-NotDriveRoot {
    param([string]$Path)

    $full = [IO.Path]::GetFullPath($Path).TrimEnd('\', '/')
    $driveRoot = [IO.Path]::GetPathRoot($full)
    if (-not [string]::IsNullOrWhiteSpace($driveRoot)) {
        $driveRoot = $driveRoot.TrimEnd('\', '/')
    }
    if ($full -eq $driveRoot) {
        throw "Refusing to use a drive root for dogfood output or fixtures: $Path"
    }
}

function Assert-NewFixtureRoot {
    param([string]$Path)

    Assert-NotDriveRoot -Path $Path
    if (Test-Path -LiteralPath $Path) {
        $existing = @(Get-ChildItem -LiteralPath $Path -Force -ErrorAction Stop)
        if ($existing.Count -gt 0) {
            throw "Fixture root already exists and is not empty: $Path"
        }
    }
}

function Get-SafeName {
    param([string]$Value)

    return ($Value -replace '[^A-Za-z0-9_.-]', '-')
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

function Find-JsonValues {
    param(
        [object]$Node,
        [string[]]$Names,
        [bool]$Unique = $true
    )

    $values = [System.Collections.Generic.List[string]]::new()
    Add-JsonValues -Node $Node -Names $Names -Values $values
    if ($Unique) {
        return @($values | Select-Object -Unique)
    }
    return @($values)
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
            caveats = @(Find-JsonValues -Node $json -Names @("estimate_caveats") -Unique $false)
            totals = Get-ObjectProperty -Object $data -Name "totals"
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
        }
    }
}

function Get-CaveatCodesFromValue {
    param([object]$Value)

    if ($null -eq $Value) {
        return @()
    }
    if ($Value -is [string]) {
        $text = $Value.Trim()
        if ([string]::IsNullOrWhiteSpace($text)) {
            return @()
        }
        if ($text.StartsWith("{") -or $text.StartsWith("[")) {
            try {
                $parsed = $text | ConvertFrom-Json -Depth 32
                return @(Get-CaveatCodesFromValue -Value $parsed)
            }
            catch {
            }
        }
        return @($text)
    }
    if ($Value -is [System.Collections.IEnumerable]) {
        $codes = @()
        foreach ($item in $Value) {
            $codes += @(Get-CaveatCodesFromValue -Value $item)
        }
        return @($codes)
    }

    $code = Get-ObjectProperty -Object $Value -Name "code"
    if ([string]::IsNullOrWhiteSpace([string]$code)) {
        return @()
    }
    return @([string]$code)
}

function Get-CaveatCodeCounts {
    param([object[]]$Caveats)

    $counts = @{}
    foreach ($caveat in @($Caveats)) {
        foreach ($code in @(Get-CaveatCodesFromValue -Value $caveat)) {
            if ([string]::IsNullOrWhiteSpace($code)) {
                continue
            }
            if (-not $counts.ContainsKey($code)) {
                $counts[$code] = 0
            }
            $counts[$code] += 1
        }
    }

    return @($counts.Keys | Sort-Object | ForEach-Object {
        [pscustomobject]@{
            code = [string]$_
            count = [int]$counts[$_]
        }
    })
}

function Join-CaveatCodeCounts {
    param([object[]]$Counts)

    return (@($Counts) | ForEach-Object { "$($_.code)=$($_.count)" }) -join ";"
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

function Join-ReportValues {
    param([object[]]$Values)

    if ($null -eq $Values) {
        return ""
    }
    return (@($Values) | Where-Object { $null -ne $_ -and [string]$_ -ne "" }) -join ";"
}

function Get-BackendSourceKind {
    param([object[]]$Sources)

    $sourceValues = @($Sources | ForEach-Object { [string]$_ })
    if ($sourceValues -contains "windows-ntfs-mft-experimental-targeted-fsctl") {
        return "ntfs-targeted-fsctl"
    }
    if ($sourceValues -contains "windows-ntfs-mft-experimental-sequential") {
        return "ntfs-full-index-sequential"
    }
    if ($sourceValues -contains "windows-ntfs-mft-experimental-fsctl-record") {
        return "ntfs-full-index-fsctl-record"
    }
    if ($sourceValues -contains "windows-ntfs-mft-experimental-persistent-cache") {
        return "ntfs-full-index-persistent-cache"
    }
    if ($sourceValues.Count -gt 0) {
        return "other"
    }
    return ""
}

function Test-RebuildSourceKind {
    param([string]$SourceKind)

    return $SourceKind -in @("ntfs-full-index-sequential", "ntfs-full-index-fsctl-record")
}

function Test-PersistentSourceKind {
    param([string]$SourceKind)

    return $SourceKind -eq "ntfs-full-index-persistent-cache"
}

function New-DeterministicFile {
    param(
        [string]$Path,
        [int]$ByteCount,
        [int]$Seed
    )

    $parent = Split-Path -Parent $Path
    New-Item -ItemType Directory -Force -Path $parent | Out-Null
    $bytes = [byte[]]::new($ByteCount)
    for ($i = 0; $i -lt $bytes.Length; $i++) {
        $bytes[$i] = [byte](($Seed + $i) % 251)
    }
    [IO.File]::WriteAllBytes($Path, $bytes)
}

function New-UsnReplayFixture {
    param([string]$Root)

    Assert-NewFixtureRoot -Path $Root
    New-Item -ItemType Directory -Force -Path $Root | Out-Null

    $targetRoot = Join-Path $Root "target"
    $unrelatedRoot = Join-Path $Root "unrelated"
    New-DeterministicFile -Path (Join-Path $targetRoot "nested\a.bin") -ByteCount 32768 -Seed 11
    New-DeterministicFile -Path (Join-Path $targetRoot "nested\b.bin") -ByteCount 49152 -Seed 23
    New-DeterministicFile -Path (Join-Path $targetRoot "top.bin") -ByteCount 16384 -Seed 37
    New-DeterministicFile -Path (Join-Path $unrelatedRoot "stable.bin") -ByteCount 65536 -Seed 41

    return [pscustomobject]@{
        fixture_root = $Root
        target_root = $targetRoot
        unrelated_root = $unrelatedRoot
        expected_initial_target_logical_bytes = 98304
    }
}

function Add-UnrelatedChange {
    param(
        [string]$UnrelatedRoot,
        [int]$Sequence
    )

    $path = Join-Path $UnrelatedRoot "unrelated-change-$Sequence.bin"
    New-DeterministicFile -Path $path -ByteCount 12288 -Seed (100 + $Sequence)
    return $path
}

function Add-TargetChange {
    param(
        [string]$TargetRoot,
        [int]$Sequence
    )

    $path = Join-Path $TargetRoot "nested\target-change-$Sequence.bin"
    New-DeterministicFile -Path $path -ByteCount 8192 -Seed (150 + $Sequence)
    return $path
}

function Get-CacheSnapshot {
    param([string]$CacheRoot)

    $ntfsRoot = Join-Path $CacheRoot "ntfs-volume-index"
    $files = @()
    if (Test-Path -LiteralPath $ntfsRoot) {
        $files = @(Get-ChildItem -LiteralPath $ntfsRoot -File -Recurse -Force -ErrorAction Stop)
    }
    $totalBytes = 0L
    foreach ($file in $files) {
        $totalBytes += [int64]$file.Length
    }

    return [pscustomobject]@{
        cache_root = $CacheRoot
        ntfs_volume_index_root = $ntfsRoot
        file_count = $files.Count
        total_bytes = $totalBytes
    }
}

function New-PhaseSummary {
    param(
        [string]$Phase,
        [int]$PhaseIndex,
        [int]$ExitCode,
        [bool]$TimedOut,
        [int64]$DurationMs,
        [string]$StdoutPath,
        [string]$StderrPath,
        [object]$Probe,
        [object]$CacheSnapshot
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
    $caveatCodeCounts = @(Get-CaveatCodeCounts -Caveats $Probe.caveats)

    return [pscustomobject]@{
        phase = $Phase
        phase_index = $PhaseIndex
        status = $status
        expectation_status = ""
        expectation_detail = ""
        exit_code = $ExitCode
        timed_out = $TimedOut
        duration_ms = $DurationMs
        actual_backend = $actualBackend
        actual_backends = $actualBackends
        backend_sources = @($Probe.backend_sources)
        backend_source_kind = Get-BackendSourceKind -Sources $Probe.backend_sources
        fallback_reasons = @($Probe.fallback_reasons)
        caveat_codes = @($caveatCodeCounts | ForEach-Object { $_.code })
        caveat_code_counts = $caveatCodeCounts
        logical_bytes = Get-MetricValue -Object $totals -Name "logical_bytes"
        allocated_bytes = Get-MetricValue -Object $totals -Name "allocated_bytes"
        files = Get-MetricValue -Object $totals -Name "files"
        directories = Get-MetricValue -Object $totals -Name "directories"
        logical_delta_from_previous = $null
        cache_file_count = $CacheSnapshot.file_count
        cache_total_bytes = $CacheSnapshot.total_bytes
        stdout_path = $StdoutPath
        stderr_path = $StderrPath
        parse_error = $Probe.parse_error
    }
}

function Invoke-InspectMapPhase {
    param(
        [string]$RepoRoot,
        [string]$TargetRoot,
        [string]$Phase,
        [int]$PhaseIndex,
        [int]$TopLimit,
        [int]$DiagnosticLimitValue,
        [int]$TimeoutSecondsValue,
        [string]$RawDirectory,
        [hashtable]$EnvironmentOverrides
    )

    $safePhase = Get-SafeName -Value ("{0:D2}-{1}" -f $PhaseIndex, $Phase)
    $stdoutPath = Join-Path $RawDirectory "$safePhase.stdout.json"
    $stderrPath = Join-Path $RawDirectory "$safePhase.stderr.txt"

    $arguments = [System.Collections.Generic.List[string]]::new()
    foreach ($arg in @("run", "-q", "-p", "rebecca", "--", "inspect", "map", "--format", "json", "--root", $TargetRoot, "--top", [string]$TopLimit, "--diagnostic-limit", [string]$DiagnosticLimitValue, "--scan-backend", "windows-ntfs-mft-experimental", "--group-by", "extension", "--group-by", "depth")) {
        $arguments.Add($arg)
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
    [IO.File]::WriteAllText($stdoutPath, $stdout)
    [IO.File]::WriteAllText($stderrPath, $stderr)
    $durationMs = [int64]([DateTimeOffset]::UtcNow - $started).TotalMilliseconds
    $exitCode = if ($timedOut) { -1 } else { $process.ExitCode }
    $probe = Convert-InspectMapJson -Raw $stdout
    $cacheSnapshot = Get-CacheSnapshot -CacheRoot $EnvironmentOverrides["REBECCA_CACHE_DIR"]

    return New-PhaseSummary -Phase $Phase -PhaseIndex $PhaseIndex -ExitCode $exitCode -TimedOut $timedOut -DurationMs $durationMs -StdoutPath $stdoutPath -StderrPath $stderrPath -Probe $probe -CacheSnapshot $cacheSnapshot
}

function Set-Expectation {
    param(
        [object]$Phase,
        [bool]$Passed,
        [string]$Detail
    )

    $Phase.expectation_status = if ($Passed) { "passed" } else { "failed" }
    $Phase.expectation_detail = $Detail
}

function Add-PhaseExpectations {
    param([object[]]$Phases)

    $byPhase = @{}
    $previous = $null
    foreach ($phase in $Phases) {
        if ($null -ne $previous -and $null -ne $phase.logical_bytes -and $null -ne $previous.logical_bytes) {
            $phase.logical_delta_from_previous = $phase.logical_bytes - $previous.logical_bytes
        }
        $byPhase[$phase.phase] = $phase
        $previous = $phase
    }

    foreach ($phase in $Phases) {
        if ($phase.status -ne "passed") {
            Set-Expectation -Phase $phase -Passed $false -Detail "phase did not produce a parsed successful inspect-map response"
            continue
        }

        switch ($phase.phase) {
            "warm-build" {
                Set-Expectation -Phase $phase -Passed (Test-RebuildSourceKind -SourceKind $phase.backend_source_kind) -Detail "expected a full-index rebuild from a fresh isolated cache"
            }
            "unrelated-replay" {
                $warm = $byPhase["warm-build"]
                $sourcePassed = Test-PersistentSourceKind -SourceKind $phase.backend_source_kind
                $logicalPassed = $null -ne $warm -and $phase.logical_bytes -eq $warm.logical_bytes
                Set-Expectation -Phase $phase -Passed ($sourcePassed -and $logicalPassed) -Detail "expected persistent-cache hit and unchanged target logical bytes after unrelated mutation"
            }
            "target-invalidates" {
                $unrelated = $byPhase["unrelated-replay"]
                $sourcePassed = Test-RebuildSourceKind -SourceKind $phase.backend_source_kind
                $logicalPassed = $null -ne $unrelated -and $null -ne $phase.logical_bytes -and $null -ne $unrelated.logical_bytes -and $phase.logical_bytes -gt $unrelated.logical_bytes
                Set-Expectation -Phase $phase -Passed ($sourcePassed -and $logicalPassed) -Detail "expected target mutation to invalidate the cache and increase target logical bytes"
            }
            "post-rebuild-hit" {
                $target = $byPhase["target-invalidates"]
                $sourcePassed = Test-PersistentSourceKind -SourceKind $phase.backend_source_kind
                $logicalPassed = $null -ne $target -and $phase.logical_bytes -eq $target.logical_bytes
                Set-Expectation -Phase $phase -Passed ($sourcePassed -and $logicalPassed) -Detail "expected persistent-cache hit after the target-change rebuild"
            }
            default {
                Set-Expectation -Phase $phase -Passed $false -Detail "unknown phase"
            }
        }
    }

    return @($Phases)
}

function Get-ReportFailureMessages {
    param(
        [object[]]$Phases,
        [bool]$AllowMismatchValue
    )

    $messages = [System.Collections.Generic.List[string]]::new()
    $failedRuns = @($Phases | Where-Object { $_.status -ne "passed" })
    if ($failedRuns.Count -gt 0) {
        $messages.Add("phase failures: " + (($failedRuns | ForEach-Object { "$($_.phase)=$($_.status)" }) -join ", "))
    }

    if (-not $AllowMismatchValue) {
        $failedExpectations = @($Phases | Where-Object { $_.expectation_status -ne "passed" })
        if ($failedExpectations.Count -gt 0) {
            $messages.Add("expectation failures: " + (($failedExpectations | ForEach-Object { "$($_.phase)=$($_.backend_source_kind)" }) -join ", ") + " (pass -AllowMismatch to keep the report exit code at 0)")
        }
    }

    return @($messages)
}

function Format-MarkdownCell {
    param([object]$Value)

    if ($null -eq $Value) {
        return ""
    }
    return ([string]$Value).Replace("|", "\|").Replace("`r", " ").Replace("`n", " ")
}

function New-MarkdownSummary {
    param(
        [object]$Report,
        [object[]]$Phases
    )

    $lines = [System.Collections.Generic.List[string]]::new()
    $lines.Add("# NTFS USN Replay Dogfood")
    $lines.Add("")
    $lines.Add("- Fixture: $($Report.fixture_root)")
    $lines.Add("- Target: $($Report.target_root)")
    $lines.Add("- Generated: $($Report.generated_at_utc)")
    $lines.Add("- Commit: $($Report.git_commit)")
    $lines.Add("- Cache root: $($Report.runtime.cache_dir)")
    $lines.Add("")
    $lines.Add("| Phase | Status | Expectation | Source kind | ms | Logical bytes | Delta | Cache files | Cache bytes | Caveats |")
    $lines.Add("| --- | --- | --- | --- | ---: | ---: | ---: | ---: | ---: | --- |")
    foreach ($phase in $Phases) {
        $caveats = Join-CaveatCodeCounts -Counts $phase.caveat_code_counts
        if ([string]::IsNullOrWhiteSpace($caveats)) {
            $caveats = "-"
        }
        $lines.Add("| $(Format-MarkdownCell $phase.phase) | $($phase.status) | $($phase.expectation_status) | $(Format-MarkdownCell $phase.backend_source_kind) | $($phase.duration_ms) | $($phase.logical_bytes) | $($phase.logical_delta_from_previous) | $($phase.cache_file_count) | $($phase.cache_total_bytes) | $(Format-MarkdownCell $caveats) |")
    }
    $lines.Add("")
    $lines.Add("## Expectations")
    $lines.Add("")
    foreach ($phase in $Phases) {
        $lines.Add("- ``$($phase.phase)``: $($phase.expectation_status) - $($phase.expectation_detail)")
    }
    $lines.Add("")
    return ($lines -join [Environment]::NewLine)
}

function Invoke-SelfTest {
    $persistentSample = @{
        api_version = "rebecca.cli.v1"
        kind = "success"
        command = "inspect map"
        payload_kind = "inspect-map"
        data = @{
            totals = @{
                logical_bytes = 100
                allocated_bytes = 100
                files = 3
                directories = 1
            }
            top_entries = @(
                @{
                    estimate_backend = "windows-ntfs-mft-experimental"
                    estimate_backend_source = "windows-ntfs-mft-experimental-persistent-cache"
                    estimate_caveats = @()
                }
            )
        }
    } | ConvertTo-Json -Depth 32
    $rebuildSample = @{
        api_version = "rebecca.cli.v1"
        kind = "success"
        command = "inspect map"
        payload_kind = "inspect-map"
        data = @{
            totals = @{
                logical_bytes = 108
                allocated_bytes = 108
                files = 4
                directories = 1
            }
            top_entries = @(
                @{
                    estimate_backend = "windows-ntfs-mft-experimental"
                    estimate_backend_source = "windows-ntfs-mft-experimental-sequential"
                    estimate_caveats = @(
                        @{
                            code = "mft-persistent-cache-miss"
                            message = "persistent NTFS/MFT volume-index cache missed; reason=manifest-missing"
                        }
                    )
                }
            )
        }
    } | ConvertTo-Json -Depth 32

    $persistentProbe = Convert-InspectMapJson -Raw $persistentSample
    $rebuildProbe = Convert-InspectMapJson -Raw $rebuildSample
    if (-not $persistentProbe.parsed -or -not $rebuildProbe.parsed) {
        throw "self-test JSON parsing failed"
    }
    if ((Get-BackendSourceKind -Sources $persistentProbe.backend_sources) -ne "ntfs-full-index-persistent-cache") {
        throw "self-test persistent source classification failed"
    }
    if (@($rebuildProbe.caveats).Count -ne 1) {
        throw "self-test caveat extraction failed"
    }

    $cacheSnapshot = [pscustomobject]@{
        file_count = 2
        total_bytes = 4096
    }
    $phases = @(
        (New-PhaseSummary -Phase "warm-build" -PhaseIndex 1 -ExitCode 0 -TimedOut $false -DurationMs 1 -StdoutPath "out" -StderrPath "err" -Probe $rebuildProbe -CacheSnapshot $cacheSnapshot),
        (New-PhaseSummary -Phase "unrelated-replay" -PhaseIndex 2 -ExitCode 0 -TimedOut $false -DurationMs 1 -StdoutPath "out" -StderrPath "err" -Probe $persistentProbe -CacheSnapshot $cacheSnapshot),
        (New-PhaseSummary -Phase "target-invalidates" -PhaseIndex 3 -ExitCode 0 -TimedOut $false -DurationMs 1 -StdoutPath "out" -StderrPath "err" -Probe $rebuildProbe -CacheSnapshot $cacheSnapshot),
        (New-PhaseSummary -Phase "post-rebuild-hit" -PhaseIndex 4 -ExitCode 0 -TimedOut $false -DurationMs 1 -StdoutPath "out" -StderrPath "err" -Probe $persistentProbe -CacheSnapshot $cacheSnapshot)
    )
    $phases[0].logical_bytes = 100
    $phases[1].logical_bytes = 100
    $phases[2].logical_bytes = 108
    $phases[3].logical_bytes = 108
    $phases = @(Add-PhaseExpectations -Phases $phases)
    if (@($phases | Where-Object { $_.expectation_status -ne "passed" }).Count -ne 0) {
        throw "self-test expectation pass logic failed"
    }
    if ((Join-CaveatCodeCounts -Counts $phases[0].caveat_code_counts) -ne "mft-persistent-cache-miss=1") {
        throw "self-test caveat code count failed"
    }

    $phases[1].backend_source_kind = "ntfs-full-index-sequential"
    $phases = @(Add-PhaseExpectations -Phases $phases)
    if (@(Get-ReportFailureMessages -Phases $phases -AllowMismatchValue $false).Count -ne 1) {
        throw "self-test expectation failure detection failed"
    }
    if (@(Get-ReportFailureMessages -Phases $phases -AllowMismatchValue $true).Count -ne 0) {
        throw "self-test AllowMismatch handling failed"
    }

    $markdown = New-MarkdownSummary -Report ([pscustomobject]@{
        fixture_root = "C:\fixture"
        target_root = "C:\fixture\target"
        generated_at_utc = "1970-01-01T00:00:00Z"
        git_commit = "selftest"
        runtime = [pscustomobject]@{ cache_dir = "C:\cache" }
    }) -Phases $phases
    if ($markdown -notlike "*NTFS USN Replay Dogfood*" -or $markdown -notlike "*unrelated-replay*") {
        throw "self-test markdown failed"
    }
    if ($markdown -like "*`$(@*") {
        throw "self-test markdown phase escaping failed"
    }

    Write-Host "Self-test passed."
}

if ($SelfTest) {
    Invoke-SelfTest
    exit 0
}

if ($Top -lt 1) {
    throw "Top must be at least 1."
}
if ($DiagnosticLimit -lt 0) {
    throw "DiagnosticLimit must not be negative."
}
if ($TimeoutSeconds -lt 1) {
    throw "TimeoutSeconds must be at least 1."
}
if ($IndexTimeoutSeconds -lt 0) {
    throw "IndexTimeoutSeconds must not be negative."
}

$repoRoot = Get-RepoRoot
$fixtureRootPath = Resolve-UnderRepoPath -Path $FixtureRoot -DefaultRelativeRoot "target\ntfs-usn-replay-fixtures"
$outputRoot = Resolve-UnderRepoPath -Path $OutputDirectory -DefaultRelativeRoot "target\ntfs-usn-replay-dogfood"
Assert-NotDriveRoot -Path $outputRoot
New-Item -ItemType Directory -Force -Path $outputRoot | Out-Null
$rawDirectory = Join-Path $outputRoot "raw"
New-Item -ItemType Directory -Force -Path $rawDirectory | Out-Null

$fixture = New-UsnReplayFixture -Root $fixtureRootPath
$runtimeRoot = Join-Path $outputRoot "runtime"
$environmentOverrides = @{
    REBECCA_CONFIG_DIR = Join-Path $runtimeRoot "config"
    REBECCA_STATE_DIR = Join-Path $runtimeRoot "state"
    REBECCA_CACHE_DIR = Join-Path $runtimeRoot "cache"
    REBECCA_HISTORY_FILE = Join-Path (Join-Path $runtimeRoot "state") "history.jsonl"
    REBECCA_NTFS_MFT_VOLUME_INDEX_CACHE = "1"
    REBECCA_NTFS_MFT_FULL_INDEX_FALLBACK = "1"
    REBECCA_NTFS_MFT_INDEX_TIMINGS = "1"
    REBECCA_NTFS_MFT_INDEX_TIMEOUT_SECONDS = [string]$IndexTimeoutSeconds
}
foreach ($value in $environmentOverrides.Values) {
    $parent = if ([IO.Path]::HasExtension($value)) { Split-Path -Parent $value } else { $value }
    if (-not [string]::IsNullOrWhiteSpace($parent)) {
        New-Item -ItemType Directory -Force -Path $parent | Out-Null
    }
}

$fixtureManifestPath = Join-Path $outputRoot "ntfs-usn-replay-fixture.json"
$fixture | ConvertTo-Json -Depth 16 | Set-Content -LiteralPath $fixtureManifestPath -Encoding utf8

$phases = @()
$phases += Invoke-InspectMapPhase -RepoRoot $repoRoot -TargetRoot $fixture.target_root -Phase "warm-build" -PhaseIndex 1 -TopLimit $Top -DiagnosticLimitValue $DiagnosticLimit -TimeoutSecondsValue $TimeoutSeconds -RawDirectory $rawDirectory -EnvironmentOverrides $environmentOverrides
$unrelatedChangePath = Add-UnrelatedChange -UnrelatedRoot $fixture.unrelated_root -Sequence 1
$phases += Invoke-InspectMapPhase -RepoRoot $repoRoot -TargetRoot $fixture.target_root -Phase "unrelated-replay" -PhaseIndex 2 -TopLimit $Top -DiagnosticLimitValue $DiagnosticLimit -TimeoutSecondsValue $TimeoutSeconds -RawDirectory $rawDirectory -EnvironmentOverrides $environmentOverrides
$targetChangePath = Add-TargetChange -TargetRoot $fixture.target_root -Sequence 1
$phases += Invoke-InspectMapPhase -RepoRoot $repoRoot -TargetRoot $fixture.target_root -Phase "target-invalidates" -PhaseIndex 3 -TopLimit $Top -DiagnosticLimitValue $DiagnosticLimit -TimeoutSecondsValue $TimeoutSeconds -RawDirectory $rawDirectory -EnvironmentOverrides $environmentOverrides
$phases += Invoke-InspectMapPhase -RepoRoot $repoRoot -TargetRoot $fixture.target_root -Phase "post-rebuild-hit" -PhaseIndex 4 -TopLimit $Top -DiagnosticLimitValue $DiagnosticLimit -TimeoutSecondsValue $TimeoutSeconds -RawDirectory $rawDirectory -EnvironmentOverrides $environmentOverrides
$phases = @(Add-PhaseExpectations -Phases $phases)

$report = [pscustomobject]@{
    schema_version = 1
    generated_at_utc = [DateTimeOffset]::UtcNow.ToString("O")
    git_commit = (& git -C $repoRoot rev-parse --short HEAD 2>$null)
    repo_root = $repoRoot
    fixture_root = $fixture.fixture_root
    target_root = $fixture.target_root
    unrelated_root = $fixture.unrelated_root
    unrelated_change_path = $unrelatedChangePath
    target_change_path = $targetChangePath
    output_directory = $outputRoot
    timeout_seconds = $TimeoutSeconds
    index_timeout_seconds = $IndexTimeoutSeconds
    top = $Top
    diagnostic_limit = $DiagnosticLimit
    runtime = [pscustomobject]@{
        config_dir = $environmentOverrides["REBECCA_CONFIG_DIR"]
        state_dir = $environmentOverrides["REBECCA_STATE_DIR"]
        cache_dir = $environmentOverrides["REBECCA_CACHE_DIR"]
        history_file = $environmentOverrides["REBECCA_HISTORY_FILE"]
    }
    environment = [pscustomobject]@{
        REBECCA_NTFS_MFT_VOLUME_INDEX_CACHE = $environmentOverrides["REBECCA_NTFS_MFT_VOLUME_INDEX_CACHE"]
        REBECCA_NTFS_MFT_FULL_INDEX_FALLBACK = $environmentOverrides["REBECCA_NTFS_MFT_FULL_INDEX_FALLBACK"]
        REBECCA_NTFS_MFT_INDEX_TIMINGS = $environmentOverrides["REBECCA_NTFS_MFT_INDEX_TIMINGS"]
        REBECCA_NTFS_MFT_INDEX_TIMEOUT_SECONDS = $environmentOverrides["REBECCA_NTFS_MFT_INDEX_TIMEOUT_SECONDS"]
    }
    phases = @($phases)
}

$reportPath = Join-Path $outputRoot "ntfs-usn-replay-report.json"
$summaryPath = Join-Path $outputRoot "ntfs-usn-replay-summary.md"
$report | ConvertTo-Json -Depth 64 | Set-Content -LiteralPath $reportPath -Encoding utf8
New-MarkdownSummary -Report $report -Phases $phases | Set-Content -LiteralPath $summaryPath -Encoding utf8

Write-Host "NTFS USN replay dogfood report written to $outputRoot"
Write-Host "  JSON: $reportPath"
Write-Host "  Summary: $summaryPath"
Write-Host "  Fixture: $fixtureManifestPath"

$failureMessages = @(Get-ReportFailureMessages -Phases $phases -AllowMismatchValue ([bool]$AllowMismatch))
if ($failureMessages.Count -gt 0) {
    foreach ($message in $failureMessages) {
        [Console]::Error.WriteLine($message)
    }
    exit 1
}

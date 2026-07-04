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
    [switch]$AllowOutputInsideRoot,
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

function Get-NormalizedPathForComparison {
    param([string]$Path)

    $full = [System.IO.Path]::GetFullPath($Path).TrimEnd('\', '/')
    if ([string]::IsNullOrWhiteSpace($full)) {
        return $full
    }
    $root = [System.IO.Path]::GetPathRoot($full)
    if ($null -ne $root -and $full -eq $root.TrimEnd('\', '/')) {
        return $full
    }
    return $full
}

function Test-PathSameOrChild {
    param(
        [string]$Parent,
        [string]$Child
    )

    $parentFull = Get-NormalizedPathForComparison -Path $Parent
    $childFull = Get-NormalizedPathForComparison -Path $Child
    if ($childFull.Equals($parentFull, [System.StringComparison]::OrdinalIgnoreCase)) {
        return $true
    }
    $parentWithSeparator = $parentFull.TrimEnd('\', '/') + [System.IO.Path]::DirectorySeparatorChar
    return $childFull.StartsWith($parentWithSeparator, [System.StringComparison]::OrdinalIgnoreCase)
}

function Assert-OutputDirectoryOutsideRoot {
    param(
        [string]$RootPath,
        [string]$OutputRoot,
        [bool]$AllowInside
    )

    if ($AllowInside) {
        return
    }
    if (Test-PathSameOrChild -Parent $RootPath -Child $OutputRoot) {
        throw "Refusing to place dogfood output inside the scanned root because it can pollute backend comparisons. Pass -OutputDirectory outside '$RootPath' or add -AllowOutputInsideRoot: $OutputRoot"
    }
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

function Get-ReportFailureMessages {
    param(
        [object[]]$Runs,
        [bool]$AllowMismatchValue
    )

    $messages = [System.Collections.Generic.List[string]]::new()
    $failedRuns = @($Runs | Where-Object { $_.status -ne "passed" })
    if ($failedRuns.Count -gt 0) {
        $messages.Add("run failures: " + (($failedRuns | ForEach-Object { "$($_.run_id)=$($_.status)" }) -join ", "))
    }

    if (-not $AllowMismatchValue) {
        $comparisonFailures = @(
            $Runs | Where-Object {
                $_.comparison_status -in @("mismatched", "baseline-missing", "run-not-passed")
            }
        )
        if ($comparisonFailures.Count -gt 0) {
            $messages.Add("comparison failures: " + (($comparisonFailures | ForEach-Object { "$($_.run_id)=$($_.comparison_status)" }) -join ", ") + " (pass -AllowMismatch to keep the report exit code at 0)")
        }
    }

    return @($messages)
}

function Join-ReportValues {
    param([object[]]$Values)

    if ($null -eq $Values) {
        return ""
    }
    return (@($Values) | Where-Object { $null -ne $_ -and [string]$_ -ne "" }) -join ";"
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

function Get-CaveatCodesFromCounts {
    param([object[]]$Counts)

    return @(@($Counts) | ForEach-Object { [string]$_.code })
}

function Get-CaveatCodeCount {
    param(
        [object[]]$Counts,
        [string]$Code
    )

    foreach ($count in @($Counts)) {
        if ($count.code -eq $Code) {
            return [int]$count.count
        }
    }
    return 0
}

function Join-CaveatCodeCounts {
    param([object[]]$Counts)

    return (@($Counts) | ForEach-Object { "$($_.code)=$($_.count)" }) -join ";"
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
    if ($sourceValues.Count -gt 0) {
        return "other"
    }
    return ""
}

function Get-NtfsMirrorEvidence {
    param(
        [int]$RecordUsedCount,
        [int]$ReadFailedCount,
        [bool]$FullIndexSource
    )

    if ($RecordUsedCount -gt 0 -and $ReadFailedCount -gt 0) {
        return "record-used+read-failed"
    }
    if ($RecordUsedCount -gt 0) {
        return "record-used"
    }
    if ($ReadFailedCount -gt 0) {
        return "read-failed"
    }
    if ($FullIndexSource) {
        return "none"
    }
    return ""
}

function Format-MarkdownCell {
    param([object]$Value)

    if ($null -eq $Value) {
        return ""
    }
    return ([string]$Value).Replace("|", "\|").Replace("`r", " ").Replace("`n", " ")
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
            caveats = @(Find-JsonValues -Node $json -Names @("estimate_caveats") -Unique $false)
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

function Get-RatePerSecond {
    param(
        [object]$Value,
        [int64]$DurationMs
    )

    if ($null -eq $Value -or $DurationMs -le 0) {
        return $null
    }
    return [Math]::Round(([double]$Value * 1000.0) / [double]$DurationMs, 3)
}

function Get-NullableDelta {
    param(
        [object]$Left,
        [object]$Right
    )

    if ($null -eq $Left -or $null -eq $Right) {
        return $null
    }
    return $Left - $Right
}

function Get-NullableComparisonStatus {
    param(
        [object]$Left,
        [object]$Right
    )

    if ($null -eq $Left -or $null -eq $Right) {
        return "unknown"
    }
    if ($Left -eq $Right) {
        return "matched"
    }
    return "mismatched"
}

function Merge-ComparisonStatuses {
    param([string[]]$Statuses)

    if ($Statuses -contains "mismatched") {
        return "mismatched"
    }
    if ($Statuses -contains "unknown") {
        return "unknown"
    }
    return "matched"
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
    $logicalBytes = Get-MetricValue -Object $totals -Name "logical_bytes"
    $allocatedBytes = Get-MetricValue -Object $totals -Name "allocated_bytes"
    $uniqueLogicalBytes = Get-MetricValue -Object $totals -Name "unique_logical_bytes"
    $uniqueAllocatedBytes = Get-MetricValue -Object $totals -Name "unique_allocated_bytes"
    $files = Get-MetricValue -Object $totals -Name "files"
    $directories = Get-MetricValue -Object $totals -Name "directories"
    $caveats = @($Probe.caveats)
    $caveatCodeCounts = @(Get-CaveatCodeCounts -Caveats $caveats)
    $backendSourceKind = Get-BackendSourceKind -Sources $Probe.backend_sources
    $ntfsFullIndexSource = $backendSourceKind -in @("ntfs-full-index-sequential", "ntfs-full-index-fsctl-record")
    $mftMirrorRecordUsedCount = Get-CaveatCodeCount -Counts $caveatCodeCounts -Code "mft-mirror-record-used"
    $mftMirrorReadFailedCount = Get-CaveatCodeCount -Counts $caveatCodeCounts -Code "mft-mirror-read-failed"

    return [pscustomobject]@{
        run_id = $RunId
        repeat = $RepeatIndex
        root = $RootPath
        requested_backend = $RequestedBackend
        actual_backend = $actualBackend
        actual_backends = $actualBackends
        backend_sources = @($Probe.backend_sources)
        backend_source_kind = $backendSourceKind
        fallback_reasons = @($Probe.fallback_reasons)
        caveats = $caveats
        caveat_codes = @(Get-CaveatCodesFromCounts -Counts $caveatCodeCounts)
        caveat_code_counts = $caveatCodeCounts
        ntfs_full_index_source = $ntfsFullIndexSource
        ntfs_mirror_record_used_count = $mftMirrorRecordUsedCount
        ntfs_mirror_read_failed_count = $mftMirrorReadFailedCount
        ntfs_mirror_evidence = Get-NtfsMirrorEvidence -RecordUsedCount $mftMirrorRecordUsedCount -ReadFailedCount $mftMirrorReadFailedCount -FullIndexSource $ntfsFullIndexSource
        status = $status
        exit_code = $ExitCode
        timed_out = $TimedOut
        duration_ms = $DurationMs
        files_per_second = Get-RatePerSecond -Value $files -DurationMs $DurationMs
        directories_per_second = Get-RatePerSecond -Value $directories -DurationMs $DurationMs
        logical_bytes_per_second = Get-RatePerSecond -Value $logicalBytes -DurationMs $DurationMs
        allocated_bytes_per_second = Get-RatePerSecond -Value $allocatedBytes -DurationMs $DurationMs
        logical_bytes = $logicalBytes
        allocated_bytes = $allocatedBytes
        unique_logical_bytes = $uniqueLogicalBytes
        unique_allocated_bytes = $uniqueAllocatedBytes
        files = $files
        directories = $directories
        caveat_count = $caveats.Count
        diagnostic_total = Get-MetricValue -Object $diag -Name "total"
        diagnostic_retained = Get-MetricValue -Object $diag -Name "retained"
        diagnostic_truncated = Get-MetricValue -Object $diag -Name "truncated"
        stdout_path = $StdoutPath
        stderr_path = $StderrPath
        parse_error = $Probe.parse_error
        comparison_status = ""
        logical_delta = $null
        allocated_delta = $null
        allocated_comparison_status = ""
        file_delta = $null
        directory_delta = $null
        unique_logical_delta = $null
        unique_allocated_delta = $null
        unique_comparison_status = ""
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
            $run.allocated_delta = Get-NullableDelta -Left $run.allocated_bytes -Right $baseline.allocated_bytes
            $run.file_delta = $run.files - $baseline.files
            $run.directory_delta = $run.directories - $baseline.directories
            $run.unique_logical_delta = Get-NullableDelta -Left $run.unique_logical_bytes -Right $baseline.unique_logical_bytes
            $run.unique_allocated_delta = Get-NullableDelta -Left $run.unique_allocated_bytes -Right $baseline.unique_allocated_bytes
            $run.allocated_comparison_status = Get-NullableComparisonStatus -Left $run.allocated_bytes -Right $baseline.allocated_bytes
            $run.unique_comparison_status = Merge-ComparisonStatuses -Statuses @(
                Get-NullableComparisonStatus -Left $run.unique_logical_bytes -Right $baseline.unique_logical_bytes
                Get-NullableComparisonStatus -Left $run.unique_allocated_bytes -Right $baseline.unique_allocated_bytes
            )
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
            backend_source_kind = $_.backend_source_kind
            fallback_reasons = Join-ReportValues $_.fallback_reasons
            status = $_.status
            comparison_status = $_.comparison_status
            exit_code = $_.exit_code
            timed_out = $_.timed_out
            duration_ms = $_.duration_ms
            files_per_second = $_.files_per_second
            directories_per_second = $_.directories_per_second
            logical_bytes_per_second = $_.logical_bytes_per_second
            allocated_bytes_per_second = $_.allocated_bytes_per_second
            logical_bytes = $_.logical_bytes
            allocated_bytes = $_.allocated_bytes
            unique_logical_bytes = $_.unique_logical_bytes
            unique_allocated_bytes = $_.unique_allocated_bytes
            files = $_.files
            directories = $_.directories
            caveat_count = $_.caveat_count
            caveat_codes = Join-ReportValues $_.caveat_codes
            caveat_code_counts = Join-CaveatCodeCounts -Counts $_.caveat_code_counts
            ntfs_full_index_source = $_.ntfs_full_index_source
            ntfs_mirror_record_used_count = $_.ntfs_mirror_record_used_count
            ntfs_mirror_read_failed_count = $_.ntfs_mirror_read_failed_count
            ntfs_mirror_evidence = $_.ntfs_mirror_evidence
            logical_delta = $_.logical_delta
            allocated_delta = $_.allocated_delta
            allocated_comparison_status = $_.allocated_comparison_status
            file_delta = $_.file_delta
            directory_delta = $_.directory_delta
            unique_logical_delta = $_.unique_logical_delta
            unique_allocated_delta = $_.unique_allocated_delta
            unique_comparison_status = $_.unique_comparison_status
            diagnostic_total = $_.diagnostic_total
            diagnostic_retained = $_.diagnostic_retained
            diagnostic_truncated = $_.diagnostic_truncated
            stdout_path = $_.stdout_path
            stderr_path = $_.stderr_path
            parse_error = $_.parse_error
        }
    })
}

function Get-PercentileValue {
    param(
        [int64[]]$Values,
        [double]$Percentile
    )

    if ($Values.Count -eq 0) {
        return $null
    }
    $sorted = @($Values | Sort-Object)
    $index = [Math]::Ceiling(($Percentile / 100.0) * $sorted.Count) - 1
    if ($index -lt 0) { $index = 0 }
    if ($index -ge $sorted.Count) { $index = $sorted.Count - 1 }
    return [int64]$sorted[$index]
}

function New-RepeatStats {
    param([object[]]$Runs)

    return @($Runs | Group-Object requested_backend | ForEach-Object {
        $durations = @($_.Group | Where-Object { $null -ne $_.duration_ms } | ForEach-Object { [int64]$_.duration_ms })
        [pscustomobject]@{
            backend = $_.Name
            run_count = $durations.Count
            duration_min_ms = Get-PercentileValue -Values $durations -Percentile 0
            duration_median_ms = Get-PercentileValue -Values $durations -Percentile 50
            duration_p95_ms = Get-PercentileValue -Values $durations -Percentile 95
            duration_max_ms = Get-PercentileValue -Values $durations -Percentile 100
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
    $lines.Add("| Run | Backend | Actual | Status | Compare | Allocated compare | Unique compare | ms | Logical/s | Files/s | Caveats |")
    $lines.Add("| --- | --- | --- | --- | --- | --- | --- | ---: | ---: | ---: | ---: |")
    foreach ($run in $Runs) {
        $lines.Add("| $($run.run_id) | $($run.requested_backend) | $($run.actual_backend) | $($run.status) | $($run.comparison_status) | $($run.allocated_comparison_status) | $($run.unique_comparison_status) | $($run.duration_ms) | $($run.logical_bytes_per_second) | $($run.files_per_second) | $($run.caveat_count) |")
    }
    $lines.Add("")
    $lines.Add("## Backend Evidence")
    $lines.Add("")
    $lines.Add("| Run | Source kind | Full index | Mirror evidence | Caveat codes |")
    $lines.Add("| --- | --- | --- | --- | --- |")
    foreach ($run in $Runs) {
        $sourceKind = if ([string]::IsNullOrWhiteSpace($run.backend_source_kind)) { "-" } else { $run.backend_source_kind }
        $mirrorEvidence = if ([string]::IsNullOrWhiteSpace($run.ntfs_mirror_evidence)) { "-" } else { $run.ntfs_mirror_evidence }
        $caveatCounts = Join-CaveatCodeCounts -Counts $run.caveat_code_counts
        if ([string]::IsNullOrWhiteSpace($caveatCounts)) {
            $caveatCounts = "-"
        }
        $lines.Add("| $(Format-MarkdownCell $run.run_id) | $(Format-MarkdownCell $sourceKind) | $($run.ntfs_full_index_source) | $(Format-MarkdownCell $mirrorEvidence) | $(Format-MarkdownCell $caveatCounts) |")
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
                allocated_bytes = 10
                unique_logical_bytes = 10
                unique_allocated_bytes = 10
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
                    allocated_bytes = 10
                    unique_logical_bytes = 10
                    unique_allocated_bytes = 10
                    files = 2
                    directories = 1
                    estimate_source = "fresh-scan"
                    estimate_backend = "portable-recursive"
                    estimate_backend_source = "windows-ntfs-mft-experimental-sequential"
                    estimate_confidence = "exact"
                    estimate_caveats = @(
                        @{
                            code = "mft-mirror-record-used"
                            message = "record 0 recovered from bounded mirror bytes"
                        }
                        @{
                            code = "mft-mirror-record-used"
                            message = "record 1 recovered from bounded mirror bytes"
                        }
                        @{
                            code = "mft-mirror-read-failed"
                            message = "mirror bytes were unavailable"
                        }
                        @{
                            code = "mft-mirror-read-failed"
                            message = "mirror bytes were unavailable"
                        }
                    )
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
                        allocated_bytes = 10
                        unique_logical_bytes = 10
                        unique_allocated_bytes = 10
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
    if ($portable.caveat_count -ne 4) {
        throw "self-test caveat count failed"
    }
    if ($portable.backend_source_kind -ne "ntfs-full-index-sequential") {
        throw "self-test backend source kind failed"
    }
    if (-not $portable.ntfs_full_index_source) {
        throw "self-test full-index source failed"
    }
    if ($portable.ntfs_mirror_record_used_count -ne 2) {
        throw "self-test mirror record-used count failed"
    }
    if ($portable.ntfs_mirror_read_failed_count -ne 2) {
        throw "self-test mirror read-failed count failed"
    }
    if ($portable.ntfs_mirror_evidence -ne "record-used+read-failed") {
        throw "self-test mirror evidence failed"
    }
    $caveatCodeCounts = Join-CaveatCodeCounts -Counts $portable.caveat_code_counts
    if ($caveatCodeCounts -ne "mft-mirror-read-failed=2;mft-mirror-record-used=2") {
        throw "self-test caveat code counts failed: $caveatCodeCounts"
    }
    $runs = @(Add-RunComparisons -Runs @($portable, $native))
    if (($runs | Where-Object { $_.requested_backend -eq "windows-native" }).comparison_status -ne "matched") {
        throw "self-test comparison failed"
    }
    $nativeRun = $runs | Where-Object { $_.requested_backend -eq "windows-native" } | Select-Object -First 1
    if ($nativeRun.allocated_comparison_status -ne "matched") {
        throw "self-test allocated comparison failed"
    }
    if ($nativeRun.unique_comparison_status -ne "matched") {
        throw "self-test unique comparison failed"
    }
    if ($nativeRun.files_per_second -le 0 -or $nativeRun.logical_bytes_per_second -le 0) {
        throw "self-test throughput failed"
    }
    if ((Get-NullableComparisonStatus -Left $null -Right 1) -ne "unknown") {
        throw "self-test unknown nullable comparison failed"
    }
    $repeatStats = @(New-RepeatStats -Runs $runs)
    if ($repeatStats.Count -eq 0 -or $null -eq $repeatStats[0].duration_p95_ms) {
        throw "self-test repeat stats failed"
    }
    $mismatch = [pscustomobject]@{
        run_id = "r1-native-mismatch"
        status = "passed"
        comparison_status = "mismatched"
    }
    if (@(Get-ReportFailureMessages -Runs @($mismatch) -AllowMismatchValue $false).Count -ne 1) {
        throw "self-test mismatch failure detection failed"
    }
    if (@(Get-ReportFailureMessages -Runs @($mismatch) -AllowMismatchValue $true).Count -ne 0) {
        throw "self-test mismatch opt-in failed"
    }

    $temp = Join-Path ([System.IO.Path]::GetTempPath()) ("rebecca-inspect-map-dogfood-selftest-" + [Guid]::NewGuid().ToString("N"))
    New-Item -ItemType Directory -Force -Path $temp | Out-Null
    try {
        $rootForOverlap = Join-Path $temp "root"
        $insideOutput = Join-Path $rootForOverlap "target\inspect-map-dogfood\run"
        $outsideOutput = Join-Path $temp "outside"
        New-Item -ItemType Directory -Force -Path $rootForOverlap | Out-Null
        try {
            Assert-OutputDirectoryOutsideRoot -RootPath $rootForOverlap -OutputRoot $insideOutput -AllowInside $false
            throw "self-test output overlap rejection failed"
        }
        catch {
            if ($_.Exception.Message -notlike "Refusing to place dogfood output inside*") {
                throw
            }
        }
        Assert-OutputDirectoryOutsideRoot -RootPath $rootForOverlap -OutputRoot $insideOutput -AllowInside $true
        Assert-OutputDirectoryOutsideRoot -RootPath $rootForOverlap -OutputRoot $outsideOutput -AllowInside $false
        Convert-RunsForCsv -Runs $runs | Export-Csv -LiteralPath (Join-Path $temp "runs.csv") -NoTypeInformation
        $rows | Export-Csv -LiteralPath (Join-Path $temp "rows.csv") -NoTypeInformation
        if (-not (Test-Path -LiteralPath (Join-Path $temp "runs.csv"))) { throw "self-test csv failed" }
        if (-not (Test-Path -LiteralPath (Join-Path $temp "rows.csv"))) { throw "self-test rows csv failed" }
        $runCsvRows = @(Convert-RunsForCsv -Runs @($portable))
        if ($runCsvRows[0].caveat_code_counts -ne "mft-mirror-read-failed=2;mft-mirror-record-used=2") {
            throw "self-test csv caveat code counts failed"
        }
        if ($runCsvRows[0].ntfs_mirror_evidence -ne "record-used+read-failed") {
            throw "self-test csv mirror evidence failed"
        }
        $markdown = New-MarkdownSummary -Report ([pscustomobject]@{
            root = "C:\tmp"
            generated_at_utc = "1970-01-01T00:00:00Z"
            git_commit = "selftest"
            cleanup_advice = $false
        }) -Runs @($portable)
        if ($markdown -notlike "*mft-mirror-record-used=2*") {
            throw "self-test markdown caveat evidence failed"
        }
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
Assert-OutputDirectoryOutsideRoot -RootPath $resolvedRoot -OutputRoot $outputRoot -AllowInside ([bool]$AllowOutputInsideRoot)
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
    repeat_stats = @(New-RepeatStats -Runs $runs)
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

$failureMessages = @(Get-ReportFailureMessages -Runs $runs -AllowMismatchValue ([bool]$AllowMismatch))
if ($failureMessages.Count -gt 0) {
    foreach ($message in $failureMessages) {
        [Console]::Error.WriteLine("ERROR: $message")
    }
    exit 1
}

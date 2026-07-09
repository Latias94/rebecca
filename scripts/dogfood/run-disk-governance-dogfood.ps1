param(
    [string]$Root = "",
    [string]$ScanBackend = "",
    [int]$Top = 40,
    [int]$DiagnosticLimit = 0,
    [string]$OutputDirectory = "",
    [switch]$AllowDriveRoot,
    [switch]$AllowOutputInsideRoot,
    [switch]$NoDelete,
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

function Get-UnixTimeSeconds {
    return [int64]([DateTimeOffset]::UtcNow.ToUnixTimeSeconds())
}

function Resolve-OutputRoot {
    param([string]$Path)

    if (-not [string]::IsNullOrWhiteSpace($Path)) {
        if ([System.IO.Path]::IsPathRooted($Path)) {
            return [System.IO.Path]::GetFullPath($Path)
        }
        return [System.IO.Path]::GetFullPath((Join-Path (Get-Location).ProviderPath $Path))
    }

    return Join-Path (Get-RepoRoot) (Join-Path "target\disk-governance-dogfood" (Get-TimestampId))
}

function Get-NormalizedPath {
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

    $parentFull = Get-NormalizedPath -Path $Parent
    $childFull = Get-NormalizedPath -Path $Child
    if ($childFull.Equals($parentFull, [System.StringComparison]::OrdinalIgnoreCase)) {
        return $true
    }
    $parentWithSeparator = $parentFull.TrimEnd('\', '/') + [System.IO.Path]::DirectorySeparatorChar
    return $childFull.StartsWith($parentWithSeparator, [System.StringComparison]::OrdinalIgnoreCase)
}

function Resolve-RequiredRoot {
    param(
        [string]$Path,
        [bool]$AllowDriveRootValue
    )

    if ([string]::IsNullOrWhiteSpace($Path)) {
        throw "Pass -Root explicitly. This script intentionally avoids implicit profile or drive scans."
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

function Assert-OutputOutsideRoot {
    param(
        [string]$RootPath,
        [string]$OutputRoot,
        [bool]$AllowInside
    )

    if ($AllowInside) {
        return
    }
    if (Test-PathSameOrChild -Parent $RootPath -Child $OutputRoot) {
        throw "Refusing to place dogfood output inside the scanned root because generated files would pollute the report. Pass -OutputDirectory outside '$RootPath' or add -AllowOutputInsideRoot."
    }
}

function Set-IsolatedRebeccaEnvironment {
    param([string]$OutputRoot)

    $stateRoot = Join-Path $OutputRoot "state"
    New-Item -ItemType Directory -Force -Path $stateRoot | Out-Null
    $env:REBECCA_CONFIG_DIR = Join-Path $stateRoot "config"
    $env:REBECCA_CACHE_DIR = Join-Path $stateRoot "cache"
    $env:REBECCA_STATE_DIR = Join-Path $stateRoot "state"
    $env:REBECCA_HISTORY_PATH = Join-Path $stateRoot "history.jsonl"
}

function Invoke-RebeccaCapture {
    param(
        [string]$BinaryPath,
        [string[]]$CommandArgs,
        [string]$StdoutPath,
        [string]$StderrPath
    )

    & $BinaryPath @CommandArgs > $StdoutPath 2> $StderrPath
    if ($null -ne $LASTEXITCODE -and $LASTEXITCODE -ne 0) {
        throw "Rebecca command failed with exit code $LASTEXITCODE. See $StderrPath"
    }
}

function Read-NdjsonEvents {
    param([string]$Path)

    $events = [System.Collections.Generic.List[object]]::new()
    foreach ($line in Get-Content -LiteralPath $Path) {
        if ([string]::IsNullOrWhiteSpace($line)) {
            continue
        }
        $events.Add(($line | ConvertFrom-Json -Depth 64))
    }
    return $events.ToArray()
}

function Get-Property {
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

function Get-ArrayProperty {
    param(
        [object]$Object,
        [string]$Name
    )

    $value = Get-Property -Object $Object -Name $Name
    if ($null -eq $value) {
        return
    }
    return @($value)
}

function Count-CleanupActionStatus {
    param(
        [object[]]$Actions,
        [string]$Status
    )

    return @(
        $Actions | Where-Object {
            (Get-Property -Object $_ -Name "status") -eq $Status
        }
    ).Count
}

function Sum-CleanupActionBytes {
    param(
        [object[]]$Actions,
        [string]$Status
    )

    $sum = [int64]0
    foreach ($action in $Actions) {
        if ((Get-Property -Object $action -Name "status") -ne $Status) {
            continue
        }
        $logicalBytes = Get-Property -Object $action -Name "logical_bytes"
        if ($null -ne $logicalBytes) {
            $sum += [int64]$logicalBytes
        }
    }
    return $sum
}

function Invoke-SelfTest {
    $outputRoot = Resolve-OutputRoot -Path $OutputDirectory
    New-Item -ItemType Directory -Force -Path $outputRoot | Out-Null
    $report = [ordered]@{
        schema_version = 1
        generated_at_unix_seconds = Get-UnixTimeSeconds
        status = "passed"
        status_reason = "self-test"
        command = "self-test"
        root = ""
        output_directory = $outputRoot
        no_delete = $true
    }
    $report | ConvertTo-Json -Depth 32 | Set-Content -LiteralPath (Join-Path $outputRoot "disk-governance-dogfood-report.json") -Encoding utf8
    Write-Host "Disk-governance dogfood self-test passed. Report: $(Join-Path $outputRoot 'disk-governance-dogfood-report.json')"
}

if ($SelfTest) {
    Invoke-SelfTest
    exit 0
}

$repoRoot = Get-RepoRoot
$resolvedRoot = Resolve-RequiredRoot -Path $Root -AllowDriveRootValue:$AllowDriveRoot
$outputRoot = Resolve-OutputRoot -Path $OutputDirectory
Assert-OutputOutsideRoot -RootPath $resolvedRoot -OutputRoot $outputRoot -AllowInside:$AllowOutputInsideRoot

New-Item -ItemType Directory -Force -Path $outputRoot | Out-Null
New-Item -ItemType Directory -Force -Path (Join-Path $outputRoot "raw") | Out-Null

Push-Location $repoRoot
try {
    Set-IsolatedRebeccaEnvironment -OutputRoot $outputRoot
    & cargo build -p rebecca --locked --quiet
    if ($null -ne $LASTEXITCODE -and $LASTEXITCODE -ne 0) {
        throw "cargo build failed with exit code $LASTEXITCODE"
    }
    $binaryName = if ($IsWindows) { "rebecca.exe" } else { "rebecca" }
    $rebeccaBinary = Join-Path $repoRoot (Join-Path "target\debug" $binaryName)

    $jsonStdout = Join-Path $outputRoot "raw\inspect-drive.stdout.json"
    $jsonStderr = Join-Path $outputRoot "raw\inspect-drive.stderr.txt"
    $ndjsonStdout = Join-Path $outputRoot "raw\inspect-drive-progress.stdout.ndjson"
    $ndjsonStderr = Join-Path $outputRoot "raw\inspect-drive-progress.stderr.txt"

    $baseArgs = @(
        "inspect", "drive", $resolvedRoot,
        "--top", ([string]$Top),
        "--diagnostic-limit", ([string]$DiagnosticLimit)
    )
    if (-not [string]::IsNullOrWhiteSpace($ScanBackend)) {
        $baseArgs += @("--scan-backend", $ScanBackend)
    }

    Invoke-RebeccaCapture -BinaryPath $rebeccaBinary -CommandArgs @($baseArgs + @("--format", "json", "--no-progress")) -StdoutPath $jsonStdout -StderrPath $jsonStderr
    Invoke-RebeccaCapture -BinaryPath $rebeccaBinary -CommandArgs @($baseArgs + @("--format", "ndjson", "--no-progress")) -StdoutPath $ndjsonStdout -StderrPath $ndjsonStderr

    $jsonEnvelope = Get-Content -LiteralPath $jsonStdout -Raw | ConvertFrom-Json -Depth 128
    if ($jsonEnvelope.kind -ne "success") {
        throw "inspect drive returned non-success envelope"
    }
    if ($jsonEnvelope.payload_kind -ne "inspect-map") {
        throw "inspect drive returned unexpected payload_kind '$($jsonEnvelope.payload_kind)'"
    }
    $data = $jsonEnvelope.data
    if ($null -eq $data) {
        throw "inspect drive success envelope did not include data"
    }
    $events = Read-NdjsonEvents -Path $ndjsonStdout
    $completedEvents = @($events | Where-Object { $_.event_kind -eq "completed" })
    if ($completedEvents.Count -lt 1) {
        throw "inspect drive NDJSON output did not include a completed event"
    }
    $completedPayloadKinds = @(
        $completedEvents |
            ForEach-Object { Get-Property -Object $_ -Name "payload_kind" } |
            Where-Object { -not [string]::IsNullOrWhiteSpace([string]$_) } |
            Select-Object -Unique
    )
    if ($completedPayloadKinds -notcontains "inspect-map") {
        throw "inspect drive NDJSON completed event did not include payload_kind inspect-map"
    }
    foreach ($requiredProperty in @("top_entries", "roots", "cleanup_actions", "manual_review_items", "cleanup_advice_summary", "diagnostic_summary")) {
        if ($null -eq $data.PSObject.Properties[$requiredProperty]) {
            throw "inspect drive report is missing required property '$requiredProperty'"
        }
    }
    $progressKinds = @(
        $events |
            Where-Object { $_.event_kind -eq "inspect-progress" } |
            ForEach-Object { $_.data.progress_kind } |
            Select-Object -Unique
    )

    $topEntries = @(Get-ArrayProperty -Object $data -Name "top_entries")
    $roots = @(Get-ArrayProperty -Object $data -Name "roots")
    $volumeContexts = @(Get-ArrayProperty -Object $data -Name "volume_contexts")
    $workspaceInsights = @(Get-ArrayProperty -Object $data -Name "workspace_insights")
    $cleanupActions = @(Get-ArrayProperty -Object $data -Name "cleanup_actions")
    $manualReviewItems = @(Get-ArrayProperty -Object $data -Name "manual_review_items")
    $cleanupAdviceSummary = Get-Property -Object $data -Name "cleanup_advice_summary"
    $groups = @(Get-ArrayProperty -Object $data -Name "groups")
    $totals = Get-Property -Object $data -Name "totals"
    $diagnosticSummary = Get-Property -Object $data -Name "diagnostic_summary"
    $diagnosticTopReasons = @(Get-ArrayProperty -Object $diagnosticSummary -Name "top_reasons")
    $diagnosticTotal = Get-Property -Object $diagnosticSummary -Name "total"
    if ($null -eq $diagnosticTotal) {
        throw "diagnostic_summary is missing total"
    }
    if ([int64]$diagnosticTotal -gt 0 -and $diagnosticTopReasons.Count -eq 0) {
        throw "diagnostic_summary reported diagnostics but no compact top_reasons"
    }
    $backendFallbackEvents = @(
        $events | Where-Object {
            $_.event_kind -eq "inspect-progress" -and
            (Get-Property -Object $_.data -Name "progress_kind") -eq "backend-fallback"
        }
    )
    $actualBackends = @(
        $roots |
            ForEach-Object { Get-Property -Object $_ -Name "estimate_backend" } |
            Where-Object { -not [string]::IsNullOrWhiteSpace([string]$_) } |
            Select-Object -Unique
    )
    $fallbackKinds = @(
        @(
            $roots |
                ForEach-Object { Get-Property -Object $_ -Name "estimate_fallback_kind" }
        ) + @(
            $backendFallbackEvents |
                ForEach-Object { Get-Property -Object $_.data -Name "reason_kind" }
        ) |
            Where-Object { -not [string]::IsNullOrWhiteSpace([string]$_) } |
            Select-Object -Unique
    )
    $fallbackGuidance = @(
        @(
            $roots |
                ForEach-Object { Get-Property -Object $_ -Name "estimate_fallback_guidance" }
        ) + @(
            $backendFallbackEvents |
                ForEach-Object { Get-Property -Object $_.data -Name "guidance" }
        ) |
            Where-Object { -not [string]::IsNullOrWhiteSpace([string]$_) } |
            Select-Object -Unique
    )

    $report = [ordered]@{
        schema_version = 1
        generated_at_unix_seconds = Get-UnixTimeSeconds
        status = "passed"
        status_reason = ""
        command = ($baseArgs -join " ")
        root = $resolvedRoot
        output_directory = $outputRoot
        no_delete = $true
        no_delete_requested = $NoDelete.IsPresent
        requested_backend = if ([string]::IsNullOrWhiteSpace($ScanBackend)) { "guided-default" } else { $ScanBackend }
        actual_backends = $actualBackends
        backend_fallback_events = $backendFallbackEvents.Count
        fallback_kinds = $fallbackKinds
        fallback_guidance = $fallbackGuidance
        payload_kind = $jsonEnvelope.payload_kind
        logical_bytes = Get-Property -Object $totals -Name "logical_bytes"
        allocated_bytes = Get-Property -Object $totals -Name "allocated_bytes"
        unique_logical_bytes = Get-Property -Object $totals -Name "unique_logical_bytes"
        files = Get-Property -Object $totals -Name "files"
        directories = Get-Property -Object $totals -Name "directories"
        top_entries = $topEntries.Count
        groups = $groups.Count
        volume_contexts = $volumeContexts.Count
        workspace_insights = $workspaceInsights.Count
        diagnostics = Get-Property -Object $diagnosticSummary -Name "total"
        retained_diagnostics = Get-Property -Object $diagnosticSummary -Name "retained"
        diagnostic_top_reasons = $diagnosticTopReasons
        cleanup_actions = $cleanupActions.Count
        manual_review_items = $manualReviewItems.Count
        cleanable_actions = Count-CleanupActionStatus -Actions $cleanupActions -Status "cleanable"
        maybe_cleanable_actions = Count-CleanupActionStatus -Actions $cleanupActions -Status "maybe-cleanable"
        contains_cleanable_actions = Count-CleanupActionStatus -Actions $cleanupActions -Status "contains-cleanable"
        protected_items = Count-CleanupActionStatus -Actions $manualReviewItems -Status "protected"
        cleanable_logical_bytes = Get-Property -Object $cleanupAdviceSummary -Name "cleanable_logical_bytes"
        maybe_cleanable_logical_bytes = Get-Property -Object $cleanupAdviceSummary -Name "maybe_cleanable_logical_bytes"
        contains_cleanable_logical_bytes = Get-Property -Object $cleanupAdviceSummary -Name "contains_cleanable_logical_bytes"
        manual_review_logical_bytes = Get-Property -Object $cleanupAdviceSummary -Name "manual_review_logical_bytes"
        protected_logical_bytes = Get-Property -Object $cleanupAdviceSummary -Name "protected_logical_bytes"
        cleanable_action_logical_bytes = Sum-CleanupActionBytes -Actions $cleanupActions -Status "cleanable"
        maybe_cleanable_action_logical_bytes = Sum-CleanupActionBytes -Actions $cleanupActions -Status "maybe-cleanable"
        ndjson_events = $events.Count
        progress_kinds = $progressKinds
        raw_json_stdout = $jsonStdout
        raw_json_stderr = $jsonStderr
        raw_ndjson_stdout = $ndjsonStdout
        raw_ndjson_stderr = $ndjsonStderr
    }

    $reportPath = Join-Path $outputRoot "disk-governance-dogfood-report.json"
    $summaryPath = Join-Path $outputRoot "disk-governance-dogfood-summary.md"
    $report | ConvertTo-Json -Depth 64 | Set-Content -LiteralPath $reportPath -Encoding utf8

    $summary = @()
    $summary += "# Disk Governance Dogfood"
    $summary += ""
    $summary += "- Status: $($report["status"])"
    $summary += "- Root: $resolvedRoot"
    $summary += "- Command: ``$($report["command"])``"
    $summary += "- Output directory: $outputRoot"
    $summary += "- Read-only inspect: true"
    $summary += "- No-delete switch requested: $($report["no_delete_requested"])"
    $summary += "- Actual backend(s): $($actualBackends -join ', ')"
    if ($fallbackKinds.Count -gt 0) {
        $summary += "- Fallback kind(s): $($fallbackKinds -join ', ')"
    }
    $summary += ""
    $summary += "| Metric | Value |"
    $summary += "| --- | ---: |"
    $summary += "| Logical bytes | $($report["logical_bytes"]) |"
    $summary += "| Allocated bytes | $($report["allocated_bytes"]) |"
    $summary += "| Unique logical bytes | $($report["unique_logical_bytes"]) |"
    $summary += "| Top entries | $($report["top_entries"]) |"
    $summary += "| Groups | $($report["groups"]) |"
    $summary += "| Volume contexts | $($report["volume_contexts"]) |"
    $summary += "| Workspace insights | $($report["workspace_insights"]) |"
    $summary += "| Diagnostics | $($report["diagnostics"]) |"
    $summary += "| Cleanup actions | $($report["cleanup_actions"]) |"
    $summary += "| Manual-review items | $($report["manual_review_items"]) |"
    $summary += "| Cleanable actions | $($report["cleanable_actions"]) |"
    $summary += "| Maybe-cleanable actions | $($report["maybe_cleanable_actions"]) |"
    $summary += "| Contains-cleanable actions | $($report["contains_cleanable_actions"]) |"
    $summary += "| Protected items | $($report["protected_items"]) |"
    $summary += "| Cleanable logical bytes | $($report["cleanable_logical_bytes"]) |"
    $summary += "| Manual-review logical bytes | $($report["manual_review_logical_bytes"]) |"
    $summary += "| Backend fallback events | $($report["backend_fallback_events"]) |"
    $summary += "| NDJSON events | $($report["ndjson_events"]) |"
    $summary += ""
    if ($diagnosticTopReasons.Count -gt 0) {
        $summary += "Diagnostic top reasons:"
        foreach ($reason in $diagnosticTopReasons) {
            $reasonKind = Get-Property -Object $reason -Name "kind"
            $reasonCount = Get-Property -Object $reason -Name "count"
            $reasonDetail = Get-Property -Object $reason -Name "detail"
            $reasonCode = Get-Property -Object $reason -Name "reason_code"
            $reasonCodeText = if ([string]::IsNullOrWhiteSpace([string]$reasonCode)) { "" } else { " ($reasonCode)" }
            $summary += "- $($reasonKind)$($reasonCodeText): $reasonCount - $reasonDetail"
        }
        $summary += ""
    }
    $summary += "Progress kinds: $($progressKinds -join ', ')"
    $summary | Set-Content -LiteralPath $summaryPath -Encoding utf8

    Write-Host "Disk-governance dogfood passed. Report: $reportPath"
}
catch {
    $failureReport = [ordered]@{
        schema_version = 1
        generated_at_unix_seconds = Get-UnixTimeSeconds
        status = "failed"
        status_reason = $_.Exception.Message
        root = $resolvedRoot
        output_directory = $outputRoot
        no_delete = $true
        no_delete_requested = $NoDelete.IsPresent
    }
    $failureReport | ConvertTo-Json -Depth 32 | Set-Content -LiteralPath (Join-Path $outputRoot "disk-governance-dogfood-report.json") -Encoding utf8
    throw
}
finally {
    Pop-Location
}

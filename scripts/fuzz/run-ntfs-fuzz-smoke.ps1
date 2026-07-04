param(
    [string[]]$Target = @("mft_record", "attribute_list", "i30_index", "runlist"),
    [int]$SecondsPerTarget = 10,
    [string]$OutputDirectory = ""
)

$ErrorActionPreference = "Stop"

function Resolve-RepoRoot {
    $scriptRoot = Split-Path -Parent $PSCommandPath
    return (Resolve-Path (Join-Path $scriptRoot "..\..")).Path
}

function New-Result {
    param(
        [string]$Phase,
        [string]$TargetName,
        [string]$Status,
        [int]$ExitCode,
        [int64]$DurationMs,
        [string]$LogPath,
        [string]$SkippedReason = ""
    )

    [pscustomobject]@{
        phase = $Phase
        target = $TargetName
        status = $Status
        exit_code = $ExitCode
        duration_ms = $DurationMs
        log_path = $LogPath
        skipped_reason = $SkippedReason
    }
}

function Invoke-LoggedCommand {
    param(
        [string]$Phase,
        [string]$TargetName,
        [string]$LogPath,
        [scriptblock]$Command
    )

    $started = Get-Date
    & $Command *> $LogPath
    $exitCode = if ($null -eq $LASTEXITCODE) { 0 } else { $LASTEXITCODE }
    $durationMs = [int64]((Get-Date) - $started).TotalMilliseconds
    $status = if ($exitCode -eq 0) { "passed" } else { "failed" }
    New-Result -Phase $Phase -TargetName $TargetName -Status $status -ExitCode $exitCode -DurationMs $durationMs -LogPath $LogPath
}

$repoRoot = Resolve-RepoRoot
$manifestPath = Join-Path $repoRoot "crates\rebecca-ntfs\fuzz\Cargo.toml"
$timestamp = Get-Date -Format "yyyyMMdd-HHmmss"
if ([string]::IsNullOrWhiteSpace($OutputDirectory)) {
    $OutputDirectory = Join-Path $repoRoot "target\fuzz-smoke\$timestamp"
}
$outputRoot = (New-Item -ItemType Directory -Force -Path $OutputDirectory).FullName

$results = @()
$checkLog = Join-Path $outputRoot "cargo-check.log"
$results += Invoke-LoggedCommand -Phase "cargo-check" -TargetName "all" -LogPath $checkLog -Command {
    cargo check --manifest-path $manifestPath --bins
}

$cargoFuzz = Get-Command "cargo-fuzz" -ErrorAction SilentlyContinue
if ($null -eq $cargoFuzz) {
    foreach ($targetName in $Target) {
        $results += New-Result `
            -Phase "cargo-fuzz-run" `
            -TargetName $targetName `
            -Status "skipped" `
            -ExitCode 0 `
            -DurationMs 0 `
            -LogPath "" `
            -SkippedReason "cargo-fuzz not found on PATH"
    }
} else {
    foreach ($targetName in $Target) {
        $safeTarget = $targetName -replace "[^A-Za-z0-9_.-]", "_"
        $logPath = Join-Path $outputRoot "$safeTarget.fuzz.log"
        $results += Invoke-LoggedCommand -Phase "cargo-fuzz-run" -TargetName $targetName -LogPath $logPath -Command {
            cargo fuzz run $targetName --manifest-path $manifestPath -- -max_total_time=$SecondsPerTarget
        }
    }
}

$failed = @($results | Where-Object { $_.status -eq "failed" })
$report = [pscustomobject]@{
    generated_at = (Get-Date).ToString("o")
    repo_root = $repoRoot
    manifest_path = $manifestPath
    seconds_per_target = $SecondsPerTarget
    output_directory = $outputRoot
    cargo_fuzz_available = ($null -ne $cargoFuzz)
    results = $results
    failed_count = $failed.Count
}

$jsonPath = Join-Path $outputRoot "fuzz-smoke-report.json"
$markdownPath = Join-Path $outputRoot "fuzz-smoke-summary.md"
$report | ConvertTo-Json -Depth 8 | Set-Content -Path $jsonPath -Encoding utf8

$markdown = @()
$markdown += "# NTFS Fuzz Smoke"
$markdown += ""
$markdown += "- Generated: $($report.generated_at)"
$markdown += "- Manifest: ``$manifestPath``"
$markdown += "- cargo-fuzz available: $($report.cargo_fuzz_available)"
$markdown += "- Seconds per target: $SecondsPerTarget"
$markdown += "- Failed phases: $($failed.Count)"
$markdown += ""
$markdown += "| Phase | Target | Status | Exit | Duration ms | Skipped reason |"
$markdown += "|---|---|---:|---:|---:|---|"
foreach ($result in $results) {
    $markdown += "| $($result.phase) | $($result.target) | $($result.status) | $($result.exit_code) | $($result.duration_ms) | $($result.skipped_reason) |"
}
$markdown | Set-Content -Path $markdownPath -Encoding utf8

Write-Host "Wrote fuzz smoke report: $jsonPath"
Write-Host "Wrote fuzz smoke summary: $markdownPath"

if ($failed.Count -gt 0) {
    exit 1
}

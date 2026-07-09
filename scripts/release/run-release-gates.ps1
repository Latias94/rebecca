param(
    [string]$OutputDirectory = "",
    [string]$DogfoodRoot = "docs\plans",
    [string]$BenchmarkBaselinePath = "",
    [ValidateSet("skip", "smoke", "full")]
    [string]$Benchmark = "smoke",
    [ValidateSet("skip", "self-test", "stable", "all")]
    [string]$Dogfood = "stable",
    [switch]$SkipWorkspaceTests,
    [switch]$SkipClippy,
    [switch]$SkipDeny,
    [switch]$AllowDogfoodMismatch,
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

function Resolve-OutputDirectory {
    param(
        [string]$RepoRoot,
        [string]$Path
    )

    if ([string]::IsNullOrWhiteSpace($Path)) {
        return Join-Path $RepoRoot (Join-Path "target\release-gates" (Get-TimestampId))
    }
    if ([System.IO.Path]::IsPathRooted($Path)) {
        return [System.IO.Path]::GetFullPath($Path)
    }
    return [System.IO.Path]::GetFullPath((Join-Path (Get-Location).ProviderPath $Path))
}

function New-GateReport {
    param(
        [string]$RepoRoot,
        [string]$OutputRoot
    )

    $commit = (& git -C $RepoRoot rev-parse --short HEAD 2>$null)
    if ($LASTEXITCODE -ne 0) {
        $commit = ""
    }
    $branch = (& git -C $RepoRoot branch --show-current 2>$null)
    if ($LASTEXITCODE -ne 0) {
        $branch = ""
    }

    return [ordered]@{
        schema_version = 1
        generated_at_unix_seconds = Get-UnixTimeSeconds
        status = "running"
        status_reason = ""
        repo_root = $RepoRoot
        git_branch = [string]$branch
        git_commit = [string]$commit
        output_directory = $OutputRoot
        checks = @()
    }
}

function Save-GateReport {
    param(
        [System.Collections.IDictionary]$Report,
        [string]$OutputRoot
    )

    New-Item -ItemType Directory -Force -Path $OutputRoot | Out-Null
    $jsonPath = Join-Path $OutputRoot "release-gates-report.json"
    $Report | ConvertTo-Json -Depth 32 | Set-Content -LiteralPath $jsonPath -Encoding utf8

    $lines = @()
    $lines += "# Rebecca Release Gates"
    $lines += ""
    $lines += "- Status: $($Report["status"])"
    if (-not [string]::IsNullOrWhiteSpace($Report["status_reason"])) {
        $lines += "- Status reason: $($Report["status_reason"])"
    }
    $lines += "- Git branch: $($Report["git_branch"])"
    $lines += "- Git commit: $($Report["git_commit"])"
    $lines += "- Output directory: $($Report["output_directory"])"
    $lines += ""
    $lines += "| Check | Status | Exit | Duration ms |"
    $lines += "| --- | --- | ---: | ---: |"
    foreach ($check in @($Report["checks"])) {
        $lines += "| $($check.name) | $($check.status) | $($check.exit_code) | $($check.duration_ms) |"
    }
    $lines | Set-Content -LiteralPath (Join-Path $OutputRoot "release-gates-summary.md") -Encoding utf8
}

function Get-SafeName {
    param([string]$Name)

    return ($Name.ToLowerInvariant() -replace "[^a-z0-9._-]+", "-").Trim("-")
}

function Invoke-GateCommand {
    param(
        [System.Collections.IDictionary]$Report,
        [string]$OutputRoot,
        [string]$Name,
        [string[]]$Command,
        [switch]$AllowFailure
    )

    $safeName = Get-SafeName -Name $Name
    $stdoutPath = Join-Path $OutputRoot "$safeName.stdout.txt"
    $stderrPath = Join-Path $OutputRoot "$safeName.stderr.txt"
    $started = Get-Date
    $exitCode = 0
    $status = "passed"
    $reason = ""

    try {
        & $Command[0] @($Command | Select-Object -Skip 1) > $stdoutPath 2> $stderrPath
        if ($null -ne $LASTEXITCODE) {
            $exitCode = [int]$LASTEXITCODE
        }
        if ($exitCode -ne 0) {
            $status = if ($AllowFailure) { "allowed-failure" } else { "failed" }
            $reason = "Command exited with code $exitCode"
        }
    }
    catch {
        $exitCode = 1
        $status = if ($AllowFailure) { "allowed-failure" } else { "failed" }
        $reason = $_.Exception.Message
        $reason | Set-Content -LiteralPath $stderrPath -Encoding utf8
    }

    $durationMs = [int64]((Get-Date) - $started).TotalMilliseconds
    $result = [ordered]@{
        name = $Name
        status = $status
        status_reason = $reason
        command = $Command -join " "
        exit_code = $exitCode
        duration_ms = $durationMs
        stdout_path = $stdoutPath
        stderr_path = $stderrPath
    }
    $Report["checks"] = @($Report["checks"]) + @($result)
    Save-GateReport -Report $Report -OutputRoot $OutputRoot

    if ($status -eq "failed") {
        throw "$Name failed. See $stderrPath"
    }
    return $result
}

function Invoke-JsonGate {
    param(
        [System.Collections.IDictionary]$Report,
        [string]$OutputRoot,
        [string]$Name,
        [string[]]$Command,
        [string]$PayloadPath
    )

    $result = Invoke-GateCommand -Report $Report -OutputRoot $OutputRoot -Name $Name -Command $Command
    Copy-Item -LiteralPath $result.stdout_path -Destination $PayloadPath -Force
    $payload = Get-Content -LiteralPath $PayloadPath -Raw | ConvertFrom-Json
    if ($payload.kind -ne "success") {
        throw "$Name returned non-success machine envelope"
    }
    return $payload
}

function Invoke-SelfTest {
    $repoRoot = Get-RepoRoot
    $outputRoot = Resolve-OutputDirectory -RepoRoot $repoRoot -Path $OutputDirectory
    New-Item -ItemType Directory -Force -Path $outputRoot | Out-Null
    $report = New-GateReport -RepoRoot $repoRoot -OutputRoot $outputRoot

    Push-Location $repoRoot
    try {
        Invoke-GateCommand -Report $report -OutputRoot $outputRoot -Name "release gate self-test" -Command @(
            "pwsh", "-NoProfile", "-Command", "Write-Output 'release gate self-test ok'"
        ) | Out-Null
        $report.status = "passed"
        Save-GateReport -Report $report -OutputRoot $outputRoot
        Write-Host "Release gate self-test passed. Report: $(Join-Path $outputRoot 'release-gates-report.json')"
    }
    finally {
        Pop-Location
    }
}

if ($SelfTest) {
    Invoke-SelfTest
    exit 0
}

$repoRoot = Get-RepoRoot
$outputRoot = Resolve-OutputDirectory -RepoRoot $repoRoot -Path $OutputDirectory
New-Item -ItemType Directory -Force -Path $outputRoot | Out-Null
$report = New-GateReport -RepoRoot $repoRoot -OutputRoot $outputRoot

Push-Location $repoRoot
try {
    Invoke-GateCommand -Report $report -OutputRoot $outputRoot -Name "format check" -Command @(
        "cargo", "fmt", "--all", "--", "--check"
    ) | Out-Null

    if (-not $SkipClippy) {
        Invoke-GateCommand -Report $report -OutputRoot $outputRoot -Name "clippy workspace" -Command @(
            "cargo", "clippy", "--workspace", "--all-targets", "--", "-D", "warnings"
        ) | Out-Null
    }

    if (-not $SkipWorkspaceTests) {
        Invoke-GateCommand -Report $report -OutputRoot $outputRoot -Name "workspace tests" -Command @(
            "cargo", "nextest", "run", "--workspace", "--locked"
        ) | Out-Null
    }

    if (-not $SkipDeny) {
        Invoke-GateCommand -Report $report -OutputRoot $outputRoot -Name "dependency policy" -Command @(
            "cargo", "deny", "check"
        ) | Out-Null
    }

    Invoke-JsonGate -Report $report -OutputRoot $outputRoot -Name "catalog validate" -PayloadPath (Join-Path $outputRoot "catalog-validate.json") -Command @(
        "cargo", "run", "-p", "rebecca", "--locked", "--quiet", "--", "catalog", "validate", "--format", "json"
    ) | Out-Null

    Invoke-JsonGate -Report $report -OutputRoot $outputRoot -Name "cache inspect" -PayloadPath (Join-Path $outputRoot "cache-inspect.json") -Command @(
        "cargo", "run", "-p", "rebecca", "--locked", "--quiet", "--", "cache", "inspect", "--format", "json", "--namespace", "all"
    ) | Out-Null

    Invoke-JsonGate -Report $report -OutputRoot $outputRoot -Name "dry-run cleanup" -PayloadPath (Join-Path $outputRoot "clean-dry-run.json") -Command @(
        "cargo", "run", "-p", "rebecca", "--locked", "--quiet", "--", "clean", "--dry-run", "--no-progress", "--format", "json", "--rule", "windows.user-temp"
    ) | Out-Null

    Invoke-GateCommand -Report $report -OutputRoot $outputRoot -Name "benchmark comparator self-test" -Command @(
        "pwsh", "-File", "scripts\perf\compare-benchmark-matrix.ps1", "-SelfTest"
    ) | Out-Null

    if ($Benchmark -eq "smoke") {
        Invoke-GateCommand -Report $report -OutputRoot $outputRoot -Name "benchmark matrix smoke" -Command @(
            "pwsh", "-File", "scripts\perf\run-benchmark-matrix.ps1", "-SkipRun", "-OutputDirectory", (Join-Path $outputRoot "perf")
        ) | Out-Null
    }
    elseif ($Benchmark -eq "full") {
        $benchmarkCommand = @(
            "pwsh", "-File", "scripts\perf\run-benchmark-matrix.ps1", "-OutputDirectory", (Join-Path $outputRoot "perf")
        )
        if (-not [string]::IsNullOrWhiteSpace($BenchmarkBaselinePath)) {
            $benchmarkCommand += @("-BaselinePath", $BenchmarkBaselinePath)
        }
        Invoke-GateCommand -Report $report -OutputRoot $outputRoot -Name "benchmark matrix full" -Command $benchmarkCommand | Out-Null
    }

    if ($Dogfood -eq "self-test") {
        Invoke-GateCommand -Report $report -OutputRoot $outputRoot -Name "inspect-map dogfood self-test" -Command @(
            "pwsh", "-File", "scripts\dogfood\run-inspect-map-report.ps1", "-SelfTest"
        ) | Out-Null
    }
    elseif ($Dogfood -eq "stable" -or $Dogfood -eq "all") {
        Invoke-GateCommand -Report $report -OutputRoot $outputRoot -Name "disk governance dogfood" -Command @(
            "pwsh", "-File", "scripts\dogfood\run-disk-governance-dogfood.ps1",
            "-Root", $DogfoodRoot,
            "-Top", "20",
            "-DiagnosticLimit", "0",
            "-OutputDirectory", (Join-Path $outputRoot "disk-governance-dogfood"),
            "-NoDelete"
        ) | Out-Null

        $backends = if ($Dogfood -eq "all") {
            "portable-recursive,windows-native,windows-ntfs-mft-experimental"
        }
        else {
            "portable-recursive,windows-native"
        }
        $dogfoodCommand = @(
            "pwsh", "-File", "scripts\dogfood\run-inspect-map-report.ps1",
            "-Root", $DogfoodRoot,
            "-Backend", $backends,
            "-Repeat", "1",
            "-Top", "20",
            "-GroupBy", "extension,depth,age",
            "-DiagnosticLimit", "0",
            "-OutputDirectory", (Join-Path $outputRoot "inspect-map-dogfood")
        )
        if ($AllowDogfoodMismatch) {
            $dogfoodCommand += "-AllowMismatch"
        }
        Invoke-GateCommand -Report $report -OutputRoot $outputRoot -Name "inspect-map dogfood" -Command $dogfoodCommand | Out-Null
    }

    $report.status = "passed"
    Save-GateReport -Report $report -OutputRoot $outputRoot
    Write-Host "Release gates passed. Report: $(Join-Path $outputRoot 'release-gates-report.json')"
}
catch {
    $report.status = "failed"
    $report.status_reason = $_.Exception.Message
    Save-GateReport -Report $report -OutputRoot $outputRoot
    throw
}
finally {
    Pop-Location
}

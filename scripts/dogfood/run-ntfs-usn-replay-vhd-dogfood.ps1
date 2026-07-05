param(
    [string]$OutputDirectory = "",
    [string]$VhdPath = "",
    [int]$VhdSizeMB = 256,
    [int]$Top = 20,
    [int]$DiagnosticLimit = 0,
    [int]$TimeoutSeconds = 180,
    [int]$IndexTimeoutSeconds = 30,
    [switch]$AllowMismatch,
    [switch]$KeepMounted,
    [switch]$RemoveVhd,
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

function Get-NormalizedPathForComparison {
    param([string]$Path)

    $full = [IO.Path]::GetFullPath($Path).TrimEnd('\', '/')
    if ([string]::IsNullOrWhiteSpace($full)) {
        return $full
    }
    $root = [IO.Path]::GetPathRoot($full)
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
    if ($childFull.Equals($parentFull, [StringComparison]::OrdinalIgnoreCase)) {
        return $true
    }
    $parentWithSeparator = $parentFull.TrimEnd('\', '/') + [IO.Path]::DirectorySeparatorChar
    return $childFull.StartsWith($parentWithSeparator, [StringComparison]::OrdinalIgnoreCase)
}

function Assert-NotDriveRoot {
    param([string]$Path)

    $full = [IO.Path]::GetFullPath($Path).TrimEnd('\', '/')
    $driveRoot = [IO.Path]::GetPathRoot($full)
    if (-not [string]::IsNullOrWhiteSpace($driveRoot)) {
        $driveRoot = $driveRoot.TrimEnd('\', '/')
    }
    if ($full -eq $driveRoot) {
        throw "Refusing to use a drive root for dogfood output or VHD paths: $Path"
    }
}

function Assert-NewFilePath {
    param([string]$Path)

    Assert-NotDriveRoot -Path $Path
    if (Test-Path -LiteralPath $Path) {
        throw "Refusing to overwrite existing VHD path: $Path"
    }
}

function Assert-RemovableVhdPath {
    param(
        [string]$RunRoot,
        [string]$Path
    )

    if (-not (Test-PathSameOrChild -Parent $RunRoot -Child $Path)) {
        throw "Refusing to remove VHD outside the dogfood output directory: $Path"
    }
}

function Get-FreeDriveLetter {
    $used = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($drive in [IO.DriveInfo]::GetDrives()) {
        if ($drive.Name.Length -ge 1) {
            [void]$used.Add($drive.Name.Substring(0, 1))
        }
    }

    foreach ($letter in @("Z", "Y", "X", "W", "V", "U", "T", "S", "R", "Q", "P", "O", "N", "M", "L", "K", "J", "I", "H", "G", "F")) {
        if (-not $used.Contains($letter)) {
            return $letter
        }
    }
    throw "No free drive letter is available for the dogfood VHD."
}

function Get-DiskPartCreateCommands {
    param(
        [string]$Path,
        [int]$SizeMegabytes,
        [string]$DriveLetter,
        [string]$Label
    )

    return @(
        ('create vdisk file="' + $Path + '" maximum=' + $SizeMegabytes + ' type=expandable'),
        ('select vdisk file="' + $Path + '"'),
        "attach vdisk",
        "create partition primary",
        ('format fs=ntfs quick label="' + $Label + '"'),
        ('assign letter=' + $DriveLetter)
    )
}

function Get-DiskPartDetachCommands {
    param([string]$Path)

    return @(
        ('select vdisk file="' + $Path + '"'),
        "detach vdisk"
    )
}

function Join-DriveRootChild {
    param(
        [string]$DriveRoot,
        [string]$Child
    )

    return $DriveRoot.TrimEnd('\', '/') + "\" + $Child.TrimStart('\', '/')
}

function Invoke-DiskPartScript {
    param(
        [string[]]$Commands,
        [string]$ScriptPath,
        [string]$LogPath
    )

    [IO.File]::WriteAllLines($ScriptPath, $Commands, [Text.Encoding]::ASCII)
    $output = & diskpart.exe /s $ScriptPath 2>&1
    $lines = @($output | ForEach-Object { [string]$_ })
    [IO.File]::WriteAllLines($LogPath, $lines, [Text.Encoding]::UTF8)
    $joined = $lines -join [Environment]::NewLine
    if ($LASTEXITCODE -ne 0 -or $joined -match "DiskPart has encountered an error") {
        throw "diskpart failed; see $LogPath"
    }
}

function Invoke-FsutilUsnCreateJournal {
    param(
        [string]$DriveRoot,
        [string]$LogPath
    )

    $drive = $DriveRoot.TrimEnd('\', '/')
    $output = & fsutil.exe usn createjournal m=1048576 a=65536 $drive 2>&1
    $lines = @($output | ForEach-Object { [string]$_ })
    [IO.File]::WriteAllLines($LogPath, $lines, [Text.Encoding]::UTF8)
    if ($LASTEXITCODE -ne 0) {
        throw "fsutil USN journal creation failed; see $LogPath"
    }
}

function Invoke-UsnDogfood {
    param(
        [string]$FixtureRoot,
        [string]$ReportRoot,
        [int]$TopLimit,
        [int]$DiagnosticLimitValue,
        [int]$TimeoutSecondsValue,
        [int]$IndexTimeoutSecondsValue,
        [bool]$AllowMismatchValue
    )

    $script = Join-Path $PSScriptRoot "run-ntfs-usn-replay-dogfood.ps1"
    $arguments = @(
        "-NoProfile",
        "-File",
        $script,
        "-FixtureRoot",
        $FixtureRoot,
        "-OutputDirectory",
        $ReportRoot,
        "-Top",
        [string]$TopLimit,
        "-DiagnosticLimit",
        [string]$DiagnosticLimitValue,
        "-TimeoutSeconds",
        [string]$TimeoutSecondsValue,
        "-IndexTimeoutSeconds",
        [string]$IndexTimeoutSecondsValue
    )
    if ($AllowMismatchValue) {
        $arguments += "-AllowMismatch"
    }

    $output = & pwsh @arguments 2>&1
    $exitCode = $LASTEXITCODE
    foreach ($line in @($output)) {
        if ($line -is [System.Management.Automation.ErrorRecord]) {
            [Console]::Error.WriteLine($line.ToString())
        }
        else {
            Write-Host ([string]$line)
        }
    }
    return [int]$exitCode
}

function New-MarkdownSummary {
    param([object]$Report)

    $lines = [System.Collections.Generic.List[string]]::new()
    $lines.Add("# NTFS USN Replay VHD Dogfood")
    $lines.Add("")
    $lines.Add("- Generated: $($Report.generated_at_utc)")
    $lines.Add("- Commit: $($Report.git_commit)")
    $lines.Add("- VHD: $($Report.vhd_path)")
    $lines.Add("- Mounted drive: $($Report.drive_root)")
    $lines.Add("- Dogfood report: $($Report.dogfood_output_directory)")
    $lines.Add("- Dogfood exit code: $($Report.dogfood_exit_code)")
    $lines.Add("- Kept mounted: $($Report.keep_mounted)")
    $lines.Add("- Removed VHD: $($Report.removed_vhd)")
    $lines.Add("")
    return ($lines -join [Environment]::NewLine)
}

function Invoke-SelfTest {
    $commands = Get-DiskPartCreateCommands -Path "C:\tmp\scratch.vhdx" -SizeMegabytes 256 -DriveLetter "Z" -Label "REBECCA_TEST"
    if ($commands[0] -ne 'create vdisk file="C:\tmp\scratch.vhdx" maximum=256 type=expandable') {
        throw "self-test create command failed"
    }
    if ($commands[-1] -ne "assign letter=Z") {
        throw "self-test drive-letter command failed"
    }
    $detach = Get-DiskPartDetachCommands -Path "C:\tmp\scratch.vhdx"
    if ($detach.Count -ne 2 -or $detach[1] -ne "detach vdisk") {
        throw "self-test detach command failed"
    }
    if ((Join-DriveRootChild -DriveRoot "Z:\" -Child "fixture") -ne "Z:\fixture") {
        throw "self-test drive-root path join failed"
    }
    if (-not (Test-PathSameOrChild -Parent "C:\a\b" -Child "C:\a\b\c\d.vhdx")) {
        throw "self-test path child detection failed"
    }
    if (Test-PathSameOrChild -Parent "C:\a\b" -Child "C:\a\other\d.vhdx") {
        throw "self-test path child rejection failed"
    }
    $markdown = New-MarkdownSummary -Report ([pscustomobject]@{
        generated_at_utc = "1970-01-01T00:00:00Z"
        git_commit = "selftest"
        vhd_path = "C:\tmp\scratch.vhdx"
        drive_root = "Z:\"
        dogfood_output_directory = "C:\tmp\report"
        dogfood_exit_code = 0
        keep_mounted = $false
        removed_vhd = $false
    })
    if ($markdown -notlike "*NTFS USN Replay VHD Dogfood*" -or $markdown -notlike "*scratch.vhdx*") {
        throw "self-test markdown failed"
    }

    Write-Host "Self-test passed."
}

if ($SelfTest) {
    Invoke-SelfTest
    exit 0
}

if ($VhdSizeMB -lt 128) {
    throw "VhdSizeMB must be at least 128."
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
$runRoot = Resolve-UnderRepoPath -Path $OutputDirectory -DefaultRelativeRoot "target\ntfs-usn-replay-vhd-dogfood"
Assert-NotDriveRoot -Path $runRoot
New-Item -ItemType Directory -Force -Path $runRoot | Out-Null

$vhdFile = if ([string]::IsNullOrWhiteSpace($VhdPath)) {
    Join-Path $runRoot "scratch.vhdx"
}
elseif ([IO.Path]::IsPathRooted($VhdPath)) {
    [IO.Path]::GetFullPath($VhdPath)
}
else {
    [IO.Path]::GetFullPath((Join-Path (Get-Location).ProviderPath $VhdPath))
}
Assert-NewFilePath -Path $vhdFile
New-Item -ItemType Directory -Force -Path (Split-Path -Parent $vhdFile) | Out-Null

$driveLetter = Get-FreeDriveLetter
$driveRoot = "${driveLetter}:\"
$label = ("REB" + (Get-Date).ToUniversalTime().ToString("MMddHHmmss"))
$createScriptPath = Join-Path $runRoot "diskpart-create.txt"
$createLogPath = Join-Path $runRoot "diskpart-create.log"
$detachScriptPath = Join-Path $runRoot "diskpart-detach.txt"
$detachLogPath = Join-Path $runRoot "diskpart-detach.log"
$usnJournalLogPath = Join-Path $runRoot "fsutil-usn-createjournal.log"
$dogfoodReportRoot = Join-Path $runRoot "dogfood"
$fixtureRoot = Join-DriveRootChild -DriveRoot $driveRoot -Child "rebecca-usn-fixture"
$mounted = $false
$detached = $false
$dogfoodExitCode = -1
$dogfoodError = $null
$removedVhd = $false

try {
    Invoke-DiskPartScript -Commands (Get-DiskPartCreateCommands -Path $vhdFile -SizeMegabytes $VhdSizeMB -DriveLetter $driveLetter -Label $label) -ScriptPath $createScriptPath -LogPath $createLogPath
    $mounted = $true
    if (-not (Test-Path -LiteralPath $driveRoot)) {
        throw "VHD mounted but assigned drive root is not available: $driveRoot"
    }
    Invoke-FsutilUsnCreateJournal -DriveRoot $driveRoot -LogPath $usnJournalLogPath

    $dogfoodExitCode = Invoke-UsnDogfood -FixtureRoot $fixtureRoot -ReportRoot $dogfoodReportRoot -TopLimit $Top -DiagnosticLimitValue $DiagnosticLimit -TimeoutSecondsValue $TimeoutSeconds -IndexTimeoutSecondsValue $IndexTimeoutSeconds -AllowMismatchValue ([bool]$AllowMismatch)
    if ($dogfoodExitCode -ne 0) {
        $dogfoodError = "USN replay dogfood exited with code $dogfoodExitCode"
    }
}
catch {
    $dogfoodError = $_.Exception.Message
    if ($dogfoodExitCode -eq -1) {
        $dogfoodExitCode = 1
    }
}
finally {
    if ($mounted -and -not $KeepMounted) {
        try {
            Invoke-DiskPartScript -Commands (Get-DiskPartDetachCommands -Path $vhdFile) -ScriptPath $detachScriptPath -LogPath $detachLogPath
            $detached = $true
        }
        catch {
            [Console]::Error.WriteLine("VHD detach failed: $($_.Exception.Message)")
        }
    }
    if ($RemoveVhd) {
        if (-not $detached -and -not $KeepMounted) {
            [Console]::Error.WriteLine("VHD removal skipped because detach was not confirmed: $vhdFile")
        }
        elseif ($KeepMounted) {
            [Console]::Error.WriteLine("VHD removal skipped because -KeepMounted was used: $vhdFile")
        }
        elseif (Test-Path -LiteralPath $vhdFile) {
            Assert-RemovableVhdPath -RunRoot $runRoot -Path $vhdFile
            Remove-Item -LiteralPath $vhdFile -Force
            $removedVhd = $true
        }
    }
}

$report = [pscustomobject]@{
    schema_version = 1
    generated_at_utc = [DateTimeOffset]::UtcNow.ToString("O")
    git_commit = (& git -C $repoRoot rev-parse --short HEAD 2>$null)
    repo_root = $repoRoot
    output_directory = $runRoot
    vhd_path = $vhdFile
    vhd_size_mb = $VhdSizeMB
    volume_label = $label
    drive_letter = $driveLetter
    drive_root = $driveRoot
    fixture_root = $fixtureRoot
    dogfood_output_directory = $dogfoodReportRoot
    dogfood_exit_code = $dogfoodExitCode
    dogfood_error = $dogfoodError
    keep_mounted = [bool]$KeepMounted
    mounted = $mounted
    detached = $detached
    remove_vhd_requested = [bool]$RemoveVhd
    removed_vhd = $removedVhd
    diskpart_create_log = $createLogPath
    diskpart_detach_log = $detachLogPath
    fsutil_usn_createjournal_log = $usnJournalLogPath
}

$reportPath = Join-Path $runRoot "ntfs-usn-replay-vhd-report.json"
$summaryPath = Join-Path $runRoot "ntfs-usn-replay-vhd-summary.md"
$report | ConvertTo-Json -Depth 32 | Set-Content -LiteralPath $reportPath -Encoding utf8
New-MarkdownSummary -Report $report | Set-Content -LiteralPath $summaryPath -Encoding utf8

Write-Host "NTFS USN replay VHD dogfood report written to $runRoot"
Write-Host "  JSON: $reportPath"
Write-Host "  Summary: $summaryPath"
Write-Host "  Dogfood: $dogfoodReportRoot"
Write-Host "  VHD: $vhdFile"

if ($dogfoodExitCode -ne 0) {
    [Console]::Error.WriteLine($dogfoodError)
    exit $dogfoodExitCode
}

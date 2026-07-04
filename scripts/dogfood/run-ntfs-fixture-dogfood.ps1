param(
    [string]$FixtureRoot = "",
    [string]$OutputDirectory = "",
    [string[]]$Backend = @("portable-recursive", "windows-native", "windows-ntfs-mft-experimental"),
    [string[]]$GroupBy = @("extension", "depth", "age"),
    [int]$Repeat = 1,
    [int]$Top = 20,
    [int]$DiagnosticLimit = 0,
    [int]$TimeoutSeconds = 180,
    [int]$LargeFileCount = 128,
    [int]$SmallFileBytes = 1024,
    [switch]$CleanupAdvice,
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
        throw "Refusing to create a dogfood fixture at a drive root: $Path"
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

function Add-Section {
    param(
        [System.Collections.Generic.List[object]]$Sections,
        [string]$Name,
        [string]$Status,
        [hashtable]$Data = @{},
        [string[]]$Caveats = @()
    )

    $object = [ordered]@{
        name = $Name
        status = $Status
        caveats = @($Caveats)
    }
    foreach ($key in $Data.Keys) {
        $object[$key] = $Data[$key]
    }
    $Sections.Add([pscustomobject]$object) | Out-Null
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

function New-RepeatingTextFile {
    param(
        [string]$Path,
        [int]$ByteCount,
        [string]$Token
    )

    $parent = Split-Path -Parent $Path
    New-Item -ItemType Directory -Force -Path $parent | Out-Null
    $builder = [System.Text.StringBuilder]::new($ByteCount)
    while ($builder.Length -lt $ByteCount) {
        [void]$builder.Append($Token)
    }
    $text = $builder.ToString(0, $ByteCount)
    [IO.File]::WriteAllText($Path, $text, [System.Text.Encoding]::ASCII)
}

function Invoke-NativeCommand {
    param(
        [string]$FilePath,
        [string[]]$Arguments
    )

    try {
        $output = & $FilePath @Arguments 2>&1
        return [pscustomobject]@{
            exit_code = $LASTEXITCODE
            output = @($output | ForEach-Object { [string]$_ })
        }
    }
    catch {
        return [pscustomobject]@{
            exit_code = -1
            output = @($_.Exception.Message)
        }
    }
}

function New-SparseFile {
    param(
        [string]$Path,
        [int64]$LogicalBytes,
        [System.Collections.Generic.List[string]]$Caveats
    )

    $parent = Split-Path -Parent $Path
    New-Item -ItemType Directory -Force -Path $parent | Out-Null
    [IO.File]::WriteAllBytes($Path, [byte[]]::new(0))

    $setFlag = Invoke-NativeCommand -FilePath "fsutil.exe" -Arguments @("sparse", "setflag", $Path)
    if ($setFlag.exit_code -ne 0) {
        $Caveats.Add("fsutil sparse setflag failed: $($setFlag.output -join ' ')") | Out-Null
        return $false
    }

    $stream = [IO.File]::Open($Path, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::Read)
    try {
        $stream.SetLength($LogicalBytes)
        [void]$stream.Seek([Math]::Max(0, $LogicalBytes - 4096), [IO.SeekOrigin]::Begin)
        $tail = [byte[]]::new(4096)
        for ($i = 0; $i -lt $tail.Length; $i++) {
            $tail[$i] = [byte](65 + ($i % 23))
        }
        $stream.Write($tail, 0, $tail.Length)
    }
    finally {
        $stream.Dispose()
    }

    return $true
}

function New-NtfsDogfoodFixture {
    param(
        [string]$Root,
        [int]$LargeFileCountValue,
        [int]$SmallFileBytesValue
    )

    if ($LargeFileCountValue -lt 1) {
        throw "LargeFileCount must be at least 1."
    }
    if ($SmallFileBytesValue -lt 1) {
        throw "SmallFileBytes must be at least 1."
    }

    Assert-NewFixtureRoot -Path $Root
    New-Item -ItemType Directory -Force -Path $Root | Out-Null

    $sections = [System.Collections.Generic.List[object]]::new()
    $caveats = [System.Collections.Generic.List[string]]::new()

    $hardlinkDir = Join-Path $Root "hardlinks"
    $hardlinkSource = Join-Path $hardlinkDir "shared-source.bin"
    New-DeterministicFile -Path $hardlinkSource -ByteCount 32768 -Seed 11
    $hardlinkAliases = @(
        (Join-Path $hardlinkDir "shared-alias-a.bin"),
        (Join-Path $hardlinkDir "shared-alias-b.bin")
    )
    $createdHardlinks = 0
    foreach ($alias in $hardlinkAliases) {
        try {
            New-Item -ItemType HardLink -Path $alias -Target $hardlinkSource -ErrorAction Stop | Out-Null
            $createdHardlinks++
        }
        catch {
            $caveats.Add("hardlink creation failed for ${alias}: $($_.Exception.Message)") | Out-Null
        }
    }
    $hardlinkPathCount = 1 + $createdHardlinks
    Add-Section -Sections $sections -Name "hardlinks" -Status $(if ($createdHardlinks -eq $hardlinkAliases.Count) { "created" } else { "partial" }) -Data @{
        source = $hardlinkSource
        paths = @($hardlinkSource) + @($hardlinkAliases | Where-Object { Test-Path -LiteralPath $_ })
        path_count = $hardlinkPathCount
        expected_path_logical_bytes = 32768 * $hardlinkPathCount
        expected_unique_logical_bytes = 32768
    } -Caveats @($caveats | Where-Object { $_ -like "hardlink creation failed*" })

    $sparseCaveats = [System.Collections.Generic.List[string]]::new()
    $sparsePath = Join-Path $Root "sparse\sparse-hole.bin"
    $sparseLogicalBytes = 64MB
    $sparseCreated = New-SparseFile -Path $sparsePath -LogicalBytes $sparseLogicalBytes -Caveats $sparseCaveats
    Add-Section -Sections $sections -Name "sparse" -Status $(if ($sparseCreated) { "created" } else { "unsupported" }) -Data @{
        path = $sparsePath
        logical_bytes = $sparseLogicalBytes
    } -Caveats @($sparseCaveats)
    foreach ($item in $sparseCaveats) {
        $caveats.Add($item) | Out-Null
    }

    $compressedCaveats = [System.Collections.Generic.List[string]]::new()
    $compressedPath = Join-Path $Root "compressed\repetitive.txt"
    New-RepeatingTextFile -Path $compressedPath -ByteCount 262144 -Token "REBECCA-NTFS-COMPRESSIBLE`n"
    $compact = Invoke-NativeCommand -FilePath "compact.exe" -Arguments @("/C", "/I", $compressedPath)
    if ($compact.exit_code -ne 0) {
        $compressedCaveats.Add("compact /C failed: $($compact.output -join ' ')") | Out-Null
    }
    Add-Section -Sections $sections -Name "compressed" -Status $(if ($compressedCaveats.Count -eq 0) { "created" } else { "uncompressed" }) -Data @{
        path = $compressedPath
        logical_bytes = 262144
    } -Caveats @($compressedCaveats)
    foreach ($item in $compressedCaveats) {
        $caveats.Add($item) | Out-Null
    }

    $largeDir = Join-Path $Root "large-directory"
    for ($i = 0; $i -lt $LargeFileCountValue; $i++) {
        $file = Join-Path $largeDir ("file-{0:D4}.bin" -f $i)
        New-DeterministicFile -Path $file -ByteCount $SmallFileBytesValue -Seed ($i % 251)
    }
    Add-Section -Sections $sections -Name "large-directory" -Status "created" -Data @{
        path = $largeDir
        file_count = $LargeFileCountValue
        bytes_per_file = $SmallFileBytesValue
        expected_logical_bytes = [int64]$LargeFileCountValue * [int64]$SmallFileBytesValue
    }

    $nestedLeaf = Join-Path $Root "nested\level-01\level-02\level-03\leaf.bin"
    New-DeterministicFile -Path $nestedLeaf -ByteCount 4096 -Seed 71
    Add-Section -Sections $sections -Name "nested-directory" -Status "created" -Data @{
        path = $nestedLeaf
        logical_bytes = 4096
    }

    $fragmentDir = Join-Path $Root "fragmentation-candidates"
    New-Item -ItemType Directory -Force -Path $fragmentDir | Out-Null
    $tempFiles = @()
    for ($i = 0; $i -lt 24; $i++) {
        $temp = Join-Path $fragmentDir ("temporary-hole-{0:D2}.bin" -f $i)
        New-DeterministicFile -Path $temp -ByteCount 8192 -Seed (101 + $i)
        $tempFiles += $temp
    }
    foreach ($temp in $tempFiles | Where-Object { ([int]([IO.Path]::GetFileNameWithoutExtension($_).Split('-')[-1]) % 2) -eq 0 }) {
        Remove-Item -LiteralPath $temp -Force
    }
    for ($i = 0; $i -lt 8; $i++) {
        $candidate = Join-Path $fragmentDir ("fragment-candidate-{0:D2}.bin" -f $i)
        New-DeterministicFile -Path $candidate -ByteCount 24576 -Seed (151 + $i)
    }
    Add-Section -Sections $sections -Name "fragmentation-candidates" -Status "created" -Data @{
        path = $fragmentDir
        candidate_count = 8
        note = "Best-effort allocation churn; not a guaranteed fragmentation oracle."
    }

    $manifest = [pscustomobject]@{
        schema_version = 1
        generated_at_utc = (Get-Date).ToUniversalTime().ToString("o")
        fixture_root = $Root
        host = [pscustomobject]@{
            os = [System.Environment]::OSVersion.VersionString
            machine = [System.Environment]::MachineName
            current_user = [System.Environment]::UserName
        }
        sections = @($sections)
        caveats = @($caveats)
    }
    $manifestPath = Join-Path $Root "ntfs-fixture-manifest.json"
    $manifest | ConvertTo-Json -Depth 16 | Set-Content -LiteralPath $manifestPath -Encoding utf8

    return [pscustomobject]@{
        root = $Root
        manifest = $manifestPath
        section_count = $sections.Count
        caveat_count = $caveats.Count
    }
}

function Invoke-SelfTest {
    $fixtureRoot = Resolve-UnderRepoPath -Path "" -DefaultRelativeRoot "target\ntfs-dogfood-fixtures\selftest"
    $result = New-NtfsDogfoodFixture -Root $fixtureRoot -LargeFileCountValue 4 -SmallFileBytesValue 64
    if (-not (Test-Path -LiteralPath $result.manifest)) {
        throw "Self-test manifest was not written."
    }
    $manifest = Get-Content -LiteralPath $result.manifest -Raw | ConvertFrom-Json
    $sectionNames = @($manifest.sections | ForEach-Object { $_.name })
    foreach ($expected in @("hardlinks", "sparse", "compressed", "large-directory", "nested-directory", "fragmentation-candidates")) {
        if ($sectionNames -notcontains $expected) {
            throw "Self-test manifest missing section '$expected'."
        }
    }
    $manifestText = Get-Content -LiteralPath $result.manifest -Raw
    if ($manifestText.Contains("\Join-Path\")) {
        throw "Self-test manifest contains a malformed Join-Path literal."
    }
    if ($manifest.sections.Count -lt 6) {
        throw "Self-test manifest has too few sections."
    }
    Write-Host "Self-test passed. Fixture: $($result.root)"
}

if ($SelfTest) {
    Invoke-SelfTest
    return
}

$fixtureRootPath = Resolve-UnderRepoPath -Path $FixtureRoot -DefaultRelativeRoot "target\ntfs-dogfood-fixtures"
$outputRootPath = Resolve-UnderRepoPath -Path $OutputDirectory -DefaultRelativeRoot "target\ntfs-dogfood-reports"
$fixture = New-NtfsDogfoodFixture -Root $fixtureRootPath -LargeFileCountValue $LargeFileCount -SmallFileBytesValue $SmallFileBytes

$reportScript = Join-Path $PSScriptRoot "run-inspect-map-report.ps1"
$reportArgs = @(
    "-NoProfile",
    "-File",
    $reportScript,
    "-Root",
    $fixture.root,
    "-OutputDirectory",
    $outputRootPath,
    "-Backend",
    ($Backend -join ",")
)
$reportArgs += @("-GroupBy", ($GroupBy -join ","))
$reportArgs += @(
    "-Repeat",
    [string]$Repeat,
    "-Top",
    [string]$Top,
    "-DiagnosticLimit",
    [string]$DiagnosticLimit,
    "-TimeoutSeconds",
    [string]$TimeoutSeconds
)
if ($CleanupAdvice) {
    $reportArgs += "-CleanupAdvice"
}
if ($AllowMismatch) {
    $reportArgs += "-AllowMismatch"
}

& pwsh @reportArgs
$exitCode = $LASTEXITCODE
Write-Host "NTFS fixture manifest written to $($fixture.manifest)"
Write-Host "NTFS fixture report directory: $outputRootPath"
if ($exitCode -ne 0) {
    exit $exitCode
}

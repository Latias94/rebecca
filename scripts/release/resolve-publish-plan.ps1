param(
    [string]$Version = "",
    [string]$OutputPath = "",
    [string]$MarkdownPath = "",
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

function Get-PropertyValue {
    param(
        [object]$Object,
        [string]$Name
    )

    if ($null -eq $Object -or $null -eq $Object.PSObject.Properties[$Name]) {
        return $null
    }
    return $Object.$Name
}

function Read-CargoMetadata {
    $raw = cargo metadata --no-deps --format-version 1
    if ($LASTEXITCODE -ne 0) {
        throw "cargo metadata failed with exit code $LASTEXITCODE"
    }
    return $raw | ConvertFrom-Json
}

function Get-MetadataWorkspacePackages {
    param([object]$Metadata)

    $workspaceMemberIds = @{}
    foreach ($memberId in @($Metadata.workspace_members)) {
        $workspaceMemberIds[[string]$memberId] = $true
    }

    $packages = @()
    foreach ($package in @($Metadata.packages)) {
        if ($workspaceMemberIds.ContainsKey([string]$package.id)) {
            $packages += $package
        }
    }
    return @($packages | Sort-Object -Property name)
}

function Get-InternalDependencies {
    param(
        [object]$Package,
        [hashtable]$WorkspacePackagesByName
    )

    $dependencies = @()
    foreach ($dependency in @($Package.dependencies)) {
        $dependencyName = [string]$dependency.name
        if (-not $WorkspacePackagesByName.ContainsKey($dependencyName)) {
            continue
        }

        $kind = Get-PropertyValue -Object $dependency -Name "kind"
        if ($null -eq $kind) {
            $kind = "normal"
        }

        $dependencies += [pscustomobject]@{
            name = $dependencyName
            req = [string]$dependency.req
            kind = [string]$kind
            optional = [bool](Get-PropertyValue -Object $dependency -Name "optional")
        }
    }
    return @($dependencies | Sort-Object -Property name, kind)
}

function Resolve-PublishOrder {
    param(
        [string[]]$CrateNames,
        [hashtable]$InternalDependenciesByName
    )

    $published = [System.Collections.Generic.HashSet[string]]::new()
    $remaining = [System.Collections.Generic.HashSet[string]]::new()
    foreach ($crate in $CrateNames) {
        [void]$remaining.Add($crate)
    }

    $order = @()
    while ($remaining.Count -gt 0) {
        $ready = @(
            @($remaining) |
                Where-Object {
                    $crate = [string]$_
                    $deps = @($InternalDependenciesByName[$crate])
                    @($deps | Where-Object { -not $published.Contains([string]$_.name) }).Count -eq 0
                } |
                Sort-Object
        )

        if ($ready.Count -eq 0) {
            $remainingNames = (@($remaining) | Sort-Object) -join ", "
            throw "workspace crate dependency cycle or unresolved dependency among: $remainingNames"
        }

        foreach ($crate in $ready) {
            [void]$remaining.Remove($crate)
            [void]$published.Add($crate)
            $order += $crate
        }
    }
    return $order
}

function New-PublishPlanReport {
    param(
        [object]$Metadata,
        [string]$RequestedVersion
    )

    $packages = Get-MetadataWorkspacePackages -Metadata $Metadata
    if ($packages.Count -eq 0) {
        throw "cargo metadata did not include workspace packages"
    }

    if ([string]::IsNullOrWhiteSpace($RequestedVersion)) {
        $versions = @($packages | ForEach-Object { [string]$_.version } | Sort-Object -Unique)
        if ($versions.Count -ne 1) {
            throw "workspace packages do not share one version; pass -Version explicitly"
        }
        $RequestedVersion = $versions[0]
    }

    $packagesByName = @{}
    foreach ($package in $packages) {
        $name = [string]$package.name
        if ($packagesByName.ContainsKey($name)) {
            throw "duplicate workspace package name '$name'"
        }
        $packagesByName[$name] = $package
    }

    $internalDependenciesByName = @{}
    foreach ($package in $packages) {
        $internalDependenciesByName[[string]$package.name] =
            @(Get-InternalDependencies -Package $package -WorkspacePackagesByName $packagesByName)
    }

    $violations = [System.Collections.Generic.List[string]]::new()
    foreach ($package in $packages) {
        $crate = [string]$package.name
        $crateVersion = [string]$package.version
        if ($crateVersion -ne $RequestedVersion) {
            $violations.Add("$crate version '$crateVersion' does not match release version '$RequestedVersion'.")
        }

        foreach ($dependency in @($internalDependenciesByName[$crate])) {
            $expectedReq = "=$RequestedVersion"
            if ([string]$dependency.req -ne $expectedReq) {
                $violations.Add("$crate depends on $($dependency.name) with requirement '$($dependency.req)'; expected '$expectedReq' for lockstep workspace publishing.")
            }
        }
    }

    if ($violations.Count -gt 0) {
        throw "Invalid workspace publish metadata:`n$($violations -join "`n")"
    }

    $crateNames = @($packages | ForEach-Object { [string]$_.name })
    $publishOrder = Resolve-PublishOrder -CrateNames $crateNames -InternalDependenciesByName $internalDependenciesByName

    $workspaceCrates = @()
    foreach ($crate in $publishOrder) {
        $package = $packagesByName[$crate]
        $workspaceCrates += [pscustomobject]@{
            name = $crate
            version = [string]$package.version
            manifest_path = [string]$package.manifest_path
            internal_dependencies = @($internalDependenciesByName[$crate])
        }
    }

    return [pscustomobject]@{
        schema_version = 1
        generated_at_unix_seconds = Get-UnixTimeSeconds
        status = "passed"
        release_version = $RequestedVersion
        publish_order = @($publishOrder)
        workspace_crates = @($workspaceCrates)
    }
}

function New-MarkdownReport {
    param([object]$Report)

    $lines = @()
    $lines += "# Rebecca Crates.io Publish Plan"
    $lines += ""
    $lines += "- Status: $($Report.status)"
    $lines += "- Release version: $($Report.release_version)"
    $lines += "- Workspace crates: $($Report.workspace_crates.Count)"
    $lines += ""
    $lines += "| Order | Crate | Version | Internal dependencies |"
    $lines += "| ---: | --- | --- | --- |"
    for ($index = 0; $index -lt $Report.publish_order.Count; $index++) {
        $crate = [string]$Report.publish_order[$index]
        $entry = $Report.workspace_crates | Where-Object { $_.name -eq $crate } | Select-Object -First 1
        $deps = @($entry.internal_dependencies | ForEach-Object { "$($_.name) $($_.req)" })
        if ($deps.Count -eq 0) {
            $deps = @("-")
        }
        $lines += "| $($index + 1) | $crate | $($entry.version) | $($deps -join ", ") |"
    }
    return ($lines -join [Environment]::NewLine)
}

function Write-ReportFiles {
    param(
        [object]$Report,
        [string]$JsonPath,
        [string]$MarkdownReportPath
    )

    if (-not [string]::IsNullOrWhiteSpace($JsonPath)) {
        $jsonFull = [IO.Path]::GetFullPath($JsonPath)
        New-Item -ItemType Directory -Force -Path (Split-Path -Parent $jsonFull) | Out-Null
        $Report | ConvertTo-Json -Depth 32 | Set-Content -LiteralPath $jsonFull -Encoding utf8
    }

    if (-not [string]::IsNullOrWhiteSpace($MarkdownReportPath)) {
        $markdownFull = [IO.Path]::GetFullPath($MarkdownReportPath)
        New-Item -ItemType Directory -Force -Path (Split-Path -Parent $markdownFull) | Out-Null
        New-MarkdownReport -Report $Report | Set-Content -LiteralPath $markdownFull -Encoding utf8
    }
}

function Assert-Equal {
    param([object]$Actual, [object]$Expected, [string]$Message)

    if ($Actual -ne $Expected) {
        throw "$Message expected '$Expected' but got '$Actual'"
    }
}

function New-TestDependency {
    param([string]$Name, [string]$Req)

    return [pscustomobject]@{
        name = $Name
        req = $Req
        kind = $null
        optional = $false
    }
}

function New-TestPackage {
    param([string]$Name, [object[]]$Dependencies = @())

    return [pscustomobject]@{
        name = $Name
        version = "1.2.3"
        id = $Name
        manifest_path = "$Name/Cargo.toml"
        dependencies = @($Dependencies)
    }
}

function Invoke-SelfTest {
    $metadata = [pscustomobject]@{
        workspace_members = @("app", "core", "rules")
        packages = @(
            New-TestPackage -Name "app" -Dependencies @(
                New-TestDependency -Name "core" -Req "=1.2.3"
                New-TestDependency -Name "rules" -Req "=1.2.3"
            )
            New-TestPackage -Name "core" -Dependencies @()
            New-TestPackage -Name "rules" -Dependencies @(
                New-TestDependency -Name "core" -Req "=1.2.3"
            )
        )
    }

    $report = New-PublishPlanReport -Metadata $metadata -RequestedVersion "1.2.3"
    Assert-Equal -Actual ($report.publish_order -join ",") -Expected "core,rules,app" -Message "publish order"

    $badMetadata = [pscustomobject]@{
        workspace_members = @("app", "core")
        packages = @(
            New-TestPackage -Name "app" -Dependencies @(
                New-TestDependency -Name "core" -Req "^1.2.3"
            )
            New-TestPackage -Name "core" -Dependencies @()
        )
    }

    $failed = $false
    try {
        New-PublishPlanReport -Metadata $badMetadata -RequestedVersion "1.2.3" | Out-Null
    }
    catch {
        $failed = $true
    }
    Assert-Equal -Actual $failed -Expected $true -Message "mismatched dependency requirement should fail"
    Write-Host "Publish plan self-test passed."
}

if ($SelfTest) {
    Invoke-SelfTest
    exit 0
}

$repoRoot = Get-RepoRoot
Push-Location $repoRoot
try {
    $metadata = Read-CargoMetadata
    $report = New-PublishPlanReport -Metadata $metadata -RequestedVersion $Version
    Write-ReportFiles -Report $report -JsonPath $OutputPath -MarkdownReportPath $MarkdownPath
    Write-Host "Publish order: $($report.publish_order -join ", ")"
    if (-not [string]::IsNullOrWhiteSpace($OutputPath)) {
        Write-Host "Wrote publish plan JSON: $([IO.Path]::GetFullPath($OutputPath))"
    }
    if (-not [string]::IsNullOrWhiteSpace($MarkdownPath)) {
        Write-Host "Wrote publish plan Markdown: $([IO.Path]::GetFullPath($MarkdownPath))"
    }
}
finally {
    Pop-Location
}

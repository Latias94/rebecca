param(
    [string]$Version = "",
    [string]$PublishPlanPath = "target/release-preflight/publish-plan.json",
    [string]$PackageListDir = "target/package-lists",
    [string]$EvidencePath = "target/release-preflight/package-verification.json",
    [switch]$AllowDirty,
    [switch]$SelfTest
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Invoke-CargoCommand {
    param([string[]]$Arguments)

    & cargo @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "cargo $($Arguments -join ' ') failed with exit code $LASTEXITCODE"
    }
}

function Test-CrateVersionPublished {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Crate,
        [Parameter(Mandatory = $true)]
        [string]$Version
    )

    cargo info "$Crate@$Version" --registry crates-io *> $null
    return $LASTEXITCODE -eq 0
}

function Get-PublishPlan {
    param([string]$Path)

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "publish plan not found: $Path"
    }
    return Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json
}

function Get-CrateEntry {
    param([object]$PublishPlan, [string]$Crate)

    $entry = @($PublishPlan.workspace_crates | Where-Object { [string]$_.name -eq $Crate } | Select-Object -First 1)
    if ($entry.Count -eq 0) {
        throw "crate '$Crate' is missing from publish plan workspace_crates"
    }
    return $entry[0]
}

function Invoke-PackageList {
    param([string]$Crate, [string]$OutDir)

    New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
    $args = @("package", "-p", $Crate, "--list", "--locked")
    if ($AllowDirty) {
        $args += "--allow-dirty"
    }

    $output = & cargo @args
    if ($LASTEXITCODE -ne 0) {
        throw "cargo $($args -join ' ') failed with exit code $LASTEXITCODE"
    }
    $output | Set-Content -LiteralPath (Join-Path $OutDir "$Crate.txt") -Encoding utf8
}

function Invoke-DependencyContextPackage {
    param([string]$Crate)

    $args = @("package", "-p", $Crate, "--locked", "--no-verify")
    if ($AllowDirty) {
        $args += "--allow-dirty"
    }
    Invoke-CargoCommand -Arguments $args
}

function Assert-PackageListContains {
    param([string]$Crate, [string]$RelativePath)

    $packageList = Join-Path $PackageListDir "$Crate.txt"
    if (-not (Select-String -LiteralPath $packageList -Pattern ([regex]::Escape($RelativePath)) -Quiet)) {
        throw "$Crate package is missing required asset: $RelativePath"
    }
}

function Assert-RebeccaPackageAssets {
    foreach ($schema in @(
        "schemas/api/cli/v1/envelope.schema.json",
        "schemas/api/cli/v1/error.schema.json",
        "schemas/api/cli/v1/event.schema.json",
        "schemas/api/cli/v1/payloads.schema.json",
        "schemas/api/cli/v1/config.schema.json",
        "schemas/api/cli/v1/cleaner-manifest-v1.schema.json"
    )) {
        Assert-PackageListContains -Crate "rebecca" -RelativePath $schema
    }

    Assert-PackageListContains -Crate "rebecca" -RelativePath "skills/rebecca-disk-cleaner/SKILL.md"
}

function Get-UnpublishedVerificationKind {
    param([int]$InternalDependencyCount)

    if ($InternalDependencyCount -eq 0) {
        return "publish-dry-run"
    }
    return "package-no-verify-plus-exact-internal-dependency-proof"
}

function Assert-Equal {
    param([object]$Actual, [object]$Expected, [string]$Message)

    if ($Actual -ne $Expected) {
        throw "$Message expected '$Expected' but got '$Actual'"
    }
}

function Invoke-SelfTest {
    Assert-Equal -Actual (Get-UnpublishedVerificationKind -InternalDependencyCount 0) -Expected "publish-dry-run" -Message "registry-independent crate verification kind"
    Assert-Equal -Actual (Get-UnpublishedVerificationKind -InternalDependencyCount 1) -Expected "package-no-verify-plus-exact-internal-dependency-proof" -Message "dependent crate verification kind"
    Write-Host "Package verification self-test passed."
}

if ($SelfTest) {
    Invoke-SelfTest
    exit 0
}

if ([string]::IsNullOrWhiteSpace($Version)) {
    throw "-Version is required"
}

$publishPlan = Get-PublishPlan -Path $PublishPlanPath
New-Item -ItemType Directory -Force -Path $PackageListDir | Out-Null
New-Item -ItemType Directory -Force -Path (Split-Path -Parent ([IO.Path]::GetFullPath($EvidencePath))) | Out-Null

$results = @()
foreach ($crate in @($publishPlan.publish_order | ForEach-Object { [string]$_ })) {
    $entry = Get-CrateEntry -PublishPlan $publishPlan -Crate $crate
    $internalDependencies = @($entry.internal_dependencies)

    Invoke-PackageList -Crate $crate -OutDir $PackageListDir

    if (Test-CrateVersionPublished -Crate $crate -Version $Version) {
        Write-Host "Skipping package verifier for $crate $Version because it is already published."
        $results += [pscustomobject]@{
            crate = $crate
            verification = "already-published"
            internal_dependencies = @($internalDependencies | ForEach-Object { [string]$_.name })
        }
        continue
    }

    $verification = Get-UnpublishedVerificationKind -InternalDependencyCount $internalDependencies.Count

    if ($verification -eq "publish-dry-run") {
        Write-Host "Dry-running registry-independent crate $crate $Version"
        Invoke-CargoCommand -Arguments @("publish", "-p", $crate, "--locked", "--dry-run", "--registry", "crates-io")
    } else {
        Write-Host "Packaging dependent crate $crate $Version with exact internal dependency proof"
        Invoke-DependencyContextPackage -Crate $crate
    }

    $results += [pscustomobject]@{
        crate = $crate
        verification = $verification
        internal_dependencies = @($internalDependencies | ForEach-Object { "$($_.name) $($_.req)" })
    }
}

Assert-RebeccaPackageAssets

[pscustomobject]@{
    schema_version = 1
    release_version = $Version
    package_list_dir = $PackageListDir
    publish_plan = $PublishPlanPath
    results = @($results)
} | ConvertTo-Json -Depth 32 | Set-Content -LiteralPath $EvidencePath -Encoding utf8

Write-Host "Wrote package verification evidence: $([IO.Path]::GetFullPath($EvidencePath))"

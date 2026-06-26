param(
    [string]$Tag = $env:GITHUB_REF_NAME,
    [string]$Repository = $env:GITHUB_REPOSITORY,
    [string]$Target = "x86_64-pc-windows-msvc",
    [string]$DistDir = "dist",
    [string]$ArtifactPath = "",
    [string]$OutputFile = ""
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Get-RepoRoot {
    $scriptRoot = Split-Path -Parent $PSCommandPath
    return (Resolve-Path -LiteralPath (Join-Path $scriptRoot "..\..")).ProviderPath
}

function Get-WorkspaceVersion {
    param([string]$RepoRoot)

    $cargoToml = Join-Path $RepoRoot "Cargo.toml"
    $versionLine = Get-Content -LiteralPath $cargoToml |
        Where-Object { $_ -match '^\s*version\s*=\s*"([^"]+)"' } |
        Select-Object -First 1

    if (-not $versionLine) {
        throw "Could not find workspace package version in Cargo.toml"
    }

    if ($versionLine -notmatch '^\s*version\s*=\s*"([^"]+)"') {
        throw "Could not parse workspace package version from Cargo.toml"
    }

    return $Matches[1]
}

function Resolve-ReleaseVersion {
    param(
        [string]$RepoRoot,
        [string]$Tag
    )

    if ([string]::IsNullOrWhiteSpace($Tag)) {
        return Get-WorkspaceVersion -RepoRoot $RepoRoot
    }

    $normalizedTag = $Tag.Trim()
    if ($normalizedTag.StartsWith("refs/tags/")) {
        $normalizedTag = $normalizedTag.Substring("refs/tags/".Length)
    }

    if ($normalizedTag -match '^[vV](\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?)$') {
        return $Matches[1]
    }

    if ($normalizedTag -match '^(\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?)$') {
        return $Matches[1]
    }

    return Get-WorkspaceVersion -RepoRoot $RepoRoot
}

function Resolve-TargetLabel {
    param([string]$Target)

    switch ($Target) {
        "x86_64-pc-windows-msvc" { return "windows-x86_64-msvc" }
        default { throw "Unsupported release target: $Target" }
    }
}

function Resolve-PathForRepo {
    param(
        [string]$RepoRoot,
        [string]$Path
    )

    if ([System.IO.Path]::IsPathRooted($Path)) {
        return [System.IO.Path]::GetFullPath($Path)
    }

    return [System.IO.Path]::GetFullPath((Join-Path $RepoRoot $Path))
}

function Assert-ChildPath {
    param(
        [string]$Parent,
        [string]$Child
    )

    $parentFull = [System.IO.Path]::GetFullPath($Parent).TrimEnd([System.IO.Path]::DirectorySeparatorChar, [System.IO.Path]::AltDirectorySeparatorChar)
    $childFull = [System.IO.Path]::GetFullPath($Child)
    $prefix = $parentFull + [System.IO.Path]::DirectorySeparatorChar

    if (-not $childFull.StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing to write SBOM outside distribution directory: $childFull"
    }
}

function ConvertTo-SpdxId {
    param(
        [string]$Prefix,
        [string]$Name,
        [string]$Version
    )

    $raw = "$Prefix-$Name-$Version"
    $id = $raw -replace '[^A-Za-z0-9.-]', '-'
    $id = $id.Trim("-")

    if ([string]::IsNullOrWhiteSpace($id)) {
        throw "Could not derive SPDX id for package $Name $Version"
    }

    return "SPDXRef-$id"
}

function ConvertTo-SpdxText {
    param([AllowNull()][string]$Value)

    if ([string]::IsNullOrWhiteSpace($Value)) {
        return "NOASSERTION"
    }

    return ($Value.Trim() -replace '[\r\n]+', ' ')
}

function ConvertTo-SpdxLicense {
    param([AllowNull()][string]$License)

    if ([string]::IsNullOrWhiteSpace($License)) {
        return "NOASSERTION"
    }

    $normalized = $License.Trim() -replace '\s*/\s*', ' OR '
    return ConvertTo-SpdxText -Value $normalized
}

function Get-NonDevDependencyIds {
    param($Node)

    $ids = New-Object System.Collections.Generic.List[string]

    foreach ($dependency in $Node.deps) {
        $include = $false

        if (-not $dependency.dep_kinds -or $dependency.dep_kinds.Count -eq 0) {
            $include = $true
        }
        else {
            foreach ($kind in $dependency.dep_kinds) {
                if ($kind.kind -ne "dev") {
                    $include = $true
                    break
                }
            }
        }

        if ($include) {
            $ids.Add([string]$dependency.pkg)
        }
    }

    return $ids
}

function Get-ReachablePackageIds {
    param(
        $Metadata,
        [string]$RootPackageId
    )

    $nodesById = @{}
    foreach ($node in $Metadata.resolve.nodes) {
        $nodesById[[string]$node.id] = $node
    }

    if (-not $nodesById.ContainsKey($RootPackageId)) {
        throw "Cargo metadata did not include a resolve node for $RootPackageId"
    }

    $visited = New-Object System.Collections.Generic.HashSet[string]
    $queue = New-Object System.Collections.Generic.Queue[string]
    $queue.Enqueue($RootPackageId)

    while ($queue.Count -gt 0) {
        $id = $queue.Dequeue()
        if (-not $visited.Add($id)) {
            continue
        }

        $node = $nodesById[$id]
        foreach ($dependencyId in (Get-NonDevDependencyIds -Node $node)) {
            $queue.Enqueue($dependencyId)
        }
    }

    return $visited
}

function Get-ReleaseRootPackage {
    param($Metadata)

    $rootPackage = $Metadata.packages |
        Where-Object { $_.name -eq "rebecca-cli" -and $Metadata.workspace_members -contains $_.id } |
        Select-Object -First 1

    if (-not $rootPackage) {
        throw "Could not find rebecca-cli workspace package in Cargo metadata"
    }

    return $rootPackage
}

$repoRoot = Get-RepoRoot
$version = Resolve-ReleaseVersion -RepoRoot $repoRoot -Tag $Tag
$targetLabel = Resolve-TargetLabel -Target $Target
$distDirFull = Resolve-PathForRepo -RepoRoot $repoRoot -Path $DistDir

if (-not (Test-Path -LiteralPath $distDirFull -PathType Container)) {
    throw "Distribution directory does not exist: $distDirFull"
}

if ([string]::IsNullOrWhiteSpace($ArtifactPath)) {
    $artifactPathFull = Join-Path $distDirFull "rebecca-$version-$targetLabel.zip"
}
else {
    $artifactPathFull = Resolve-PathForRepo -RepoRoot $repoRoot -Path $ArtifactPath
}

if (-not (Test-Path -LiteralPath $artifactPathFull -PathType Leaf)) {
    throw "Release artifact does not exist: $artifactPathFull"
}

if ([string]::IsNullOrWhiteSpace($OutputFile)) {
    $outputFileFull = Join-Path $distDirFull "rebecca-$version-$targetLabel.spdx"
}
else {
    $outputFileFull = Resolve-PathForRepo -RepoRoot $repoRoot -Path $OutputFile
}

Assert-ChildPath -Parent $distDirFull -Child $outputFileFull

Push-Location $repoRoot
try {
    $metadataJson = cargo metadata --locked --format-version 1 --filter-platform $Target
    $metadata = $metadataJson | ConvertFrom-Json
}
finally {
    Pop-Location
}

$rootPackage = Get-ReleaseRootPackage -Metadata $metadata
$reachableIds = Get-ReachablePackageIds -Metadata $metadata -RootPackageId ([string]$rootPackage.id)
$packagesById = @{}
foreach ($package in $metadata.packages) {
    $packagesById[[string]$package.id] = $package
}

$componentPackages = foreach ($id in $reachableIds) {
    if ($id -ne [string]$rootPackage.id) {
        $packagesById[$id]
    }
}
$componentPackages = $componentPackages | Sort-Object name, version, source

$artifactName = [System.IO.Path]::GetFileName($artifactPathFull)
$artifactHash = (Get-FileHash -LiteralPath $artifactPathFull -Algorithm SHA256).Hash.ToLowerInvariant()
$created = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
$repositoryText = ConvertTo-SpdxText -Value $Repository
$tagText = ConvertTo-SpdxText -Value $Tag
$documentNamespaceTag = if ([string]::IsNullOrWhiteSpace($Tag)) { "untagged-$version" } else { $Tag.Trim() -replace '[^A-Za-z0-9._-]', '-' }
$documentNamespaceRepo = if ([string]::IsNullOrWhiteSpace($Repository)) { "local" } else { $Repository.Trim() -replace '[^A-Za-z0-9._/-]', '-' }
$documentNamespace = "https://rebecca.local/spdx/$documentNamespaceRepo/$documentNamespaceTag/$targetLabel"

$rootSpdxId = "SPDXRef-Package-rebecca"
$lines = New-Object System.Collections.Generic.List[string]
$lines.Add("SPDXVersion: SPDX-2.3")
$lines.Add("DataLicense: CC0-1.0")
$lines.Add("SPDXID: SPDXRef-DOCUMENT")
$lines.Add("DocumentName: Rebecca $version $targetLabel")
$lines.Add("DocumentNamespace: $documentNamespace")
$lines.Add("Creator: Tool: rebecca-release-sbom")
$lines.Add("Created: $created")
$lines.Add("")
$lines.Add("##### Package: Rebecca release artifact")
$lines.Add("PackageName: rebecca")
$lines.Add("SPDXID: $rootSpdxId")
$lines.Add("PackageVersion: $version")
$lines.Add("PackageFileName: $artifactName")
$lines.Add("PackageSupplier: Organization: Rebecca")
$lines.Add("PackageDownloadLocation: NOASSERTION")
$lines.Add("FilesAnalyzed: false")
$lines.Add("PackageChecksum: SHA256: $artifactHash")
$lines.Add("PackageLicenseConcluded: NOASSERTION")
$lines.Add("PackageLicenseDeclared: NOASSERTION")
$lines.Add("PackageCopyrightText: NOASSERTION")
$lines.Add("PackageComment: Target=$Target; Repository=$repositoryText; Tag=$tagText; Dependency data generated from cargo metadata --locked.")
$lines.Add("Relationship: SPDXRef-DOCUMENT DESCRIBES $rootSpdxId")

$index = 0
foreach ($package in $componentPackages) {
    $index += 1
    $packageId = ConvertTo-SpdxId -Prefix "Package-$index" -Name ([string]$package.name) -Version ([string]$package.version)

    $downloadLocation = "NOASSERTION"
    if ($package.source -and ([string]$package.source).StartsWith("registry+https://github.com/rust-lang/crates.io-index")) {
        $downloadLocation = "https://crates.io/crates/$($package.name)/$($package.version)"
    }
    elseif ($package.source) {
        $downloadLocation = ConvertTo-SpdxText -Value ([string]$package.source)
    }

    $license = ConvertTo-SpdxLicense -License ([string]$package.license)

    $lines.Add("")
    $lines.Add("##### Package: $($package.name)")
    $lines.Add("PackageName: $($package.name)")
    $lines.Add("SPDXID: $packageId")
    $lines.Add("PackageVersion: $($package.version)")
    $lines.Add("PackageSupplier: NOASSERTION")
    $lines.Add("PackageDownloadLocation: $downloadLocation")
    $lines.Add("FilesAnalyzed: false")
    $lines.Add("PackageLicenseConcluded: NOASSERTION")
    $lines.Add("PackageLicenseDeclared: $license")
    $lines.Add("PackageCopyrightText: NOASSERTION")
    $lines.Add("Relationship: $rootSpdxId DEPENDS_ON $packageId")
}

Set-Content -LiteralPath $outputFileFull -Value $lines -Encoding utf8
Write-Host "Wrote SBOM: $outputFileFull"

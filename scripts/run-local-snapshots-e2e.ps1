[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string] $E0InputPath,

    [Parameter(Mandatory = $true)]
    [string] $OriginalInputPath,

    [Parameter(Mandatory = $true)]
    [string] $OutputRoot,

    [Parameter(Mandatory = $true)]
    [string] $ObservedCutoff,

    [ValidateRange(1, 5000)]
    [int] $ArranqueCases = 200,

    [ValidateRange(5, 5000)]
    [int] $MinimumRecurringQueryCases = 20
)

$ErrorActionPreference = 'Stop'

function Get-FullPath {
    param([Parameter(Mandatory = $true)][string] $Path)

    try {
        return [System.IO.Path]::GetFullPath($Path)
    }
    catch {
        throw 'Input and output must be valid filesystem paths.'
    }
}

function Test-PathsOverlap {
    param(
        [Parameter(Mandatory = $true)][string] $Left,
        [Parameter(Mandatory = $true)][string] $Right
    )

    $leftPath = [System.IO.Path]::GetFullPath($Left).TrimEnd('\', '/')
    $rightPath = [System.IO.Path]::GetFullPath($Right).TrimEnd('\', '/')
    $separator = [System.IO.Path]::DirectorySeparatorChar.ToString()
    $comparison = [System.StringComparison]::OrdinalIgnoreCase
    return $leftPath.Equals($rightPath, $comparison) -or
        $rightPath.StartsWith($leftPath + $separator, $comparison) -or
        $leftPath.StartsWith($rightPath + $separator, $comparison)
}

function Assert-NoReparsePointComponents {
    param([Parameter(Mandatory = $true)][string] $Path)

    $fullPath = [System.IO.Path]::GetFullPath($Path)
    $root = [System.IO.Path]::GetPathRoot($fullPath)
    if ([string]::IsNullOrWhiteSpace($root)) {
        throw 'Input and output must be direct filesystem paths.'
    }

    $current = $root
    $components = $fullPath.Substring($root.Length).Split(
        [char[]] @([System.IO.Path]::DirectorySeparatorChar, [System.IO.Path]::AltDirectorySeparatorChar),
        [System.StringSplitOptions]::RemoveEmptyEntries
    )
    foreach ($component in $components) {
        $current = Join-Path $current $component
        try {
            $attributes = [System.IO.File]::GetAttributes($current)
        }
        catch [System.IO.FileNotFoundException] {
            continue
        }
        catch [System.IO.DirectoryNotFoundException] {
            continue
        }

        if (($attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw 'Input or output traverses a reparse point; use direct filesystem paths.'
        }
    }
}

function Resolve-InputDirectory {
    param([Parameter(Mandatory = $true)][string] $Path)

    try {
        $resolved = Resolve-Path -LiteralPath $Path -ErrorAction Stop
    }
    catch {
        throw 'Both source inputs must name existing filesystem directories.'
    }
    if ($resolved.Provider.Name -ne 'FileSystem' -or -not (Test-Path -LiteralPath $resolved.Path -PathType Container)) {
        throw 'Both source inputs must name existing filesystem directories.'
    }
    Assert-NoReparsePointComponents -Path $resolved.Path
    return Get-FullPath -Path $resolved.Path
}

$e0InputFullPath = Resolve-InputDirectory -Path $E0InputPath
$originalInputFullPath = Resolve-InputDirectory -Path $OriginalInputPath
$outputRootFullPath = Get-FullPath -Path $OutputRoot
Assert-NoReparsePointComponents -Path $outputRootFullPath

if (Test-PathsOverlap -Left $e0InputFullPath -Right $originalInputFullPath) {
    throw 'E0InputPath and OriginalInputPath must not overlap.'
}
foreach ($sourcePath in @($e0InputFullPath, $originalInputFullPath)) {
    if (Test-PathsOverlap -Left $sourcePath -Right $outputRootFullPath) {
        throw 'OutputRoot must not overlap either source input.'
    }
}
if (Test-Path -LiteralPath $outputRootFullPath) {
    throw 'OutputRoot already exists; choose a fresh directory. Existing output is never overwritten.'
}

if ($ObservedCutoff -cnotmatch '\A\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z\z') {
    throw 'ObservedCutoff must be UTC RFC3339 with whole-second precision.'
}
$parsedCutoff = [System.DateTimeOffset]::MinValue
$cutoffStyle = [System.Globalization.DateTimeStyles]::AssumeUniversal -bor [System.Globalization.DateTimeStyles]::AdjustToUniversal
if (-not [System.DateTimeOffset]::TryParseExact(
        $ObservedCutoff,
        "yyyy-MM-dd'T'HH:mm:ss'Z'",
        [System.Globalization.CultureInfo]::InvariantCulture,
        $cutoffStyle,
        [ref] $parsedCutoff
    )) {
    throw 'ObservedCutoff is not a valid UTC calendar timestamp.'
}

$repositoryRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$singleSourceRunner = Join-Path $PSScriptRoot 'run-local-e0-e2e.ps1'
if (-not (Test-Path -LiteralPath (Join-Path $repositoryRoot 'Cargo.toml') -PathType Leaf) -or
    -not (Test-Path -LiteralPath $singleSourceRunner -PathType Leaf)) {
    throw 'Could not locate the improvement-engine local runner.'
}

New-Item -ItemType Directory -Path $outputRootFullPath -ErrorAction Stop | Out-Null

Write-Output '=== E0 source run ==='
try {
    & $singleSourceRunner `
        -Source e0 `
        -InputPath $e0InputFullPath `
        -OutputPath (Join-Path $outputRootFullPath 'e0') `
        -ObservedCutoff $ObservedCutoff `
        -ArranqueCases $ArranqueCases `
        -MinimumRecurringQueryCases $MinimumRecurringQueryCases
}
catch {
    throw 'Combined local run stopped during the E0 source. Any completed derived artifacts remain in OutputRoot; use a new OutputRoot for a retry.'
}

Write-Output '=== Original-bank source run ==='
try {
    & $singleSourceRunner `
        -Source original `
        -InputPath $originalInputFullPath `
        -OutputPath (Join-Path $outputRootFullPath 'original') `
        -ObservedCutoff $ObservedCutoff
}
catch {
    throw 'Combined local run stopped during the original-bank source. Any completed derived artifacts remain in OutputRoot; use a new OutputRoot for a retry.'
}

Write-Output '=== Both local-simulation source runs completed ==='
Write-Output 'Each source wrote its own result.json and events.ndjson below separate output directories.'

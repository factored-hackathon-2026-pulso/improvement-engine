<#
.SYNOPSIS
    Produces a read-only inventory of Git worktrees for one or more repositories.

.DESCRIPTION
    Uses local refs only; it never fetches, prunes, removes, or changes worktrees.
    Review each disposition before taking any cleanup action. The default base
    ref is origin/main, which may be stale until refreshed separately.

.EXAMPLE
    .\inventory-worktrees.ps1 -RepositoryPath D:\repos\improvement-engine `
        -AdditionalRepositoryPath D:\repos\infra -MaxCombinedWorktrees 40
#>
[CmdletBinding()]
param(
    [Parameter()]
    [string] $RepositoryPath = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..')).Path,

    [Parameter()]
    [string[]] $AdditionalRepositoryPath = @(),

    [Parameter()]
    [string] $BaseRef = 'origin/main',

    [Parameter()]
    [ValidateRange(1, 1000)]
    [int] $MaxCombinedWorktrees = 40
)

$ErrorActionPreference = 'Stop'

function Invoke-InventoryGit {
    param(
        [Parameter(Mandatory)] [string] $Path,
        [Parameter(Mandatory)] [string[]] $GitArguments
    )

    $safePath = [IO.Path]::GetFullPath($Path).Replace('\', '/')
    $previousOptionalLocks = $env:GIT_OPTIONAL_LOCKS
    $env:GIT_OPTIONAL_LOCKS = '0'
    try {
        $output = & git -c "safe.directory=$safePath" -C $Path @GitArguments 2>$null
    } finally {
        if ($null -eq $previousOptionalLocks) {
            Remove-Item Env:\GIT_OPTIONAL_LOCKS -ErrorAction SilentlyContinue
        } else {
            $env:GIT_OPTIONAL_LOCKS = $previousOptionalLocks
        }
    }
    if ($LASTEXITCODE -ne 0) {
        throw "git $($GitArguments -join ' ') failed for '$Path' (exit $LASTEXITCODE)."
    }
    return $output
}

function Get-WorktreeRecords {
    param(
        [Parameter(Mandatory)] [string] $Path,
        [Parameter(Mandatory)] [string] $Reference
    )

    $repo = [IO.Path]::GetFullPath($Path)
    if (-not (Test-Path -LiteralPath $repo -PathType Container)) {
        throw "Repository path does not exist: $repo"
    }

    $records = @()
    $current = $null
    foreach ($line in (Invoke-InventoryGit -Path $repo -GitArguments @('worktree', 'list', '--porcelain'))) {
        $text = [string] $line
        if ([string]::IsNullOrWhiteSpace($text)) {
            if ($null -ne $current) { $records += $current; $current = $null }
            continue
        }
        if ($text.StartsWith('worktree ')) {
            if ($null -ne $current) { $records += $current }
            $current = [ordered]@{
                Path = $text.Substring(9)
                Head = $null
                Branch = $null
                IsDetached = $false
                IsLocked = $false
                IsPrunable = $false
            }
        } elseif ($null -ne $current -and $text.StartsWith('HEAD ')) {
            $current.Head = $text.Substring(5)
        } elseif ($null -ne $current -and $text.StartsWith('branch ')) {
            $current.Branch = $text.Substring(7) -replace '^refs/heads/', ''
        } elseif ($null -ne $current -and $text -eq 'detached') {
            $current.IsDetached = $true
        } elseif ($null -ne $current -and $text.StartsWith('locked')) {
            $current.IsLocked = $true
        } elseif ($null -ne $current -and $text.StartsWith('prunable')) {
            $current.IsPrunable = $true
        }
    }
    if ($null -ne $current) { $records += $current }

    $baseExists = $true
    try {
        [void](Invoke-InventoryGit -Path $repo -GitArguments @('rev-parse', '--verify', '--quiet', "$Reference^{commit}"))
    } catch {
        $baseExists = $false
    }

    foreach ($record in $records) {
        $checkoutExists = Test-Path -LiteralPath $record.Path -PathType Container
        $status = @()
        if ($checkoutExists -and -not $record.IsPrunable) {
            $status = @(Invoke-InventoryGit -Path $record.Path -GitArguments @('status', '--porcelain', '--untracked-files=all'))
        }
        $isDirty = $status.Count -gt 0
        $ahead = $null
        $behind = $null
        $disposition = 'NEEDS_BASE_REF'

        if ($baseExists -and $record.Head) {
            try {
                $counts = (Invoke-InventoryGit -Path $repo -GitArguments @('rev-list', '--left-right', '--count', "$Reference...$($record.Head)")) -join ''
                $parts = $counts.Trim() -split '\s+'
                $behind = [int]$parts[0]
                $ahead = [int]$parts[1]
                if ($record.IsPrunable) {
                    $disposition = 'INSPECT_PRUNABLE_NO_AUTO_DELETE'
                } elseif ($record.IsLocked) {
                    $disposition = 'KEEP_LOCKED'
                } elseif ($isDirty) {
                    $disposition = 'KEEP_DIRTY_UNMERGED'
                } elseif ($ahead -gt 0) {
                    $disposition = 'KEEP_UNMERGED'
                } elseif ($record.Branch -eq 'main' -or $record.Branch -eq 'master') {
                    $disposition = 'KEEP_BASE_WORKTREE'
                } else {
                    $disposition = 'REVIEW_MERGED_CLEAN_NO_AUTO_DELETE'
                }
            } catch {
                $disposition = 'NEEDS_MANUAL_REVIEW'
            }
        } elseif ($isDirty) {
            $disposition = 'KEEP_DIRTY_BASE_UNKNOWN'
        }

        [pscustomobject]@{
            Repository = $repo
            Path = [string]$record.Path
            Branch = if ($record.Branch) { [string]$record.Branch } else { '(detached)' }
            Head = [string]$record.Head
            IsDetached = [bool]$record.IsDetached
            IsLocked = [bool]$record.IsLocked
            IsPrunable = [bool]$record.IsPrunable
            IsDirty = $isDirty
            Ahead = $ahead
            Behind = $behind
            BaseRef = $Reference
            Disposition = $disposition
        }
    }
}

$allRepositories = @($RepositoryPath) + @($AdditionalRepositoryPath)
$canonicalRepositories = @($allRepositories | ForEach-Object { [IO.Path]::GetFullPath($_) } | Select-Object -Unique)
$inventory = @(
    foreach ($repository in $canonicalRepositories) {
        Get-WorktreeRecords -Path $repository -Reference $BaseRef
    }
)
$combinedCount = $inventory.Count
$overLimit = $combinedCount -gt $MaxCombinedWorktrees

foreach ($item in $inventory) {
    $item | Add-Member -NotePropertyName CombinedWorktreeCount -NotePropertyValue $combinedCount
    $item | Add-Member -NotePropertyName MaxCombinedWorktrees -NotePropertyValue $MaxCombinedWorktrees
    $item | Add-Member -NotePropertyName CombinedLimitExceeded -NotePropertyValue $overLimit
}

$inventory

# Read-only worktree inventory (WTR1). Never removes anything; the retirement plan is dry-run only.
[CmdletBinding()]
param(
    [int] $Cap = 40,
    [string] $Base = 'origin/main',
    [string] $WorktreeListFile,
    [string] $MergedBranchesFile,
    [switch] $Json
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$listText = if ($WorktreeListFile) { Get-Content -Raw $WorktreeListFile } else { (git worktree list --porcelain) -join "`n" }
$merged = if ($MergedBranchesFile) { @(Get-Content $MergedBranchesFile | Where-Object { $_ }) }
          else { @(git branch --merged $Base --format '%(refname:short)' | ForEach-Object { $_.Trim() }) }

$worktrees = @()
$first = $true
foreach ($block in ($listText -split "(\r?\n){2,}")) {
    if ($block -notmatch '(?m)^worktree (.+)$') { continue }
    $path = $Matches[1].Trim()
    $branch = if ($block -match '(?m)^branch refs/heads/(.+)$') { $Matches[1].Trim() } else { '(detached)' }
    $owner = if ($branch -like 'claude/*' -or $path -match '-claude-') { 'claude' }
             elseif ($branch -like 'codex/*' -or $path -match '-codex-') { 'codex' }
             else { 'unknown' }
    $worktrees += [pscustomobject]@{ path = $path; branch = $branch; owner = $owner; merged = ($merged -contains $branch); primary = $first }
    $first = $false
}

$count = $worktrees.Count
$unmerged = @($worktrees | Where-Object { -not $_.merged } | ForEach-Object { $_.branch })
$plan = @($worktrees | Where-Object { $_.merged -and -not $_.primary -and $_.owner -eq 'claude' -and $_.branch -notin @('main', 'master') } |
    ForEach-Object { [pscustomobject]@{ branch = $_.branch; path = $_.path; action = "git worktree remove `"$($_.path)`" (NOT executed)" } })
$result = [pscustomobject]@{ registered = $count; cap = $Cap; over_cap = ($count -gt $Cap); dry_run = $true
    worktrees = $worktrees; unmerged = $unmerged; retirement_plan = $plan }

if ($Json) { $result | ConvertTo-Json -Depth 5 }
else {
    $worktrees | Format-Table owner, merged, branch, path -AutoSize | Out-String | Write-Output
    Write-Output "registered=$count cap=$Cap dry_run=true unmerged=$($unmerged.Count) plan=$($plan.Count)"
}
if ($count -gt $Cap) { [Console]::Error.WriteLine("Over cap: $count registered worktrees > $Cap."); exit 1 }
exit 0

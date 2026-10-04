$ErrorActionPreference = 'Stop'

Describe 'inventory-worktrees.ps1' {
    BeforeAll {
        $scriptPath = Join-Path $PSScriptRoot 'inventory-worktrees.ps1'
        $script:repoPath = Join-Path $TestDrive 'engine repository'
        $script:featurePath = Join-Path $TestDrive 'feature worktree with spaces'
        $script:prunablePath = Join-Path $TestDrive 'missing prunable checkout'
        $script:secondRepoPath = Join-Path $TestDrive 'infra repository'
        New-Item -ItemType Directory -Path $script:repoPath | Out-Null
        & git -c init.defaultBranch=main -C $script:repoPath init | Out-Null
        & git -C $script:repoPath config user.name 'Pulso Test'
        & git -C $script:repoPath config user.email 'pulso-test@example.invalid'
        Set-Content -LiteralPath (Join-Path $script:repoPath 'README.md') -Value 'base'
        & git -C $script:repoPath add README.md
        & git -C $script:repoPath commit -m 'base' | Out-Null
        $baseSha = (& git -C $script:repoPath rev-parse HEAD).Trim()
        & git -C $script:repoPath update-ref refs/remotes/origin/main $baseSha
        & git -C $script:repoPath worktree add -b feature/active $script:featurePath main | Out-Null
        Set-Content -LiteralPath (Join-Path $script:featurePath 'slice.txt') -Value 'unmerged change'
        & git -C $script:featurePath add slice.txt
        & git -C $script:featurePath commit -m 'unmerged slice' | Out-Null
        Set-Content -LiteralPath (Join-Path $script:featurePath 'README.md') -Value 'dirty change'
        & git -C $script:repoPath worktree add -b feature/prunable $script:prunablePath main | Out-Null
        Remove-Item -LiteralPath $script:prunablePath -Recurse -Force

        New-Item -ItemType Directory -Path $script:secondRepoPath | Out-Null
        & git -c init.defaultBranch=main -C $script:secondRepoPath init | Out-Null
        & git -C $script:secondRepoPath config user.name 'Pulso Test'
        & git -C $script:secondRepoPath config user.email 'pulso-test@example.invalid'
        Set-Content -LiteralPath (Join-Path $script:secondRepoPath 'README.md') -Value 'infra base'
        & git -C $script:secondRepoPath add README.md
        & git -C $script:secondRepoPath commit -m 'infra base' | Out-Null
        $secondBaseSha = (& git -C $script:secondRepoPath rev-parse HEAD).Trim()
        & git -C $script:secondRepoPath update-ref refs/remotes/origin/main $secondBaseSha
    }

    It 'lists ahead commits and dirty worktrees without deleting or modifying them' {
        $inventory = @(& $scriptPath -RepositoryPath $script:repoPath -BaseRef 'refs/remotes/origin/main')
        $feature = $inventory | Where-Object Branch -eq 'feature/active'

        ($null -ne $feature) | Should Be $true
        $feature.Ahead | Should Be 1
        $feature.IsDirty | Should Be $true
        $feature.Disposition | Should Be 'KEEP_DIRTY_UNMERGED'
        ($inventory | Where-Object Branch -eq 'main').IsDirty | Should Be $false
        ($inventory | Where-Object Branch -eq 'main').Disposition | Should Be 'KEEP_BASE_WORKTREE'
        ($inventory | Where-Object Branch -eq 'feature/prunable').Disposition | Should Be 'INSPECT_PRUNABLE_NO_AUTO_DELETE'
        (Test-Path -LiteralPath $script:featurePath) | Should Be $true
        (Get-Content -LiteralPath (Join-Path $script:featurePath 'README.md') -Raw).Trim() | Should Be 'dirty change'
    }

    It 'reports combined worktree count and an exceeded cap across repositories without mutation' {
        $inventory = @(& $scriptPath -RepositoryPath $script:repoPath -AdditionalRepositoryPath $script:secondRepoPath -BaseRef 'refs/remotes/origin/main' -MaxCombinedWorktrees 1)

        $inventory.Count | Should Be 4
        $inventory[0].CombinedWorktreeCount | Should Be 4
        $inventory[0].MaxCombinedWorktrees | Should Be 1
        $inventory[0].CombinedLimitExceeded | Should Be $true
        (Test-Path -LiteralPath (Join-Path $script:secondRepoPath 'README.md')) | Should Be $true
    }
}

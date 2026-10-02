$ErrorActionPreference = 'Stop'

function Assert-Throws {
    param([scriptblock] $Action)
    $threw = $false
    try { & $Action | Out-Null } catch { $threw = $true }
    if (-not $threw) { throw 'Expected the command to reject this unsafe configuration.' }
}

function Assert-Matches {
    param([string] $Text, [string] $Pattern)
    if ($Text -notmatch $Pattern) { throw "Expected output to match '$Pattern'." }
}

function Assert-DoesNotMatch {
    param([string] $Text, [string] $Pattern)
    if ($Text -match $Pattern) { throw "Output unexpectedly matched '$Pattern'." }
}

Describe 'verify-local-ci.ps1' {
    BeforeAll {
        $scriptPath = Join-Path $PSScriptRoot '..\scripts\verify-local-ci.ps1'
    }

    It 'shows the same non-destructive verification gates as CI without running them' {
        $output = & $scriptPath -PlanOnly
        $text = $output -join [Environment]::NewLine
        $workflow = Get-Content (Join-Path $PSScriptRoot '..\.github\workflows\ci.yml') -Raw

        Assert-Matches $text 'cargo \+1\.98\.1 fmt --all --check'
        Assert-Matches $text 'cargo \+1\.98\.1 clippy --workspace --all-targets -- -D warnings'
        Assert-Matches $text 'cargo \+1\.98\.1 test --workspace --features test-support'
        Assert-Matches $text 'python -m unittest discover -s tests -p test_\*_contract\.py -v'
        Assert-Matches $text 'python contracts/validate_fixtures\.py'
        Assert-Matches $text 'Invoke-Pester -Path tests/run-local-e0-e2e\.Tests\.ps1 -PassThru'
        Assert-DoesNotMatch $text 'postgres_run_events'
        Assert-DoesNotMatch $text 'PULSO_TEST_POSTGRES_URL'
        Assert-Matches $workflow 'Invoke-Pester -Path tests/run-local-ci\.Tests\.ps1 -EnableExit'
    }

    It 'does not plan destructive database checks without explicit opt-in and a local test database' {
        Assert-Throws { & $scriptPath -PlanOnly -PostgresTestUrl 'postgres://user:pass@db.example.com/pulso_test' }
        Assert-Throws { & $scriptPath -PlanOnly -IncludePostgres -PostgresTestUrl 'postgres://user:pass@localhost/pulso_test' }
        Assert-Throws { & $scriptPath -PlanOnly -IncludePostgres -AllowDestructiveTestDb -PostgresTestUrl 'postgres://user:pass@localhost/production' }
    }

    It 'includes each database migration gate only after explicit local destructive-test opt-in' {
        $output = & $scriptPath -PlanOnly -IncludePostgres -AllowDestructiveTestDb -PostgresTestUrl 'postgres://user:pass@localhost/pulso_test'
        $text = $output -join [Environment]::NewLine

        Assert-Matches $text 'migration_enforces_cas_immutability_and_memory_tombstones'
        Assert-Matches $text 'migration_persists_model_attempt_cas_and_unknown_across_postgres_restart'
        Assert-Matches $text 'postgres_platform_observations'
        Assert-Matches $text 'postgres_run_events'
        Assert-Matches $text 'postgres_memory_temporal_receipts'
        Assert-DoesNotMatch $text 'user:pass'
    }

    It 'stops and reports failure instead of advancing after a non-zero command' {
        $bin = Join-Path ([System.IO.Path]::GetTempPath()) ('pulso-ci-failfast-' + [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Path $bin -Force | Out-Null
        $priorPath = $env:PATH
        try {
            Set-Content -LiteralPath (Join-Path $bin 'cargo.cmd') -Value "@echo off`r`nexit /b 7" -Encoding Ascii
            $env:PATH = $bin + ';' + $env:PATH
            $message = ''
            try { & $scriptPath } catch { $message = $_.Exception.Message }
            Assert-Matches $message 'Pinned Rust toolchain failed with exit code 7'
        }
        finally {
            $env:PATH = $priorPath
            Remove-Item -LiteralPath $bin -Recurse -Force
        }
    }
}

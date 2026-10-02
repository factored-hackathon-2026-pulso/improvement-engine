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
        $ipv6Output = & $scriptPath -PlanOnly -IncludePostgres -AllowDestructiveTestDb -PostgresTestUrl 'postgres://user:pass@[::1]:5432/pulso_test'
        $ipv6Text = $ipv6Output -join [Environment]::NewLine

        Assert-Matches $text 'migration_enforces_cas_immutability_and_memory_tombstones'
        Assert-Matches $text 'migration_persists_model_attempt_cas_and_unknown_across_postgres_restart'
        Assert-Matches $text 'postgres_platform_observations'
        Assert-Matches $text 'postgres_run_events'
        Assert-Matches $text 'postgres_memory_temporal_receipts'
        Assert-DoesNotMatch $text 'user:pass'
        Assert-Matches $ipv6Text 'postgres_memory_temporal_receipts'
        Assert-DoesNotMatch $ipv6Text 'user:pass'
    }

    It 'restores prior database environment values when an opted-in database command fails' {
        $bin = Join-Path ([System.IO.Path]::GetTempPath()) ('pulso-ci-db-failfast-' + [guid]::NewGuid().ToString('N'))
        $moduleRoot = Join-Path $bin 'modules'
        $fakePester = Join-Path $moduleRoot 'Pester\3.4.0'
        $countFile = Join-Path $bin 'count.txt'
        $priorPath = $env:PATH
        $priorUrl = $env:PULSO_TEST_POSTGRES_URL
        $priorConsent = $env:PULSO_ALLOW_DESTRUCTIVE_TEST_DB
        $priorCounter = $env:PULSO_LOCAL_CI_COUNT_FILE
        $priorModulePath = $env:PSModulePath
        try {
            New-Item -ItemType Directory -Path $bin, $fakePester -Force | Out-Null
            $cargoShim = @'
@echo off
set "countFile=%PULSO_LOCAL_CI_COUNT_FILE%"
set "count=0"
if exist "%countFile%" set /p count=<"%countFile%"
set /a count+=1
>"%countFile%" echo %count%
if %count% GEQ 6 exit /b 7
exit /b 0
'@
            Set-Content -LiteralPath (Join-Path $bin 'cargo.cmd') -Value $cargoShim -Encoding Ascii
            Set-Content -LiteralPath (Join-Path $bin 'python.cmd') -Value "@echo off`r`nexit /b 0" -Encoding Ascii
            Set-Content -LiteralPath (Join-Path $fakePester 'Pester.psd1') -Value "@{ RootModule = 'Pester.psm1'; ModuleVersion = '3.4.0'; GUID = 'a9cd5f0a-8895-4cd4-9a20-3ce1e92d99ad' }" -Encoding Ascii
            Set-Content -LiteralPath (Join-Path $fakePester 'Pester.psm1') -Value "function Invoke-Pester { [pscustomobject]@{ FailedCount = 0 } }; Export-ModuleMember -Function Invoke-Pester" -Encoding Ascii
            $env:PATH = $bin + ';' + $env:PATH
            $env:PULSO_LOCAL_CI_COUNT_FILE = $countFile
            $env:PSModulePath = $moduleRoot + ';' + $env:PSModulePath
            $env:PULSO_TEST_POSTGRES_URL = 'postgres://prior-sentinel'
            $env:PULSO_ALLOW_DESTRUCTIVE_TEST_DB = 'prior-consent-sentinel'

            $quotedScript = $scriptPath.Replace("'", "''")
            $childScript = @"
`$ErrorActionPreference = 'Continue'
try { & '$quotedScript' -IncludePostgres -AllowDestructiveTestDb -PostgresTestUrl 'postgres://user:pass@127.0.0.1:5432/pulso_test' } catch { "ERROR=`$(`$_.Exception.Message)" }
"URL=`$env:PULSO_TEST_POSTGRES_URL"
"CONSENT=`$env:PULSO_ALLOW_DESTRUCTIVE_TEST_DB"
exit 0
"@
            $encodedScript = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($childScript))
            $childOutput = (& pwsh -NoProfile -EncodedCommand $encodedScript 2>&1 | Out-String)
            Assert-Matches $childOutput 'ERROR=U02 immutable artifact migration failed with exit code 7'
            if ($childOutput -notmatch 'URL=postgres://prior-sentinel') {
                throw 'The preflight did not restore the caller PostgreSQL URL after a database failure.'
            }
            if ($childOutput -notmatch 'CONSENT=prior-consent-sentinel') {
                throw 'The preflight did not restore the caller consent value after a database failure.'
            }
        }
        finally {
            $env:PATH = $priorPath
            $env:PSModulePath = $priorModulePath
            $env:PULSO_TEST_POSTGRES_URL = $priorUrl
            $env:PULSO_ALLOW_DESTRUCTIVE_TEST_DB = $priorConsent
            $env:PULSO_LOCAL_CI_COUNT_FILE = $priorCounter
            $global:LASTEXITCODE = 0
            if (Test-Path -LiteralPath $bin) {
                Remove-Item -LiteralPath $bin -Recurse -Force
            }
        }
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
            $global:LASTEXITCODE = 0
            Remove-Item -LiteralPath $bin -Recurse -Force
        }
    }
}

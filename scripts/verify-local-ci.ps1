[CmdletBinding()]
param(
    [switch] $PlanOnly,
    [switch] $IncludePostgres,
    [switch] $AllowDestructiveTestDb,
    [string] $PostgresTestUrl
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

if ($PostgresTestUrl -and -not $IncludePostgres) {
    throw 'PostgresTestUrl is accepted only with IncludePostgres.'
}
if ($AllowDestructiveTestDb -and -not $IncludePostgres) {
    throw 'AllowDestructiveTestDb requires IncludePostgres.'
}
if ($IncludePostgres -and (-not $AllowDestructiveTestDb -or -not $PostgresTestUrl)) {
    throw 'PostgreSQL tests reset their explicitly isolated database. Pass IncludePostgres, AllowDestructiveTestDb, and a local pulso_test URL.'
}

if ($IncludePostgres) {
    $parsedUrl = $null
    if (-not [uri]::TryCreate($PostgresTestUrl, [System.UriKind]::Absolute, [ref] $parsedUrl)) {
        throw 'PostgresTestUrl must be an absolute PostgreSQL URL for the local pulso_test database.'
    }
    $localHosts = @('localhost', '127.0.0.1', '::1')
    $hostName = $parsedUrl.Host.TrimStart('[').TrimEnd(']')
    $databaseName = $parsedUrl.AbsolutePath.Trim('/')
    if ($parsedUrl.Scheme -notin @('postgres', 'postgresql') -or
        $hostName -notin $localHosts -or
        $databaseName -ne 'pulso_test' -or
        $parsedUrl.Query -or $parsedUrl.Fragment) {
        throw 'Refusing destructive PostgreSQL tests unless the URL targets local database pulso_test (without query or fragment).'
    }
}

$steps = @(
    [pscustomobject]@{ Name = 'Pinned Rust toolchain'; File = 'cargo'; Args = @('+1.98.1', '--version'); Database = $false },
    [pscustomobject]@{ Name = 'Formatting'; File = 'cargo'; Args = @('+1.98.1', 'fmt', '--all', '--check'); Database = $false },
    [pscustomobject]@{ Name = 'Clippy'; File = 'cargo'; Args = @('+1.98.1', 'clippy', '--workspace', '--all-targets', '--', '-D', 'warnings'); Database = $false },
    [pscustomobject]@{ Name = 'Rust unit tests'; File = 'cargo'; Args = @('+1.98.1', 'test', '--workspace', '--lib', '--features', 'test-support'); Database = $false },
    [pscustomobject]@{ Name = 'Rust integration tests'; File = 'cargo'; Args = @('+1.98.1', 'test', '--workspace', '--features', 'test-support'); Database = $false },
    [pscustomobject]@{ Name = 'Python contract tests'; File = 'python'; Args = @('-m', 'unittest', 'discover', '-s', 'tests', '-p', 'test_*_contract.py', '-v'); Database = $false },
    [pscustomobject]@{ Name = 'Contract fixture validation'; File = 'python'; Args = @('contracts/validate_fixtures.py'); Database = $false }
)

if ($IsWindows) {
    $steps += [pscustomobject]@{ Name = 'Pinned Pester tests'; File = 'pester'; Args = @('tests/run-local-e0-e2e.Tests.ps1', 'tests/run-local-ci.Tests.ps1'); Database = $false }
}

if ($IncludePostgres) {
    $steps += @(
        [pscustomobject]@{ Name = 'U02 immutable artifact migration'; File = 'cargo'; Args = @('+1.98.1', 'test', '--locked', '--workspace', 'migration_enforces_cas_immutability_and_memory_tombstones', '--', '--ignored'); Database = $true },
        [pscustomobject]@{ Name = 'U10 model attempt migration'; File = 'cargo'; Args = @('+1.98.1', 'test', '--locked', '--workspace', '--features', 'test-support', 'migration_persists_model_attempt_cas_and_unknown_across_postgres_restart', '--', '--ignored'); Database = $true },
        [pscustomobject]@{ Name = 'U29 platform observation RLS and retention'; File = 'cargo'; Args = @('+1.98.1', 'test', '--locked', '-p', 'improvement-engine-core', '--test', 'postgres_platform_observations', 'treated_batch_survives_reconnect_without_cross_tenant_read_or_double_insert', '--', '--ignored'); Database = $true },
        [pscustomobject]@{ Name = 'U07 durable run-event ledger'; File = 'cargo'; Args = @('+1.98.1', 'test', '--locked', '-p', 'improvement-engine-core', '--test', 'postgres_run_events', '--', '--ignored'); Database = $true },
        [pscustomobject]@{ Name = 'U24 V2 tenant-scoped sequence timeline'; File = 'cargo'; Args = @('+1.98.1', 'test', '--locked', '-p', 'improvement-engine-core', '--lib', 'run_timeline_v2::tests::authenticated_scope_sequence_pagination_and_sanitization_use_real_postgres', '--', '--ignored'); Database = $true },
        [pscustomobject]@{ Name = 'P4 temporal memory receipts'; File = 'cargo'; Args = @('+1.98.1', 'test', '--locked', '-p', 'improvement-engine-core', '--test', 'postgres_memory_temporal_receipts', '--', '--ignored', '--exact', 'temporal_use_receipt_is_bound_to_event_scope_cutoff_and_current_head'); Database = $true }
    )
}

if ($PlanOnly) {
    foreach ($step in $steps) {
        if ($step.File -eq 'pester') {
            foreach ($testPath in $step.Args) {
                Write-Output "Invoke-Pester -Path $testPath -PassThru"
            }
            continue
        }
        $command = "$($step.File) $($step.Args -join ' ')"
        if ($step.Database) { $command = "PULSO_TEST_POSTGRES_URL=<redacted local pulso_test> PULSO_ALLOW_DESTRUCTIVE_TEST_DB=1 $command" }
        Write-Output $command
    }
    return
}

$priorUrl = $env:PULSO_TEST_POSTGRES_URL
$priorConsent = $env:PULSO_ALLOW_DESTRUCTIVE_TEST_DB
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
Push-Location $repoRoot
try {
    for ($index = 0; $index -lt $steps.Count; $index++) {
        $step = $steps[$index]
        Write-Output "[$($index + 1)/$($steps.Count)] $($step.Name)"

        if ($step.Database) {
            $env:PULSO_TEST_POSTGRES_URL = $PostgresTestUrl
            $env:PULSO_ALLOW_DESTRUCTIVE_TEST_DB = '1'
        }

        if ($step.File -eq 'pester') {
            $requiredPester = [version] '3.4.0'
            $availablePester = Get-Module -ListAvailable -Name Pester |
                Where-Object { $_.Version -eq $requiredPester } |
                Select-Object -First 1
            if ($null -eq $availablePester) {
                throw "Pester $requiredPester must already be installed; the preflight will not install or upgrade it."
            }
            Import-Module Pester -RequiredVersion $requiredPester -Force
            if ((Get-Module Pester).Version -ne $requiredPester) {
                throw "Loaded Pester version does not match required version $requiredPester."
            }
            foreach ($testPath in $step.Args) {
                $pesterResult = Invoke-Pester -Path $testPath -PassThru
                if ($pesterResult.FailedCount -gt 0) { throw "Pester failed for $testPath." }
            }
        }
        else {
            & $step.File @($step.Args)
            if ($LASTEXITCODE -ne 0) { throw "$($step.Name) failed with exit code $LASTEXITCODE." }
        }

        if ($step.Database) {
            $env:PULSO_TEST_POSTGRES_URL = $priorUrl
            $env:PULSO_ALLOW_DESTRUCTIVE_TEST_DB = $priorConsent
        }
    }
}
finally {
    $env:PULSO_TEST_POSTGRES_URL = $priorUrl
    $env:PULSO_ALLOW_DESTRUCTIVE_TEST_DB = $priorConsent
    Pop-Location
}

Write-Output 'Local CI preflight passed for the selected gates.'

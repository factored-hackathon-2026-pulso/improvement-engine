$ErrorActionPreference = 'Stop'

function Assert-True {
    param([bool] $Condition, [string] $Message)
    if (-not $Condition) {
        throw $Message
    }
}

function Assert-Contains {
    param([string] $Text, [string] $Pattern)
    if ($Text -notmatch $Pattern) {
        throw "Expected output to match safe pattern '$Pattern'."
    }
}

function Assert-DoesNotContain {
    param([string] $Text, [string] $Pattern)
    if ($Text -match $Pattern) {
        throw 'Output contains a value that should have been suppressed.'
    }
}

Describe 'run-local-e0-e2e.ps1' {
    BeforeAll {
        $scriptPath = Join-Path $PSScriptRoot '..\scripts\run-local-e0-e2e.ps1'
        $script:fixtureRoot = Join-Path ([System.IO.Path]::GetTempPath()) ('pulso-e0-script-test-' + [guid]::NewGuid().ToString('N'))
        $script:inputRoot = Join-Path $script:fixtureRoot 'input'
        $script:outputRoot = Join-Path $script:fixtureRoot 'runs'
        $script:fakeBin = Join-Path $script:fixtureRoot 'bin'
        New-Item -ItemType Directory -Path $script:inputRoot, $script:fakeBin -Force | Out-Null
        Set-Content -LiteralPath (Join-Path $script:inputRoot 'sentinel.txt') -Value 'input must remain unchanged'

        $cargoShim = @'
@echo off
setlocal EnableExtensions
if defined PULSO_E2E_TEST_ARGS_FILE echo %*>"%PULSO_E2E_TEST_ARGS_FILE%"
if "%PULSO_E2E_TEST_FAIL%"=="1" exit /b 7
set "output="
:parse
if "%~1"=="" goto write
if "%~1"=="--output" (
  set "output=%~2"
  shift
  shift
  goto parse
)
shift
goto parse
:write
if not defined output exit /b 90
if not exist "%output%" mkdir "%output%"
mkdir "%output%\fixture-run"
if "%PULSO_E2E_TEST_JSON_MODE%"=="missing_holdout" (
  > "%output%\fixture-run\result.json" echo {"terminal_status":"complete_simulated","discovery_case_count":200,"excluded_replay_case_count":1800,"signals":[],"proposal":null,"formal_route":"do_nothing"}
  exit /b 0
)
if "%PULSO_E2E_TEST_JSON_MODE%"=="malformed" (
  > "%output%\fixture-run\result.json" echo {"private_customer_id":"DO_NOT_PRINT_THIS",}
  exit /b 0
)
> "%output%\fixture-run\result.json" echo {"terminal_status":"complete_simulated","discovery_case_count":200,"excluded_replay_case_count":1800,"signals":[{"metric_id":"e0_technical_error_rate","numerator":0,"denominator":187,"missing":13},{"metric_id":"e0_recurring_copilot_query_cases","numerator":154,"denominator":200,"missing":0}],"e0_recurrence_holdout":{"status":"replicated","queried_case_count":1539,"matching_case_count":1433,"interpretation":"descriptive_recurrence_only_no_causal_or_outcome_claim"},"proposal":{"status":"simulated_unverified","execution_status":"not_executed","private_text":"DO_NOT_PRINT_THIS"},"formal_route":"do_nothing","private_customer_id":"DO_NOT_PRINT_THIS"}
exit /b 0
'@
        Set-Content -LiteralPath (Join-Path $script:fakeBin 'cargo.cmd') -Value $cargoShim -Encoding Ascii
        $script:priorPath = $env:PATH
        $env:PATH = $script:fakeBin + ';' + $env:PATH
        $script:argsLog = Join-Path $script:fixtureRoot 'cargo-args.txt'
        $env:PULSO_E2E_TEST_ARGS_FILE = $script:argsLog
        Remove-Item Env:PULSO_E2E_TEST_FAIL -ErrorAction SilentlyContinue
        Remove-Item Env:PULSO_E2E_TEST_JSON_MODE -ErrorAction SilentlyContinue
    }

    AfterAll {
        $env:PATH = $script:priorPath
        Remove-Item Env:PULSO_E2E_TEST_ARGS_FILE -ErrorAction SilentlyContinue
        Remove-Item Env:PULSO_E2E_TEST_FAIL -ErrorAction SilentlyContinue
        Remove-Item Env:PULSO_E2E_TEST_JSON_MODE -ErrorAction SilentlyContinue
        if (Test-Path -LiteralPath $script:fixtureRoot) {
            Remove-Item -LiteralPath $script:fixtureRoot -Recurse -Force
        }
    }

    It 'runs locked and offline and prints only approved aggregate fields' {
        $output = & $scriptPath -InputPath $script:inputRoot -OutputPath $script:outputRoot -ObservedCutoff '2026-10-02T18:00:00Z'
        $text = $output -join [Environment]::NewLine
        Assert-Contains $text 'Status: complete_simulated'
        Assert-Contains $text 'Cases: discovery=200; replay_excluded=1800'
        Assert-Contains $text 'e0_technical_error_rate: 0/187; missing=13'
        Assert-Contains $text 'e0_recurring_copilot_query_cases: 154/200; missing=0'
        Assert-Contains $text 'Proposal: status=simulated_unverified; execution=not_executed'
        Assert-Contains $text 'Holdout: status=replicated; matches=1433/1539; descriptive_only'
        Assert-Contains $text 'Formal route: do_nothing'
        Assert-DoesNotContain $text 'DO_NOT_PRINT_THIS|sentinel|fixture-run|pulso-e0-script-test'

        $args = Get-Content -LiteralPath $script:argsLog -Raw
        Assert-Contains $args '--locked.*--offline'
        Assert-Contains $args '-p improvement-engine-runner'
        Assert-Contains $args 'local-sim.*--mode local-simulation.*--source e0'
        Assert-Contains $args ([regex]::Escape($script:inputRoot))
        Assert-Contains $args ([regex]::Escape($script:outputRoot))
        Assert-Contains $args '2026-10-02T18:00:00Z'
        Assert-Contains $args '--arranque-cases 200.*--min-recurring-query-cases 20'
        Assert-True (Test-Path -LiteralPath $script:inputRoot) 'Input directory was removed.'
        if ((Get-Content -LiteralPath (Join-Path $script:inputRoot 'sentinel.txt') -Raw).Trim() -ne 'input must remain unchanged') {
            throw 'Input file was modified.'
        }
    }

    It 'prints none when holdout is missing and suppresses malformed JSON content' {
        $env:PULSO_E2E_TEST_JSON_MODE = 'missing_holdout'
        $missingOutput = & $scriptPath -InputPath $script:inputRoot -OutputPath (Join-Path $script:fixtureRoot 'missing-output') -ObservedCutoff '2026-10-02T18:00:00Z'
        $missingText = $missingOutput -join [Environment]::NewLine
        Assert-Contains $missingText 'Holdout: none'
        Assert-DoesNotContain $missingText 'DO_NOT_PRINT_THIS|private_customer_id'

        $env:PULSO_E2E_TEST_JSON_MODE = 'malformed'
        $malformedRejected = $false
        $malformedMessage = ''
        try {
            $null = & $scriptPath -InputPath $script:inputRoot -OutputPath (Join-Path $script:fixtureRoot 'malformed-output') -ObservedCutoff '2026-10-02T18:00:00Z'
        }
        catch {
            $malformedRejected = $true
            $malformedMessage = $_.Exception.Message
        }
        finally {
            Remove-Item Env:PULSO_E2E_TEST_JSON_MODE -ErrorAction SilentlyContinue
        }
        Assert-True $malformedRejected 'Malformed result JSON was unexpectedly accepted.'
        Assert-DoesNotContain $malformedMessage 'DO_NOT_PRINT_THIS|private_customer_id'
    }

    It 'refuses an existing output directory before invoking Cargo' {
        $existing = Join-Path $script:fixtureRoot 'existing-output'
        New-Item -ItemType Directory -Path $existing -Force | Out-Null
        $sentinel = Join-Path $existing 'keep.txt'
        Set-Content -LiteralPath $sentinel -Value 'do not overwrite'
        Remove-Item -LiteralPath $script:argsLog -ErrorAction SilentlyContinue

        $thrown = $false
        try { & $scriptPath -InputPath $script:inputRoot -OutputPath $existing -ObservedCutoff '2026-10-02T18:00:00Z' | Out-Null } catch { $thrown = $true }
        Assert-True $thrown 'Existing output path was not rejected.'
        Assert-True (Test-Path -LiteralPath $sentinel) 'Existing output was modified.'
        Assert-True (-not (Test-Path -LiteralPath $script:argsLog)) 'Cargo ran despite existing output.'
    }

    It 'refuses an output path inside the input tree' {
        $nestedOutput = Join-Path $script:inputRoot 'runs'
        $thrown = $false
        try { & $scriptPath -InputPath $script:inputRoot -OutputPath $nestedOutput -ObservedCutoff '2026-10-02T18:00:00Z' | Out-Null } catch { $thrown = $true }
        Assert-True $thrown 'Nested output path was not rejected.'
        Assert-True (-not (Test-Path -LiteralPath $nestedOutput)) 'Nested output path was created.'
    }

    It 'refuses an output path through a junction that aliases the input tree' {
        $junction = Join-Path ([System.IO.Path]::GetTempPath()) ('pulso-e0-junction-' + [guid]::NewGuid().ToString('N'))
        $junctionCreated = $false
        try {
            New-Item -ItemType Junction -Path $junction -Target $script:inputRoot -ErrorAction Stop | Out-Null
            $junctionCreated = $true
        }
        catch {
            throw 'Could not create a temporary Windows junction; the reparse-point isolation regression cannot be exercised.'
        }

        try {
            Remove-Item -LiteralPath $script:argsLog -ErrorAction SilentlyContinue
            $junctionOutput = Join-Path $junction 'runs'
            $thrown = $false
            try { & $scriptPath -InputPath $script:inputRoot -OutputPath $junctionOutput -ObservedCutoff '2026-10-02T18:00:00Z' | Out-Null } catch { $thrown = $true }
            Assert-True $thrown 'Output through an input-alias junction was not rejected.'
            Assert-True (-not (Test-Path -LiteralPath $script:argsLog)) 'Cargo ran before the reparse point was rejected.'
            Assert-True (-not (Test-Path -LiteralPath (Join-Path $script:inputRoot 'runs'))) 'Junction output contaminated the input tree.'
        }
        finally {
            if ($junctionCreated -and (Test-Path -LiteralPath $junction)) {
                [System.IO.Directory]::Delete($junction, $false)
            }
        }
    }

    It 'refuses an input path that traverses a junction alias' {
        $junction = Join-Path ([System.IO.Path]::GetTempPath()) ('pulso-e0-input-junction-' + [guid]::NewGuid().ToString('N'))
        $junctionCreated = $false
        try {
            New-Item -ItemType Junction -Path $junction -Target $script:inputRoot -ErrorAction Stop | Out-Null
            $junctionCreated = $true
        }
        catch {
            throw 'Could not create a temporary Windows junction; the input reparse-point regression cannot be exercised.'
        }

        try {
            Remove-Item -LiteralPath $script:argsLog -ErrorAction SilentlyContinue
            $inputAliasOutput = Join-Path $script:fixtureRoot 'input-alias-output'
            $thrown = $false
            try { & $scriptPath -InputPath $junction -OutputPath $inputAliasOutput -ObservedCutoff '2026-10-02T18:00:00Z' | Out-Null } catch { $thrown = $true }
            Assert-True $thrown 'Input through a junction was not rejected.'
            Assert-True (-not (Test-Path -LiteralPath $script:argsLog)) 'Cargo ran before the input reparse point was rejected.'
            Assert-True (-not (Test-Path -LiteralPath $inputAliasOutput)) 'Output was created from a junction-backed input.'
        }
        finally {
            if ($junctionCreated -and (Test-Path -LiteralPath $junction)) {
                [System.IO.Directory]::Delete($junction, $false)
            }
        }
    }

    It 'allows a new output sibling whose name shares an input prefix' {
        $prefixSibling = Join-Path $script:fixtureRoot 'input-runs'
        $output = & $scriptPath -InputPath $script:inputRoot -OutputPath $prefixSibling -ObservedCutoff '2026-10-02T18:00:00Z'
        Assert-Contains ($output -join [Environment]::NewLine) 'Status: complete_simulated'
        Assert-True (Test-Path -LiteralPath (Join-Path $prefixSibling 'fixture-run\result.json')) 'Sibling output was not written.'
    }

    It 'requires an explicit observed cutoff' {
        Remove-Item -LiteralPath $script:argsLog -ErrorAction SilentlyContinue
        $thrown = $false
        try { & $scriptPath -InputPath $script:inputRoot -OutputPath $script:outputRoot | Out-Null } catch { $thrown = $true }
        Assert-True $thrown 'Missing cutoff was not rejected.'
        Assert-True (-not (Test-Path -LiteralPath $script:argsLog)) 'Cargo ran without an explicit cutoff.'
    }

    It 'does not print Cargo diagnostics if the offline command fails' {
        $env:PULSO_E2E_TEST_FAIL = '1'
        $failedOutput = Join-Path $script:fixtureRoot 'failed-runs'
        $failure = ''
        try {
            & $scriptPath -InputPath $script:inputRoot -OutputPath $failedOutput -ObservedCutoff '2026-10-02T18:00:00Z' | Out-Null
        }
        catch {
            $failure = $_.Exception.Message
        }
        Assert-Contains $failure 'exit code 7'
        Assert-DoesNotContain $failure 'DO_NOT_PRINT_THIS|sentinel|fixture-run'
        Assert-True (-not (Test-Path -LiteralPath $failedOutput)) 'Failed fake Cargo run wrote output.'
        Remove-Item Env:PULSO_E2E_TEST_FAIL -ErrorAction SilentlyContinue
    }
}

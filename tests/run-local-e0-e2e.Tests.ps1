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
        $script:originalInputRoot = Join-Path $script:fixtureRoot 'original-input'
        $script:outputRoot = Join-Path $script:fixtureRoot 'runs'
        $script:fakeBin = Join-Path $script:fixtureRoot 'bin'
        New-Item -ItemType Directory -Path $script:inputRoot, $script:originalInputRoot, $script:fakeBin -Force | Out-Null
        Set-Content -LiteralPath (Join-Path $script:inputRoot 'sentinel.txt') -Value 'input must remain unchanged'

        $cargoShim = @'
@echo off
setlocal EnableExtensions
if defined PULSO_E2E_TEST_ARGS_FILE (
  if "%PULSO_E2E_TEST_APPEND_ARGS%"=="1" (echo %*>>"%PULSO_E2E_TEST_ARGS_FILE%") else echo %*>"%PULSO_E2E_TEST_ARGS_FILE%"
)
set "PULSO_E2E_TEST_SOURCE_VALIDATE="
for %%A in (%*) do if "%%~A"=="source" set "PULSO_E2E_TEST_SOURCE_VALIDATE=1"
if defined PULSO_E2E_TEST_FAIL_VALIDATE if "%~9"=="source" exit /b 8
if defined PULSO_E2E_TEST_SOURCE_VALIDATE (
  >&1 echo {"schema_version":1,"source_kind":"enriched_history","validation_status":"valid","contract_version":"0.5.1","manifest_digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","snapshot_digest":"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}
  exit /b 0
)
>&2 echo {"schema_version":1,"event":"run_progress","phase":"source_preparation","stage":"inventory","status":"started","files_completed":0,"files_total":0,"bytes_completed":0,"bytes_total":0,"elapsed_ms":0}
>&2 echo {"schema_version":1,"event":"run_progress","phase":"source_preparation","stage":"inventory","status":"completed","files_completed":2,"files_total":2,"bytes_completed":52,"bytes_total":52,"elapsed_ms":12}
>&2 echo private_customer_id=DO_NOT_PRINT_THIS
if "%PULSO_E2E_TEST_FAIL%"=="1" exit /b 7
set "output="
set "source="
:parse
if "%~1"=="" goto write
if "%~1"=="--output" (
  set "output=%~2"
  shift
  shift
  goto parse
)
if "%~1"=="--source" (
  set "source=%~2"
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
if "%source%"=="original" (
if "%PULSO_E2E_TEST_JSON_MODE%"=="forged_original_portfolio" (
    > "%output%\fixture-run\result.json" echo {"terminal_status":"snapshot_projection_complete","source_kind":"original_bank","discovery_case_count":0,"excluded_replay_case_count":0,"signals":[],"proposal":null,"local_simulation_portfolio":{"source_family":"e0","authority":"simulator_only","status":"candidates_ready","dispositions":[{"metric_id":"e0_technical_error_rate","state":"candidate_for_simulated_investigation","signal_digest":"sha256_DO_NOT_PRINT_THIS","reason":"private_reason"}]},"snapshot_descriptive_envelope":null,"formal_route":"do_nothing","e0_recurrence_holdout":null}
    exit /b 0
  )
  if "%PULSO_E2E_TEST_JSON_MODE%"=="unsafe_original_candidate" (
    > "%output%\fixture-run\result.json" echo {"terminal_status":"snapshot_descriptive_finding_ready","source_kind":"original_bank","discovery_case_count":0,"excluded_replay_case_count":0,"signals":[],"proposal":{"status":"simulated_unverified","execution_status":"not_executed"},"snapshot_descriptive_envelope":{"agent_core_candidate":"dependency_blocked_snapshot_semantics","finding":{"coverage":"partial","temporal_basis":"literal_source_wall_clock_month","value_semantics":"final_extract_facts_only"},"proposal":{"status":"simulated_unverified","execution_status":"not_executed","publication_eligible":false,"formal_route":"do_nothing"}},"formal_route":"do_nothing","e0_recurrence_holdout":null}
    exit /b 0
  )
  > "%output%\fixture-run\result.json" echo {"terminal_status":"snapshot_descriptive_finding_ready","source_kind":"original_bank","discovery_case_count":0,"excluded_replay_case_count":0,"signals":[],"proposal":null,"snapshot_descriptive_envelope":{"agent_core_candidate":"dependency_blocked_snapshot_semantics","finding":{"coverage":"partial","temporal_basis":"literal_source_wall_clock_month","value_semantics":"final_extract_facts_only"},"proposal":{"status":"simulated_unverified","execution_status":"not_executed","publication_eligible":false,"formal_route":"do_nothing"}},"formal_route":"do_nothing","e0_recurrence_holdout":null}
  exit /b 0
)
if "%PULSO_E2E_TEST_JSON_MODE%"=="missing_holdout" (
  > "%output%\fixture-run\result.json" echo {"terminal_status":"complete_simulated","source_kind":"e0","discovery_case_count":200,"excluded_replay_case_count":null,"recurrence_measurement_status":"source_table_unavailable","signal":{"digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},"signals":[{"metric_id":"e0_technical_error_rate","numerator":0,"denominator":10,"missing":0,"digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},{"metric_id":"e0_tool_retry_case_rate","numerator":0,"denominator":10,"missing":0,"digest":"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}],"local_simulation_portfolio":{"source_family":"e0","authority":"simulator_only","status":"insufficient_evidence","dispositions":[{"metric_id":"e0_technical_error_rate","signal_digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","state":"not_qualified"},{"metric_id":"e0_tool_retry_case_rate","signal_digest":"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","state":"not_qualified"},{"metric_id":"e0_recurring_copilot_query_cases","signal_digest":null,"state":"unavailable"}],"candidate_signal_digests":[],"primary_signal_digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},"proposal":null,"formal_route":"do_nothing"}
  exit /b 0
)
if "%PULSO_E2E_TEST_JSON_MODE%"=="unsafe_missing_holdout" (
  > "%output%\fixture-run\result.json" echo {"terminal_status":"complete_simulated","source_kind":"e0","discovery_case_count":200,"excluded_replay_case_count":4,"recurrence_measurement_status":"source_table_unavailable","signal":{"digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},"signals":[{"metric_id":"e0_technical_error_rate","numerator":0,"denominator":10,"missing":0,"digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},{"metric_id":"e0_tool_retry_case_rate","numerator":0,"denominator":10,"missing":0,"digest":"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}],"local_simulation_portfolio":{"source_family":"e0","authority":"simulator_only","status":"insufficient_evidence","dispositions":[{"metric_id":"e0_technical_error_rate","signal_digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","state":"not_qualified"},{"metric_id":"e0_tool_retry_case_rate","signal_digest":"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","state":"not_qualified"},{"metric_id":"e0_recurring_copilot_query_cases","signal_digest":null,"state":"unavailable"}],"candidate_signal_digests":[],"primary_signal_digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},"proposal":null,"formal_route":"do_nothing"}
  exit /b 0
)
if "%PULSO_E2E_TEST_JSON_MODE%"=="insufficient_holdout" (
  > "%output%\fixture-run\result.json" echo {"terminal_status":"complete_simulated","source_kind":"e0","discovery_case_count":200,"excluded_replay_case_count":null,"recurrence_measurement_status":"source_table_unavailable","signal":{"digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},"signals":[{"metric_id":"e0_technical_error_rate","numerator":0,"denominator":10,"missing":1,"digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},{"metric_id":"e0_tool_retry_case_rate","numerator":0,"denominator":10,"missing":0,"digest":"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}],"local_simulation_portfolio":{"source_family":"e0","authority":"simulator_only","status":"insufficient_evidence","dispositions":[{"metric_id":"e0_technical_error_rate","signal_digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","state":"insufficient_evidence"},{"metric_id":"e0_tool_retry_case_rate","signal_digest":"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","state":"not_qualified"},{"metric_id":"e0_recurring_copilot_query_cases","signal_digest":null,"state":"unavailable"}],"candidate_signal_digests":[],"primary_signal_digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},"e0_recurrence_holdout":{"status":"insufficient_support","reproduction_case_count":null,"queried_case_count":null,"matching_case_count":null,"recurrence_rate_basis_points":null},"proposal":null,"formal_route":"do_nothing"}
  exit /b 0
)
if "%PULSO_E2E_TEST_JSON_MODE%"=="unsafe_insufficient_holdout" (
  > "%output%\fixture-run\result.json" echo {"terminal_status":"complete_simulated","source_kind":"e0","discovery_case_count":200,"excluded_replay_case_count":1800,"recurrence_measurement_status":"source_table_unavailable","signal":{"digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},"signals":[{"metric_id":"e0_technical_error_rate","numerator":0,"denominator":10,"missing":1,"digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},{"metric_id":"e0_tool_retry_case_rate","numerator":0,"denominator":10,"missing":0,"digest":"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}],"local_simulation_portfolio":{"source_family":"e0","authority":"simulator_only","status":"insufficient_evidence","dispositions":[{"metric_id":"e0_technical_error_rate","signal_digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","state":"insufficient_evidence"},{"metric_id":"e0_tool_retry_case_rate","signal_digest":"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","state":"not_qualified"},{"metric_id":"e0_recurring_copilot_query_cases","signal_digest":null,"state":"unavailable"}],"candidate_signal_digests":[],"primary_signal_digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},"e0_recurrence_holdout":{"status":"insufficient_support","reproduction_case_count":5,"queried_case_count":5,"matching_case_count":1,"recurrence_rate_basis_points":2000},"proposal":null,"formal_route":"do_nothing"}
  exit /b 0
)
if "%PULSO_E2E_TEST_JSON_MODE%"=="malformed" (
  > "%output%\fixture-run\result.json" echo {"private_customer_id":"DO_NOT_PRINT_THIS",}
  exit /b 0
)
if "%PULSO_E2E_TEST_JSON_MODE%"=="inconsistent_portfolio" (
  > "%output%\fixture-run\result.json" echo {"terminal_status":"complete_no_opportunity","source_kind":"e0","discovery_case_count":1,"excluded_replay_case_count":null,"recurrence_measurement_status":"observed","signal":{"digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},"signals":[{"metric_id":"e0_technical_error_rate","numerator":1,"denominator":10,"missing":0,"digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},{"metric_id":"e0_tool_retry_case_rate","numerator":0,"denominator":10,"missing":0,"digest":"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},{"metric_id":"e0_recurring_copilot_query_cases","numerator":0,"denominator":10,"missing":0,"digest":"sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"}],"local_simulation_portfolio":{"source_family":"e0","authority":"simulator_only","status":"insufficient_evidence","dispositions":[{"metric_id":"e0_technical_error_rate","signal_digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","state":"candidate_for_simulated_investigation","reason":"private_reason"},{"metric_id":"e0_tool_retry_case_rate","signal_digest":"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","state":"not_qualified"},{"metric_id":"e0_recurring_copilot_query_cases","signal_digest":"sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc","state":"not_qualified"}],"candidate_signal_digests":["sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"],"primary_signal_digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},"proposal":null,"formal_route":"do_nothing"}
  exit /b 0
)
if "%PULSO_E2E_TEST_JSON_MODE%"=="duplicate_disposition" (
  > "%output%\fixture-run\result.json" echo {"terminal_status":"complete_simulated","source_kind":"e0","discovery_case_count":1,"excluded_replay_case_count":null,"signals":[{"metric_id":"e0_technical_error_rate","numerator":1,"denominator":10,"missing":0,"digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}],"local_simulation_portfolio":{"source_family":"e0","authority":"simulator_only","status":"candidates_ready","dispositions":[{"metric_id":"e0_technical_error_rate","signal_digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","state":"candidate_for_simulated_investigation","reason":"measured_threshold_met"},{"metric_id":"e0_technical_error_rate","signal_digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","state":"candidate_for_simulated_investigation","reason":"measured_threshold_met"}],"candidate_signal_digests":["sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"],"primary_signal_digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},"proposal":null,"formal_route":"do_nothing"}
  exit /b 0
)
if "%PULSO_E2E_TEST_JSON_MODE%"=="missing_disposition" (
  > "%output%\fixture-run\result.json" echo {"terminal_status":"complete_simulated","source_kind":"e0","discovery_case_count":1,"excluded_replay_case_count":null,"signals":[{"metric_id":"e0_technical_error_rate","numerator":1,"denominator":10,"missing":0,"digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},{"metric_id":"e0_tool_retry_case_rate","numerator":1,"denominator":10,"missing":0,"digest":"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}],"local_simulation_portfolio":{"source_family":"e0","authority":"simulator_only","status":"candidates_ready","dispositions":[{"metric_id":"e0_technical_error_rate","signal_digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","state":"candidate_for_simulated_investigation","reason":"measured_threshold_met"}],"candidate_signal_digests":["sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"],"primary_signal_digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},"proposal":null,"formal_route":"do_nothing"}
  exit /b 0
)
if "%PULSO_E2E_TEST_JSON_MODE%"=="mismatched_digest" (
  > "%output%\fixture-run\result.json" echo {"terminal_status":"complete_simulated","source_kind":"e0","discovery_case_count":1,"excluded_replay_case_count":null,"signals":[{"metric_id":"e0_technical_error_rate","numerator":1,"denominator":10,"missing":0,"digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}],"local_simulation_portfolio":{"source_family":"e0","authority":"simulator_only","status":"candidates_ready","dispositions":[{"metric_id":"e0_technical_error_rate","signal_digest":"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","state":"candidate_for_simulated_investigation","reason":"measured_threshold_met"}],"candidate_signal_digests":["sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"],"primary_signal_digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},"proposal":null,"formal_route":"do_nothing"}
  exit /b 0
)
if "%PULSO_E2E_TEST_JSON_MODE%"=="paired_omission" (
  > "%output%\fixture-run\result.json" echo {"terminal_status":"complete_simulated","source_kind":"e0","discovery_case_count":1,"excluded_replay_case_count":null,"recurrence_measurement_status":"observed","signal":{"digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},"signals":[{"metric_id":"e0_technical_error_rate","numerator":1,"denominator":10,"missing":0,"digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},{"metric_id":"e0_tool_retry_case_rate","numerator":0,"denominator":10,"missing":0,"digest":"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}],"local_simulation_portfolio":{"source_family":"e0","authority":"simulator_only","status":"candidates_ready","dispositions":[{"metric_id":"e0_technical_error_rate","signal_digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","state":"candidate_for_simulated_investigation"},{"metric_id":"e0_tool_retry_case_rate","signal_digest":"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","state":"not_qualified"}],"candidate_signal_digests":["sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"],"primary_signal_digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},"proposal":null,"formal_route":"do_nothing"}
  exit /b 0
)
if "%PULSO_E2E_TEST_JSON_MODE%"=="wrong_primary" (
  > "%output%\fixture-run\result.json" echo {"terminal_status":"complete_simulated","source_kind":"e0","discovery_case_count":1,"excluded_replay_case_count":null,"recurrence_measurement_status":"source_table_unavailable","signal":{"digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},"signals":[{"metric_id":"e0_technical_error_rate","numerator":1,"denominator":10,"missing":0,"digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},{"metric_id":"e0_tool_retry_case_rate","numerator":1,"denominator":10,"missing":0,"digest":"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}],"local_simulation_portfolio":{"source_family":"e0","authority":"simulator_only","status":"candidates_ready","dispositions":[{"metric_id":"e0_technical_error_rate","signal_digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","state":"candidate_for_simulated_investigation"},{"metric_id":"e0_tool_retry_case_rate","signal_digest":"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","state":"candidate_for_simulated_investigation"},{"metric_id":"e0_recurring_copilot_query_cases","signal_digest":null,"state":"unavailable"}],"candidate_signal_digests":["sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"],"primary_signal_digest":"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},"proposal":null,"formal_route":"do_nothing"}
  exit /b 0
)
> "%output%\fixture-run\result.json" echo {"terminal_status":"complete_simulated","source_kind":"e0","discovery_case_count":200,"excluded_replay_case_count":null,"recurrence_measurement_status":"observed","signal":{"digest":"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},"signals":[{"metric_id":"e0_technical_error_rate","numerator":0,"denominator":187,"missing":13,"digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},{"metric_id":"e0_tool_retry_case_rate","numerator":10,"denominator":150,"missing":50,"digest":"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},{"metric_id":"e0_recurring_copilot_query_cases","numerator":154,"denominator":200,"missing":0,"digest":"sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"}],"local_simulation_portfolio":{"source_family":"e0","authority":"simulator_only","status":"candidates_ready","dispositions":[{"metric_id":"e0_technical_error_rate","signal_digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","state":"insufficient_evidence","reason":"partial_missing_observations"},{"metric_id":"e0_tool_retry_case_rate","signal_digest":"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","state":"candidate_for_simulated_investigation","reason":"measured_threshold_met_with_missing_observations"},{"metric_id":"e0_recurring_copilot_query_cases","signal_digest":"sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc","state":"candidate_for_simulated_investigation","reason":"measured_threshold_met"}],"candidate_signal_digests":["sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"],"primary_signal_digest":"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},"e0_recurrence_holdout":{"status":"replicated","queried_case_count":1539,"matching_case_count":1433,"interpretation":"descriptive_recurrence_only_no_causal_or_outcome_claim"},"proposal":{"status":"simulated_unverified","execution_status":"not_executed","private_text":"DO_NOT_PRINT_THIS"},"formal_route":"do_nothing","private_customer_id":"DO_NOT_PRINT_THIS"}
exit /b 0
'@
        Set-Content -LiteralPath (Join-Path $script:fakeBin 'cargo.cmd') -Value $cargoShim -Encoding Ascii
        $script:priorPath = $env:PATH
        $env:PATH = $script:fakeBin + ';' + $env:PATH
        $script:argsLog = Join-Path $script:fixtureRoot 'cargo-args.txt'
        $env:PULSO_E2E_TEST_ARGS_FILE = $script:argsLog
        Remove-Item Env:PULSO_E2E_TEST_FAIL -ErrorAction SilentlyContinue
        Remove-Item Env:PULSO_E2E_TEST_FAIL_VALIDATE -ErrorAction SilentlyContinue
        Remove-Item Env:PULSO_E2E_TEST_JSON_MODE -ErrorAction SilentlyContinue
        Remove-Item Env:PULSO_E2E_TEST_APPEND_ARGS -ErrorAction SilentlyContinue
    }

    AfterAll {
        $env:PATH = $script:priorPath
        Remove-Item Env:PULSO_E2E_TEST_ARGS_FILE -ErrorAction SilentlyContinue
        Remove-Item Env:PULSO_E2E_TEST_FAIL -ErrorAction SilentlyContinue
        Remove-Item Env:PULSO_E2E_TEST_FAIL_VALIDATE -ErrorAction SilentlyContinue
        Remove-Item Env:PULSO_E2E_TEST_JSON_MODE -ErrorAction SilentlyContinue
        Remove-Item Env:PULSO_E2E_TEST_APPEND_ARGS -ErrorAction SilentlyContinue
        if (Test-Path -LiteralPath $script:fixtureRoot) {
            Remove-Item -LiteralPath $script:fixtureRoot -Recurse -Force
        }
    }

    It 'runs locked and offline and prints only approved aggregate fields' {
        $output = & $scriptPath -InputPath $script:inputRoot -OutputPath $script:outputRoot -ObservedCutoff '2026-10-02T18:00:00Z'
        $text = $output -join [Environment]::NewLine
        Assert-Contains $text 'Status: complete_simulated'
        Assert-Contains $text 'Cases: discovery=200; replay_excluded=suppressed'
        Assert-Contains $text 'Portfolio: status=candidates_ready; candidates=2; not_qualified=0; insufficient=1; unavailable=0'
        Assert-Contains $text 'e0_technical_error_rate: 0/187; missing=13'
        Assert-Contains $text 'e0_tool_retry_case_rate: 10/150; missing=50'
        Assert-Contains $text 'e0_recurring_copilot_query_cases: 154/200; missing=0'
        Assert-Contains $text 'Proposal: status=simulated_unverified; execution=not_executed'
        Assert-Contains $text 'Holdout: status=replicated; matches=1433/1539; descriptive_only'
        Assert-Contains $text 'Formal route: do_nothing'
        $portfolioLine = @($text -split [Environment]::NewLine | Where-Object { $_ -like 'Portfolio:*' })
        Assert-True ($portfolioLine.Count -eq 1) 'Expected one sanitized E0 portfolio summary line.'
        Assert-DoesNotContain $portfolioLine[0] 'sha256|e0_|numerator|denominator|tenant|customer|reason|raw'
        Assert-DoesNotContain $text 'DO_NOT_PRINT_THIS|sentinel|fixture-run|pulso-e0-script-test'

        $args = Get-Content -LiteralPath $script:argsLog -Raw
        Assert-Contains $args '--locked.*--offline'
        Assert-Contains $args '-p improvement-engine-runner'
        Assert-Contains $args 'local-sim.*--mode local-simulation.*--source e0'
        Assert-Contains $args ([regex]::Escape($script:inputRoot))
        Assert-Contains $args ([regex]::Escape($script:outputRoot))
        Assert-Contains $args '2026-10-02T18:00:00Z'
        Assert-Contains $args '--arranque-cases 200.*--min-recurring-query-cases 20'
        Assert-Contains $args '--progress-jsonl'
        Assert-True (Test-Path -LiteralPath $script:inputRoot) 'Input directory was removed.'
        if ((Get-Content -LiteralPath (Join-Path $script:inputRoot 'sentinel.txt') -Raw).Trim() -ne 'input must remain unchanged') {
            throw 'Input file was modified.'
        }
    }

    It 'validates the E0 package before starting local simulation' {
        $orderedOutput = Join-Path $script:fixtureRoot 'validated-e0-output'
        Remove-Item -LiteralPath $script:argsLog -ErrorAction SilentlyContinue
        $env:PULSO_E2E_TEST_APPEND_ARGS = '1'
        try {
            $null = & $scriptPath -InputPath $script:inputRoot -OutputPath $orderedOutput -ObservedCutoff '2026-10-02T18:00:00Z'
            $commands = @(Get-Content -LiteralPath $script:argsLog)
            Assert-True ($commands.Count -eq 2) 'Expected source validation and local simulation commands.'
            Assert-Contains $commands[0] '^run --locked --offline .* -- source validate --kind enriched_history --input .+ --contract-version 0\.5\.1$'
            Assert-Contains $commands[1] '^run --locked --offline .* -- local-sim --mode local-simulation --source e0 .*'
            Assert-True (Test-Path -LiteralPath (Join-Path $orderedOutput 'fixture-run\result.json')) 'Validated E0 output was not published.'
        }
        finally {
            Remove-Item Env:PULSO_E2E_TEST_APPEND_ARGS -ErrorAction SilentlyContinue
        }
    }

    It 'fails closed when E0 source validation fails without running or publishing output' {
        $failedValidationOutput = Join-Path $script:fixtureRoot 'failed-validation-output'
        Remove-Item -LiteralPath $script:argsLog -ErrorAction SilentlyContinue
        $env:PULSO_E2E_TEST_APPEND_ARGS = '1'
        $env:PULSO_E2E_TEST_FAIL_VALIDATE = '1'
        $failure = ''
        try {
            try {
                & $scriptPath -InputPath $script:inputRoot -OutputPath $failedValidationOutput -ObservedCutoff '2026-10-02T18:00:00Z' | Out-Null
            }
            catch {
                $failure = $_.Exception.Message
            }

            $commands = @(Get-Content -LiteralPath $script:argsLog)
            Assert-True ($commands.Count -eq 1) 'Local simulation ran after source validation failed.'
            Assert-Contains $failure 'source validation failed'
            Assert-DoesNotContain $failure 'DO_NOT_PRINT_THIS|private_customer_id|fixture-root'
            Assert-Contains $commands[0] '^run --locked --offline .* -- source validate --kind enriched_history --input .+ --contract-version 0\.5\.1$'
            Assert-True (-not (Test-Path -LiteralPath $failedValidationOutput)) 'Failed validation published run output.'
        }
        finally {
            Remove-Item Env:PULSO_E2E_TEST_APPEND_ARGS -ErrorAction SilentlyContinue
            Remove-Item Env:PULSO_E2E_TEST_FAIL_VALIDATE -ErrorAction SilentlyContinue
        }
    }

    It 'runs an original-bank snapshot through the public wrapper without calling a provider' {
        $originalOutput = Join-Path $script:fixtureRoot 'original-output'
        $output = & $scriptPath -Source original -InputPath $script:inputRoot -OutputPath $originalOutput -ObservedCutoff '2026-10-02T18:00:00Z'
        $text = $output -join [Environment]::NewLine
        Assert-Contains $text 'Source: original_bank'
        Assert-DoesNotContain $text 'Portfolio:'
        Assert-Contains $text 'Descriptive draft: descriptive_status=simulated_unverified; execution=not_executed; publication_eligible=false; agent_core=dependency_blocked_snapshot_semantics'
        Assert-DoesNotContain $text 'DO_NOT_PRINT_THIS|pulso-e0-script-test'

        $args = Get-Content -LiteralPath $script:argsLog -Raw
        Assert-Contains $args 'local-sim.*--mode local-simulation.*--source original'
        Assert-DoesNotContain $args '--arranque-cases'
        Assert-DoesNotContain $args '--min-recurring-query-cases'

        $env:PULSO_E2E_TEST_JSON_MODE = 'unsafe_original_candidate'
        $unsafeOutput = Join-Path $script:fixtureRoot 'unsafe-original-output'
        $unsafeRejected = $false
        $unsafeMessage = ''
        try {
            $null = & $scriptPath -Source original -InputPath $script:originalInputRoot -OutputPath $unsafeOutput -ObservedCutoff '2026-10-02T18:00:00Z'
        }
        catch {
            $unsafeRejected = $true
            $unsafeMessage = $_.Exception.Message
        }
        finally {
            Remove-Item Env:PULSO_E2E_TEST_JSON_MODE -ErrorAction SilentlyContinue
        }
        Assert-True $unsafeRejected 'The original-bank wrapper accepted an executable-looking proposal field.'
        Assert-DoesNotContain $unsafeMessage 'DO_NOT_PRINT_THIS|candidate|hypothesis'
    }

    It 'shows only validated aggregate source progress while suppressing raw Cargo diagnostics' {
        $progressOutput = & $scriptPath `
            -Source original `
            -InputPath $script:originalInputRoot `
            -OutputPath (Join-Path $script:fixtureRoot 'progress-original-output') `
            -ObservedCutoff '2026-10-02T18:00:00Z' 6>&1
        $text = @($progressOutput | ForEach-Object {
            if ($_ -is [System.Management.Automation.InformationRecord]) { $_.MessageData }
            else { $_ }
        }) -join [Environment]::NewLine
        Assert-Contains $text 'Progress: source inventory complete — 2/2 files; 52/52 bytes; 12 ms.'
        Assert-Contains $text 'Progress: source inventory started — inventory pending; size pending; 0 ms.'
        Assert-DoesNotContain $text 'DO_NOT_PRINT_THIS|private_customer_id|sha256|[A-Z]:\\'
    }

    It 'rejects a forged E0 portfolio in an otherwise valid OriginalBank result' {
        $env:PULSO_E2E_TEST_JSON_MODE = 'forged_original_portfolio'
        $failure = ''
        try {
            $null = & $scriptPath -Source original -InputPath $script:originalInputRoot -OutputPath (Join-Path $script:fixtureRoot 'forged-original-portfolio') -ObservedCutoff '2026-10-02T18:00:00Z'
        }
        catch {
            $failure = $_.Exception.Message
        }
        finally {
            Remove-Item Env:PULSO_E2E_TEST_JSON_MODE -ErrorAction SilentlyContinue
        }
        Assert-Contains $failure 'must not report an E0 portfolio'
        Assert-DoesNotContain $failure 'DO_NOT_PRINT_THIS|private_reason|sha256|simulator_only|candidate_for_simulated'
    }

    It 'rejects E0 portfolios whose dispositions do not map one-to-one to signals and digests' {
        foreach ($mode in @('duplicate_disposition', 'missing_disposition', 'mismatched_digest', 'paired_omission')) {
            $env:PULSO_E2E_TEST_JSON_MODE = $mode
            $failure = ''
            try {
                $null = & $scriptPath -InputPath $script:inputRoot -OutputPath (Join-Path $script:fixtureRoot ('invalid-portfolio-' + $mode)) -ObservedCutoff '2026-10-02T18:00:00Z'
            }
            catch {
                $failure = $_.Exception.Message
            }
            Assert-Contains $failure 'does not match E0 signals'
            Assert-DoesNotContain $failure 'sha256|private_reason|e0_technical_error_rate|e0_tool_retry_case_rate'
        }
        Remove-Item Env:PULSO_E2E_TEST_JSON_MODE -ErrorAction SilentlyContinue
    }

    It 'binds the portfolio primary digest to the selected result signal' {
        $env:PULSO_E2E_TEST_JSON_MODE = 'wrong_primary'
        $failure = ''
        try {
            $null = & $scriptPath -InputPath $script:inputRoot -OutputPath (Join-Path $script:fixtureRoot 'wrong-primary-output') -ObservedCutoff '2026-10-02T18:00:00Z'
        }
        catch {
            $failure = $_.Exception.Message
        }
        finally {
            Remove-Item Env:PULSO_E2E_TEST_JSON_MODE -ErrorAction SilentlyContinue
        }
        Assert-Contains $failure 'does not match E0 signals'
        Assert-DoesNotContain $failure 'sha256|e0_technical_error_rate|private_reason'
    }

    It 'rejects an inconsistent portfolio summary without exposing raw portfolio fields' {
        $env:PULSO_E2E_TEST_JSON_MODE = 'inconsistent_portfolio'
        $failure = ''
        try {
            $null = & $scriptPath -InputPath $script:inputRoot -OutputPath (Join-Path $script:fixtureRoot 'inconsistent-portfolio-output') -ObservedCutoff '2026-10-02T18:00:00Z'
        }
        catch {
            $failure = $_.Exception.Message
        }
        finally {
            Remove-Item Env:PULSO_E2E_TEST_JSON_MODE -ErrorAction SilentlyContinue
        }
        Assert-Contains $failure 'inconsistent E0 portfolio status'
        Assert-DoesNotContain $failure 'DO_NOT_PRINT_THIS|private_reason|sha256|candidate_for_simulated'
    }

    It 'rejects explicitly supplied E0-only options for original source before invoking Cargo' {
        $incompatibleArguments = @(
            @{ Name = 'ArranqueCases'; Value = 50 },
            @{ Name = 'MinimumRecurringQueryCases'; Value = 25 }
        )
        foreach ($incompatibleArgument in $incompatibleArguments) {
            Remove-Item -LiteralPath $script:argsLog -ErrorAction SilentlyContinue
            $thrown = $false
            $arguments = @{
                Source = 'original'
                InputPath = $script:originalInputRoot
                OutputPath = Join-Path $script:fixtureRoot ('original-with-' + $incompatibleArgument.Name)
                ObservedCutoff = '2026-10-02T18:00:00Z'
            }
            $arguments[$incompatibleArgument.Name] = $incompatibleArgument.Value
            try {
                & $scriptPath @arguments | Out-Null
            }
            catch {
                $thrown = $true
            }

            Assert-True $thrown "An explicitly supplied E0-only option '$($incompatibleArgument.Name)' was silently ignored for original source."
            Assert-True (-not (Test-Path -LiteralPath $script:argsLog)) "Cargo ran despite incompatible E0-only option '$($incompatibleArgument.Name)'."
        }
    }

    It 'runs E0 and original-bank snapshots into separate fresh outputs with source-specific summaries' {
        $combinedScript = Join-Path $PSScriptRoot '..\scripts\run-local-snapshots-e2e.ps1'
        $outputRoot = Join-Path $script:fixtureRoot 'both-sources-output'
        Remove-Item -LiteralPath $script:argsLog -ErrorAction SilentlyContinue
        $env:PULSO_E2E_TEST_APPEND_ARGS = '1'
        try {
            $output = & $combinedScript `
                -E0InputPath $script:inputRoot `
                -OriginalInputPath $script:originalInputRoot `
                -OutputRoot $outputRoot `
                -ObservedCutoff '2026-10-02T18:00:00Z'
            $text = $output -join [Environment]::NewLine
            Assert-Contains $text '=== E0 source run ==='
            Assert-Contains $text 'Source: e0'
            Assert-Contains $text '=== Original-bank source run ==='
            Assert-Contains $text 'Source: original_bank'
            Assert-Contains $text 'dependency_blocked_snapshot_semantics'
            Assert-Contains $text 'Both local-simulation source runs completed'
            Assert-DoesNotContain $text 'pulso-e0-script-test|DO_NOT_PRINT_THIS'

            $e0Result = @(Get-ChildItem -LiteralPath (Join-Path $outputRoot 'e0') -Filter 'result.json' -Recurse)
            $originalResult = @(Get-ChildItem -LiteralPath (Join-Path $outputRoot 'original') -Filter 'result.json' -Recurse)
            Assert-True ($e0Result.Count -eq 1) 'E0 result was not written to its dedicated output directory.'
            Assert-True ($originalResult.Count -eq 1) 'Original-bank result was not written to its dedicated output directory.'

            $args = Get-Content -LiteralPath $script:argsLog -Raw
            Assert-Contains $args '--offline.*--source e0'
            Assert-Contains $args '--offline.*--source original'
            Assert-Contains $args 'local-sim.*--mode local-simulation'
            Assert-DoesNotContain $args 'openrouter|https?://|provider'
        }
        finally {
            Remove-Item Env:PULSO_E2E_TEST_APPEND_ARGS -ErrorAction SilentlyContinue
        }
    }

    It 'rejects an existing combined output root before invoking Cargo' {
        $combinedScript = Join-Path $PSScriptRoot '..\scripts\run-local-snapshots-e2e.ps1'
        $existingRoot = Join-Path $script:fixtureRoot 'existing-both-sources-output'
        New-Item -ItemType Directory -Path $existingRoot -Force | Out-Null
        $sentinel = Join-Path $existingRoot 'keep.txt'
        Set-Content -LiteralPath $sentinel -Value 'do not overwrite'
        Remove-Item -LiteralPath $script:argsLog -ErrorAction SilentlyContinue

        $thrown = $false
        try {
            & $combinedScript `
                -E0InputPath $script:inputRoot `
                -OriginalInputPath $script:originalInputRoot `
                -OutputRoot $existingRoot `
                -ObservedCutoff '2026-10-02T18:00:00Z' | Out-Null
        }
        catch {
            $thrown = $true
        }
        Assert-True $thrown 'An existing combined output root was accepted.'
        Assert-True ((Get-Content -LiteralPath $sentinel -Raw).Trim() -eq 'do not overwrite') 'Existing output was modified.'
        Assert-True (-not (Test-Path -LiteralPath $script:argsLog)) 'Cargo ran despite the existing output root.'
    }

    It 'rejects a combined output root through a junction before creating source outputs' {
        $combinedScript = Join-Path $PSScriptRoot '..\scripts\run-local-snapshots-e2e.ps1'
        $junction = Join-Path ([System.IO.Path]::GetTempPath()) ('pulso-both-sources-junction-' + [guid]::NewGuid().ToString('N'))
        $junctionCreated = $false
        try {
            New-Item -ItemType Junction -Path $junction -Target $script:inputRoot -ErrorAction Stop | Out-Null
            $junctionCreated = $true
        }
        catch {
            throw 'Could not create a temporary Windows junction; the combined-output alias regression cannot be exercised.'
        }

        try {
            Remove-Item -LiteralPath $script:argsLog -ErrorAction SilentlyContinue
            $aliasedOutput = Join-Path $junction 'combined-output'
            $thrown = $false
            try {
                & $combinedScript `
                    -E0InputPath $script:inputRoot `
                    -OriginalInputPath $script:originalInputRoot `
                    -OutputRoot $aliasedOutput `
                    -ObservedCutoff '2026-10-02T18:00:00Z' | Out-Null
            }
            catch {
                $thrown = $true
            }
            Assert-True $thrown 'The combined wrapper accepted an output root through an input junction.'
            Assert-True (-not (Test-Path -LiteralPath (Join-Path $script:inputRoot 'combined-output'))) 'The aliased output root contaminated the input.'
            Assert-True (-not (Test-Path -LiteralPath $script:argsLog)) 'Cargo ran before the combined output alias was rejected.'
        }
        finally {
            if ($junctionCreated -and (Test-Path -LiteralPath $junction)) {
                [System.IO.Directory]::Delete($junction, $false)
            }
        }
    }

    It 'prints none when holdout is missing and suppresses malformed JSON content' {
        $env:PULSO_E2E_TEST_JSON_MODE = 'missing_holdout'
        $missingOutput = & $scriptPath -InputPath $script:inputRoot -OutputPath (Join-Path $script:fixtureRoot 'missing-output') -ObservedCutoff '2026-10-02T18:00:00Z'
        $missingText = $missingOutput -join [Environment]::NewLine
        Assert-Contains $missingText 'Cases: discovery=200; replay_excluded=suppressed'
        Assert-Contains $missingText 'Holdout: none'
        Assert-DoesNotContain $missingText 'DO_NOT_PRINT_THIS|private_customer_id'

        $env:PULSO_E2E_TEST_JSON_MODE = 'unsafe_missing_holdout'
        $unsafeMissingRejected = $false
        $unsafeMissingMessage = ''
        try {
            $null = & $scriptPath -InputPath $script:inputRoot -OutputPath (Join-Path $script:fixtureRoot 'unsafe-missing-output') -ObservedCutoff '2026-10-02T18:00:00Z'
        }
        catch {
            $unsafeMissingRejected = $true
            $unsafeMissingMessage = $_.Exception.Message
        }
        Assert-True $unsafeMissingRejected 'Wrapper accepted an E0 replay count when holdout evaluation was absent.'
        Assert-DoesNotContain $unsafeMissingMessage '4|replay_excluded'

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

    It 'suppresses all insufficient-support aggregates and rejects leaked small cells' {
        try {
            $env:PULSO_E2E_TEST_JSON_MODE = 'insufficient_holdout'
            $suppressedOutput = & $scriptPath -InputPath $script:inputRoot -OutputPath (Join-Path $script:fixtureRoot 'suppressed-output') -ObservedCutoff '2026-10-02T18:00:00Z'
            $suppressedText = $suppressedOutput -join [Environment]::NewLine
            Assert-Contains $suppressedText 'Holdout: status=insufficient_support; counts=suppressed; descriptive_only'
            Assert-Contains $suppressedText 'Cases: discovery=200; replay_excluded=suppressed'
            Assert-DoesNotContain $suppressedText 'replay_excluded=1800'
            Assert-DoesNotContain $suppressedText '\b(?:1|2|3|4)/5\b'

            $env:PULSO_E2E_TEST_JSON_MODE = 'unsafe_insufficient_holdout'
            $leakRejected = $false
            $leakMessage = ''
            try {
                $null = & $scriptPath -InputPath $script:inputRoot -OutputPath (Join-Path $script:fixtureRoot 'unsafe-output') -ObservedCutoff '2026-10-02T18:00:00Z'
            }
            catch {
                $leakRejected = $true
                $leakMessage = $_.Exception.Message
            }
            Assert-True $leakRejected 'Wrapper accepted exact small-cell counts for insufficient support.'
            Assert-DoesNotContain $leakMessage '1/5|2000|private'
        }
        finally {
            Remove-Item Env:PULSO_E2E_TEST_JSON_MODE -ErrorAction SilentlyContinue
        }
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

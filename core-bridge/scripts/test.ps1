<#
.SYNOPSIS
  Test entry point for core-bridge L1.  test.ps1 -Suite wire -Target mock|a2|real_local
.DESCRIPTION
  -Suite wire: L1a snapshot tests + registry-wire-contract parity against TARGET.
    mock       : starts registry_mock (real HTTP process) and compares with the recorded fixtures (mock_infidelity on mismatch)
    a2         : starts the REAL RegistryService over InMemoryRegistryStore in the pinned venv (wire_drift_detected on mismatch)
    real_local : needs REGISTRY_BASE_URL (agent-core serve over PG); requires_sim cases are skipped and reported
  Writes platform-sim/tests/parity/.out/parity_report.<target>.json (never claims real_local from mock/a2).
#>
[CmdletBinding()]
param(
    [ValidateSet('wire')][string]$Suite = 'wire',
    [ValidateSet('mock', 'a2', 'real_local')][string]$Target = 'mock'
)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$root = Resolve-Path (Join-Path $here '..')
$env:UV_PROJECT_ENVIRONMENT = Join-Path $env:TEMP 'pulso-cb-venv'
$env:TARGET = if ($Target -eq 'real_local') { 'real' } else { $Target }
Push-Location $root
try {
    uv sync --locked --python 3.12 | Out-Null
    uv run --python 3.12 pytest -c pyproject.toml tests ../platform-sim/tests/parity
    exit $LASTEXITCODE
} finally { Pop-Location }

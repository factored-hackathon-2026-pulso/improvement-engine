<#
.SYNOPSIS
  DEMO-0 entry point: replay of the ten-step E2E-THREAD-01 (no containers) -> engine-run report -> human summary
  that lists the doubles[] (what is NOT real) FIRST, then the steps with labels.
  demo/run-demo0.ps1 [-OutDir demo/out/demo0] [-FromReport report.json] [-Live] [-RealCore]
.DESCRIPTION
  -Live      prints how a live roleplay window is run (roleplay-llm/RUNBOOK.md); does not spawn models.
  -RealCore  prints the real-Core procedure (e2e-core/run.ps1: one stack, torn down); does not start containers.
  -FromReport summarises an existing report instead of replaying (no sensor exe needed).
  Replay needs the pinned venv and the Rust sensor exe (ED0_RUNNER_EXE). Exits 1 if the report fails the G1 check().
#>
[CmdletBinding()]
param([string]$OutDir, [string]$FromReport, [switch]$Live, [switch]$RealCore)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$root = (Resolve-Path (Join-Path $here '..')).Path
$pinSha = (Select-String -Path (Join-Path $root 'agent-core-assets\manifest.yaml') -Pattern '^\s+sha:\s*([0-9a-f]{40})\s*$').Matches[0].Groups[1].Value
$py = Join-Path $env:TEMP "pulso-wire-venv-$($pinSha.Substring(0, 7))\Scripts\python.exe"
if (-not (Test-Path $py)) { $py = (Get-Command python -ErrorAction Stop).Source }
$env:PYTHONPATH = (Join-Path $here 'src') + ';' + (Join-Path $root 'e2e-core\src') + ';' + (Join-Path $root 'core-bridge\src') + ';' + (Join-Path $root 'local-identity\src')
$a = @('-m', 'pulso_demo.demo0')
if ($OutDir) { $a += @('--out-dir', $OutDir) }
if ($FromReport) { $a += @('--from-report', $FromReport) }
if ($Live) { $a += '--live' }
if ($RealCore) { $a += '--real-core' }
& $py @a
exit $LASTEXITCODE

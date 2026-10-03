<#
.SYNOPSIS
  Orchestrator-cycle watcher for upstream agent-core / llm-gateway drift vs our committed pin.
.DESCRIPTION
  Thin wrapper over pin_watch.py (stdlib only). Read-only: `gh api` GETs + a scratch checkout under %TEMP%.
  All arguments are forwarded, e.g.:  pin-watch.ps1 -- --json --since-last
  Exit codes: 0 no-change | 10 additive-safe | 20 needs-bump-work | 30 breaking | 1 tool error.
  Reports: scripts/core/pin-watch/out/pin-watch-report.{json,md} (git-ignored).
#>
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$script = Join-Path $here 'pin_watch.py'
$fwd = @($args | Where-Object { $_ -ne '--' })
$uv = Get-Command uv -ErrorAction SilentlyContinue
if ($uv) { & uv run --python 3.12 --no-project python $script @fwd }
else {
    $py = Get-Command python -ErrorAction SilentlyContinue
    if (-not $py) { Write-Error 'pin-watch: neither uv nor python found'; exit 1 }
    & python $script @fwd
}
exit $LASTEXITCODE

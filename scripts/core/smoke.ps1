# Thin wrapper: forwards to local/core/smoke.ps1 (plan 17.3.8).
& (Join-Path $PSScriptRoot '..\..\local\core\smoke.ps1') @args
exit $LASTEXITCODE

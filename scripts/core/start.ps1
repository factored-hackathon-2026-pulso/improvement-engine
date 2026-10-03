# Thin wrapper: forwards to local/core/start.ps1 (plan 17.3.8).
& (Join-Path $PSScriptRoot '..\..\local\core\start.ps1') @args
exit $LASTEXITCODE

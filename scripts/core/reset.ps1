# Thin wrapper: forwards to local/core/reset.ps1 (plan 17.3.8).
& (Join-Path $PSScriptRoot '..\..\local\core\reset.ps1') @args
exit $LASTEXITCODE

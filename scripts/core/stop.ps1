# Thin wrapper: forwards to local/core/stop.ps1 (plan 17.3.8).
& (Join-Path $PSScriptRoot '..\..\local\core\stop.ps1') @args
exit $LASTEXITCODE

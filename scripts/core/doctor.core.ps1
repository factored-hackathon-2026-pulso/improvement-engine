# Thin wrapper: forwards to local/core/doctor.core.ps1 (accepts -Json). Plan 17.3.8.
& (Join-Path $PSScriptRoot '..\..\local\core\doctor.core.ps1') @args
exit $LASTEXITCODE

# Run the bridge from any directory: cd to the repo root of this script, pass all args through.
Set-Location (Join-Path $PSScriptRoot '..\..')
& python scripts/o11y/runtrace_bridge.py @args
exit $LASTEXITCODE

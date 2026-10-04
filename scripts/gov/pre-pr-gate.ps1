# W0 pre-PR gate (G0p): verify-local-ci + pytest of touched packages + replay-ratchet; writes a receipt.
# A leg whose command is not supplied is "missing" and fails the gate.
[CmdletBinding()]
param(
    [Parameter(Mandatory)] [string] $ReceiptPath,
    [string[]] $TouchedPackages = @(),
    [string] $CiCommand,
    [string] $PytestCommand,
    [string] $RatchetCommand
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$repo = Resolve-Path (Join-Path $PSScriptRoot '..\..')
$packages = @($TouchedPackages | ForEach-Object { $_ -split ',' } | Where-Object { $_ })

$bad = @($packages | Where-Object { $_ -notmatch '^[A-Za-z0-9][A-Za-z0-9._/-]*$' -or $_ -match '\.\.' })
if ($bad.Count) { [Console]::Error.WriteLine("Invalid touched package path(s): $($bad -join ', ')"); exit 1 }
if (-not $CiCommand) { $CiCommand = "& '$repo\scripts\verify-local-ci.ps1'" }
if (-not $PytestCommand -and $packages.Count) {
    $PytestCommand = ($packages | ForEach-Object { "python -m pytest -q '$repo\$_'; if (`$LASTEXITCODE) { exit `$LASTEXITCODE }" }) -join '; '
}

function Invoke-Leg([string] $Command) {
    if (-not $Command) { return [pscustomobject]@{ status = 'missing'; exit_code = $null } }
    $global:LASTEXITCODE = 0
    & pwsh -NoProfile -Command $Command | Out-Host
    $code = $LASTEXITCODE
    [pscustomobject]@{ status = $(if ($code -eq 0) { 'pass' } else { 'fail' }); exit_code = $code }
}

$legs = [ordered]@{
    ci      = Invoke-Leg $CiCommand
    pytest  = Invoke-Leg $PytestCommand
    ratchet = Invoke-Leg $RatchetCommand
}
$ok = -not ($legs.Values | Where-Object { $_.status -ne 'pass' })
$receipt = [ordered]@{
    schema = 'pre-pr-gate/v1'
    generated_utc = (Get-Date).ToUniversalTime().ToString('o')
    packages_touched = $packages
    legs = $legs
    verdict = $(if ($ok) { 'pass' } else { 'fail' })
}
$receipt | ConvertTo-Json -Depth 5 | Set-Content -Encoding utf8 $ReceiptPath
Write-Output "pre-pr-gate: $($receipt.verdict) (receipt: $ReceiptPath)"
exit $(if ($ok) { 0 } else { 1 })

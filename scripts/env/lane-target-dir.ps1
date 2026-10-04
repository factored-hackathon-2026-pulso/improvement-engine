[CmdletBinding()]
param(
    [Parameter(Mandatory)] [string] $Lane,
    [string] $Root = 'D:\cargo-targets'
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
try {
    if ($Lane -notmatch '^[A-Za-z0-9][A-Za-z0-9._-]*$') { throw "Invalid lane name '$Lane'." }
    if ($Root -notmatch ('^[Dd]:[/' + [char]92 + [char]92 + ']')) { throw 'Cargo target dirs must live on D: (never C:).' }
    Write-Output (Join-Path $Root $Lane)
} catch {
    [Console]::Error.WriteLine($_.Exception.Message)
    exit 1
}

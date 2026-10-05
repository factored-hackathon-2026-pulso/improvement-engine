<#
.SYNOPSIS
  Integrated rig: one line per component (OK/FAIL), exit 0 only when all are up. Read-only; prints no credential.
  scripts/integrated-rig/health.ps1 [-Quiet]
#>
[CmdletBinding()]
param([switch]$Quiet)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$root = (Resolve-Path (Join-Path $here '..\..')).Path
. (Join-Path $here 'rig.lib.ps1')

$settings = Get-RigSettings
$paths = Get-RigPaths -Root $root
$conn = $(if ($env:PULSO_PODMAN_CONNECTION) { $env:PULSO_PODMAN_CONNECTION } else { 'pulso-dev-root' })
$ok = $true
function Line { param([string]$Name, [bool]$Good, [string]$Extra = '') $script:ok = $script:ok -and $Good; if (-not $Quiet) { Write-Host ("{0} {1} {2}" -f $(if ($Good) { 'OK  ' } else { 'FAIL' }), $Name, $Extra) } }

foreach ($c in @("$($settings.Prefix)-postgres", "$($settings.Prefix)-llm-gateway")) {
    $st = ''
    try { $st = ((& podman --connection $conn inspect -f '{{.State.Status}}' $c 2>$null) | Out-String).Trim() } catch { }
    Line $c ($st -eq 'running') $st
}
$core = "http://127.0.0.1:$($settings.CorePort)"; $plat = "http://127.0.0.1:$($settings.PlatformPort)"
Line "llm-gateway :$($settings.GwPort) /healthz" (Test-Http "http://127.0.0.1:$($settings.GwPort)/healthz")
Line "agent-core :$($settings.CorePort) /healthz" (Test-Http "$core/healthz")
Line "agent-core :$($settings.CorePort) /readyz" (Test-Http "$core/readyz")
Line "platform :$($settings.PlatformPort) /api/v1/health" (Test-Http "$plat/api/v1/health")
Line 'platform /api/v1/meta' (Test-Http "$plat/api/v1/meta")
$exe = ''
if (Test-Path -LiteralPath (Join-Path $paths.Rig 'rig.json')) { $exe = [string](Read-JsonFile -Path (Join-Path $paths.Rig 'rig.json')).pulso_exe }
Line 'engine binary' ($exe -and (Test-Path -LiteralPath $exe)) $(if ($exe) { Split-Path -Leaf (Split-Path -Parent (Split-Path -Parent $exe)) } else { 'no rig.json (run up.ps1)' })
foreach ($f in @('identity-keys.json', 'staff-keys.json')) {
    $p = Join-Path $paths.Dev $f
    $n = 0
    if (Test-Path -LiteralPath $p) { $d = Read-JsonFile -Path $p; $n = @($d.principal_keys.PSObject.Properties).Count }
    Line "merged $f" ($n -ge 3) "$n principal kids"
}
if (-not $Quiet) { Write-Host ("free RAM {0} MB" -f (Get-FreeRamMb)) }
if ($ok) { exit 0 } else { exit 1 }

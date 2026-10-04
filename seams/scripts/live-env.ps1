<#
.SYNOPSIS
  Dot-source to export the PULSO_* env vars of the live Rust tests from a kept e2e stack. NEVER prints a value.
  Start the stack first (Podman machine pulso-dev only), e.g.:
    e2e-core\run.ps1 -BaseImage localhost/pulso-core-runtime:c814c2b-920f5e3 -Namespace claude-w4a-1 -Keep -PytestArgs '-k','nothing_selected'
  Then:  . seams\scripts\live-env.ps1 -Namespace claude-w4a-1
  Tear down (always): local\core\stop.ps1 -Namespace <ns>; local\core\reset.ps1 -Namespace <ns> -Confirm
.DESCRIPTION
  Reads local/.secrets/<ns>/e2e/{e2e-env,e2e-keys}.json (service key of the sandbox stack) and, through `podman exec`
  into the stack's own human-issuer container (no machine ssh), the SANDBOX human staff signer used by the labelled
  simulated human (`LocalSimAuthorizer`, auth.simulated=true). Values go into process env vars only.
#>
param([Parameter(Mandatory)][string]$Namespace)
$root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$dir = Join-Path $root "local\.secrets\$Namespace\e2e"
$e = Get-Content (Join-Path $dir 'e2e-env.json') -Raw | ConvertFrom-Json
$k = Get-Content (Join-Path $dir 'e2e-keys.json') -Raw | ConvertFrom-Json
function ConvertTo-Hex([string]$b64u) {
    $s = $b64u.Trim().Replace('-', '+').Replace('_', '/')
    while ($s.Length % 4) { $s += '=' }
    -join ([Convert]::FromBase64String($s) | ForEach-Object { $_.ToString('x2') })
}
$podman = 'C:\Users\alexg\AppData\Local\Programs\Podman\podman.exe'
$human = (& $podman --connection pulso-dev exec "$($e.project)-human-issuer-1" cat /run/hi-keys/human-staff-signer.json) -join '' | ConvertFrom-Json
$env:PULSO_BRIDGE_ADDR = "127.0.0.1:$($e.ports.runtime)"
$env:PULSO_CORE_ADDR = $env:PULSO_BRIDGE_ADDR
$env:PULSO_SERVICE_KID = $k.service_kid
$env:PULSO_SERVICE_SEED_HEX = ConvertTo-Hex $k.service_seed
$env:PULSO_E2E_FX_ADDR = ([uri]$e.url).Authority
$env:PULSO_HUMAN_KID = $human.kid
$env:PULSO_HUMAN_SEED_HEX = ConvertTo-Hex $human.key
$env:PULSO_LIVE_TENANT = 'tenant-local'
$env:PULSO_LIVE_AGENT = 'atencion-tarea'
$env:PULSO_LIVE_STACK_IMAGE = $e.image
$env:PULSO_LIVE_NAMESPACE = $Namespace
$m = Get-Content (Join-Path $root 'agent-core-assets\manifest.yaml')
$env:PULSO_LIVE_WRITER_RELEASE = ($m | Select-String '^\s+pulso-writer:\s*(\S+)').Matches[0].Groups[1].Value
Write-Output "live env set for $Namespace (values not printed)"

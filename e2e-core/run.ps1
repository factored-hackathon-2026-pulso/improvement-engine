<#
.SYNOPSIS
  One-command Claude-side E2E precursor of the joint H4 E2E (codex-standin drives the REAL real_local Core stack).
  e2e-core/run.ps1 [-Namespace claude-e2e-<ms>] [-Keep] [-UnitOnly] [-PytestArgs ...]
.DESCRIPTION
  1. unit tests of the stand-in (no stack); 2. LLM-gateway + inline eval-budgets env lines in the stack's core.env (compose forwards them); 3. local/core/start.ps1 -Profile real_local on machine pulso-dev with the control-api / lab-broker
  URLs pointed at the `e2e-fixtures` double container; 4. live pytest; 5. ALWAYS tear down (stop, reset -Confirm) unless -Keep. Writes e2e-core/.out/e2e-report.json.
#>
[CmdletBinding()]
param([string]$Namespace, [string]$BaseImage, [switch]$Keep, [switch]$UnitOnly, [string[]]$PytestArgs = @())
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$root = (Resolve-Path (Join-Path $here '..')).Path
$core = Join-Path $root 'local\core'
$podman = 'C:\Users\alexg\AppData\Local\Programs\Podman\podman.exe'
# SHA-agnostic: the agent-core pin is read from the manifest (pin.sha); nothing here hardcodes a SHA.
$pinSha = (Select-String -Path (Join-Path $root 'agent-core-assets\manifest.yaml') -Pattern '^\s+sha:\s*([0-9a-f]{40})\s*$').Matches[0].Groups[1].Value
$pin7 = $pinSha.Substring(0, 7)
$py = Join-Path $env:TEMP "pulso-wire-venv-$pin7\Scripts\python.exe"
if (-not (Test-Path $py)) { Write-Error "pulso:e2e_toolchain_missing: pinned venv %TEMP%\pulso-wire-venv-$pin7 not found"; exit 3 }
$env:PYTHONPATH = (Join-Path $here 'src') + ';' + (Join-Path $root 'core-bridge\src')
$out = Join-Path $here '.out'
New-Item -ItemType Directory -Force -Path $out | Out-Null
Push-Location $here
try {
    & $py -m pytest tests/unit -q -p no:cacheprovider
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    if ($UnitOnly) { exit 0 }
    if (-not $Namespace) { $Namespace = 'claude-e2e-' + [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds() }
    $dir = Join-Path $root "local\.secrets\$Namespace\e2e"
} catch { Write-Error $_; Pop-Location; exit 1 }
$exit = 1
try {
    & $py -m codex_standin.stack prepare --ns $Namespace --dir $dir
    if ($LASTEXITCODE -ne 0) { throw 'prepare failed' }
    $head7 = (git -C $root rev-parse --short=7 HEAD).Trim()
    $base = if ($BaseImage) { $BaseImage } else { "localhost/pulso-core-runtime:$pin7-$head7" }
    & $podman --connection pulso-dev image inspect $base 2>$null | Out-Null
    if ($LASTEXITCODE -ne 0) {
        Write-Output "run: base image $base absent; building (core-bridge/scripts/build-image.ps1)"
        & pwsh -NoProfile -File (Join-Path $root 'core-bridge\scripts\build-image.ps1')
        if ($LASTEXITCODE -ne 0) { throw 'image build failed' }
    }
    # The runtime and the exporter read these URLs from compose interpolation; both point at the fixtures double.
    $env:PULSO_CONTROL_API_URL = 'http://e2e-fixtures:8700'
    $env:PULSO_LAB_BROKER_URL = 'http://e2e-fixtures:8700'
    & pwsh -NoProfile -File (Join-Path $core 'start.ps1') -Namespace $Namespace -Profile real_local -Image $base
    if ($LASTEXITCODE -ne 0) { throw "start.ps1 failed (exit $LASTEXITCODE)" }
    & $py -m codex_standin.stack fixtures --ns $Namespace --dir $dir
    if ($LASTEXITCODE -ne 0) { throw 'fixtures start failed' }
    $env:E2E_ENV_FILE = Join-Path $dir 'e2e-env.json'
    $env:E2E_KEYS_FILE = Join-Path $dir 'e2e-keys.json'
    $env:E2E_REPORT = Join-Path $out 'e2e-report.json'
    $env:E2E_BASE_IMAGE = $base
    & $py -m pytest tests/live -v -p no:cacheprovider @PytestArgs
    $exit = $LASTEXITCODE
} catch {
    Write-Error $_
    $exit = 1
} finally {
    if ($Keep) { Write-Output "run: -Keep, stack left running as namespace $Namespace" } else {
        try { & pwsh -NoProfile -File (Join-Path $core 'stop.ps1') -Namespace $Namespace | Out-Null } catch { Write-Warning $_ }
        try { & pwsh -NoProfile -File (Join-Path $core 'reset.ps1') -Namespace $Namespace -Confirm | Out-Null } catch { Write-Warning $_ }
        try { & $py -m codex_standin.stack cleanup --ns $Namespace --dir $dir | Out-Null } catch { Write-Warning $_ }
        Write-Output "run: torn down namespace $Namespace"
    }
    Pop-Location
}
exit $exit

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
# local-identity/src: the real client (`assert_bound`, LocalIdentityClient) the HumanAuthorizationPort stand-in uses.
$env:PYTHONPATH = (Join-Path $here 'src') + ';' + (Join-Path $root 'core-bridge\src') + ';' + (Join-Path $root 'local-identity\src')
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
        # The image is ALWAYS built from a clean worktree of the COMMITTED HEAD (never the shared, possibly dirty tree).
        Write-Output "run: base image $base absent; building from a clean worktree of HEAD $head7 (core-bridge/scripts/build-image.ps1)"
        $wt = Join-Path ([IO.Path]::GetTempPath()) ("pulso-img-$head7-" + [guid]::NewGuid().ToString('N').Substring(0, 6))
        git -C $root worktree add --detach $wt HEAD | Out-Null
        try {
            & pwsh -NoProfile -File (Join-Path $wt 'core-bridge\scripts\build-image.ps1')
            if ($LASTEXITCODE -ne 0) { throw 'image build failed' }
        } finally { git -C $root worktree remove --force $wt 2>$null | Out-Null }
    }
    # The runtime and the exporter read these URLs from compose interpolation; both point at the fixtures double.
    $env:PULSO_CONTROL_API_URL = 'http://e2e-fixtures:8700'
    $env:PULSO_LAB_BROKER_URL = 'http://e2e-fixtures:8700'
    # The runtime's llm_gateway readiness probe reaches the scripted gateway double while it starts, so the fixtures
    # container must come up as soon as the keys volume exists (platform-sim mounts it), while start.ps1 is still running.
    $startProc = Start-Process pwsh -ArgumentList @('-NoProfile', '-File', (Join-Path $core 'start.ps1'), '-Namespace', $Namespace, '-Profile', 'real_local', '-Image', $base) -PassThru -NoNewWindow
    $project = "pulso-$Namespace"
    $deadline = (Get-Date).AddSeconds(300); $simUp = $false
    while ((Get-Date) -lt $deadline -and -not $startProc.HasExited -and -not $simUp) {
        $h = (& $podman --connection pulso-dev inspect "$project-platform-sim-1" --format '{{if .State.Health}}{{.State.Health.Status}}{{end}}' 2>$null) -join ''
        if ($h.Trim() -eq 'healthy') { $simUp = $true } else { Start-Sleep -Seconds 2 }
    }
    if (-not $simUp) { $startProc.WaitForExit(); throw "platform-sim never became healthy (start.ps1 exit $($startProc.ExitCode))" }
    & $py -m codex_standin.stack fixtures --ns $Namespace --dir $dir
    if ($LASTEXITCODE -ne 0) { throw 'fixtures start failed' }
    $startProc.WaitForExit()
    if ($startProc.ExitCode -ne 0) { throw "start.ps1 failed (exit $($startProc.ExitCode))" }
    & $py -m codex_standin.stack finish --ns $Namespace --dir $dir
    if ($LASTEXITCODE -ne 0) { throw 'stack finish failed' }
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

<#
.SYNOPSIS
  One-command 'demo magica' (plan 2, 10 steps) on our side: Codex stand-in engine + REAL real_local Core stack -> console fixture world.
  demo/run.ps1 [-Offline] [-Serve] [-Keep] [-Namespace claude-demo-<ms>] [-BaseImage <ref>] [-Port 4010]
.DESCRIPTION
  1. unit tests of the translators/analysis (no stack); 2. (unless -Offline) start the real_local Core stack on machine pulso-dev with the
  scripted-LLM/fixtures double (same procedure as e2e-core/run.ps1); 3. python -m pulso_demo.driver plays the scenario and writes
  demo/out/{results,world,replay}.json + demo-report.json; 4. ALWAYS tear the stack down (try/finally) unless -Keep; 5. -Serve starts the
  console fixture API/SSE on the produced world (Ctrl+C to stop). The honest label is in demo-report.json (doubles[]).
#>
[CmdletBinding()]
param([switch]$Offline, [switch]$Serve, [switch]$Keep, [string]$Namespace, [string]$BaseImage, [int]$Port = 4010)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$root = (Resolve-Path (Join-Path $here '..')).Path
$core = Join-Path $root 'local\core'
$podman = 'C:\Users\alexg\AppData\Local\Programs\Podman\podman.exe'
$pinSha = (Select-String -Path (Join-Path $root 'agent-core-assets\manifest.yaml') -Pattern '^\s+sha:\s*([0-9a-f]{40})\s*$').Matches[0].Groups[1].Value
$pin7 = $pinSha.Substring(0, 7)
$py = Join-Path $env:TEMP "pulso-wire-venv-$pin7\Scripts\python.exe"
if (-not (Test-Path $py)) { Write-Error "pulso:demo_toolchain_missing: pinned venv %TEMP%\pulso-wire-venv-$pin7 not found"; exit 3 }
$out = Join-Path $here 'out'
New-Item -ItemType Directory -Force -Path $out | Out-Null
$env:PYTHONPATH = (Join-Path $here 'src') + ';' + (Join-Path $root 'e2e-core\src') + ';' + (Join-Path $root 'core-bridge\src') + ';' + (Join-Path $root 'e2e-core\tests\live')
$exit = 1
Push-Location $here
try {
    & $py -m pytest tests -q -p no:cacheprovider
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
} catch { Write-Error $_; Pop-Location; exit 1 }

$e2eDir = $null
$ns = $null
try {
    if ($Offline) {
        & $py -m pulso_demo.driver --out $out --offline
        $exit = $LASTEXITCODE
    } else {
        if (-not $Namespace) { $Namespace = 'claude-demo-' + [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds() }
        $ns = $Namespace
        $e2eDir = Join-Path $root "local\.secrets\$ns\e2e"
        & $py -m codex_standin.stack prepare --ns $ns --dir $e2eDir
        if ($LASTEXITCODE -ne 0) { throw 'prepare failed' }
        $head7 = (git -C $root rev-parse --short=7 HEAD).Trim()
        $base = if ($BaseImage) { $BaseImage } else { "localhost/pulso-core-runtime:$pin7-$head7" }
        & $podman --connection pulso-dev image inspect $base 2>$null | Out-Null
        if ($LASTEXITCODE -ne 0) {
            Write-Output "run: base image $base absent; building (core-bridge/scripts/build-image.ps1)"
            & pwsh -NoProfile -File (Join-Path $root 'core-bridge\scripts\build-image.ps1')
            if ($LASTEXITCODE -ne 0) { throw 'image build failed' }
        }
        $env:PULSO_CONTROL_API_URL = 'http://e2e-fixtures:8700'
        $env:PULSO_LAB_BROKER_URL = 'http://e2e-fixtures:8700'
        # Same sequence as e2e-core/run.ps1: the runtime's llm_gateway readiness probe needs the scripted gateway double, so the fixtures
        # container starts as soon as platform-sim is healthy, while start.ps1 is still waiting; then `stack finish` merges the stack state.
        $startProc = Start-Process pwsh -ArgumentList @('-NoProfile', '-File', (Join-Path $core 'start.ps1'), '-Namespace', $ns, '-Profile', 'real_local', '-Image', $base) -PassThru -NoNewWindow
        $project = "pulso-$ns"
        $deadline = (Get-Date).AddSeconds(300); $simUp = $false
        while ((Get-Date) -lt $deadline -and -not $startProc.HasExited -and -not $simUp) {
            $h = (& $podman --connection pulso-dev inspect "$project-platform-sim-1" --format '{{if .State.Health}}{{.State.Health.Status}}{{end}}' 2>$null) -join ''
            if ($h.Trim() -eq 'healthy') { $simUp = $true } else { Start-Sleep -Seconds 2 }
        }
        if (-not $simUp) { $startProc.WaitForExit(); throw "platform-sim never became healthy (start.ps1 exit $($startProc.ExitCode))" }
        & $py -m codex_standin.stack fixtures --ns $ns --dir $e2eDir
        if ($LASTEXITCODE -ne 0) { throw 'fixtures start failed' }
        $startProc.WaitForExit()
        if ($startProc.ExitCode -ne 0) { throw "start.ps1 failed (exit $($startProc.ExitCode))" }
        & $py -m codex_standin.stack finish --ns $ns --dir $e2eDir
        if ($LASTEXITCODE -ne 0) { throw 'stack finish failed' }
        $env:E2E_ENV_FILE = Join-Path $e2eDir 'e2e-env.json'
        $env:E2E_KEYS_FILE = Join-Path $e2eDir 'e2e-keys.json'
        $env:DEMO_NAMESPACE = $ns
        & $py -m pulso_demo.driver --out $out --namespace $ns
        $exit = $LASTEXITCODE
    }
} catch {
    Write-Error $_
    $exit = 1
} finally {
    if ($ns) {
        if ($Keep) { Write-Output "run: -Keep, stack left running as namespace $ns" } else {
            try { & pwsh -NoProfile -File (Join-Path $core 'stop.ps1') -Namespace $ns | Out-Null } catch { Write-Warning $_ }
            try { & pwsh -NoProfile -File (Join-Path $core 'reset.ps1') -Namespace $ns -Confirm | Out-Null } catch { Write-Warning $_ }
            try { & $py -m codex_standin.stack cleanup --ns $ns --dir $e2eDir | Out-Null } catch { Write-Warning $_ }
            Write-Output "run: torn down namespace $ns"
        }
    }
    Pop-Location
}
if (Test-Path (Join-Path $out 'world.json')) {
    Write-Output "demo: wrote $out\demo-report.json and world.json (see demo/README.md to watch it in the console)"
    if ($Serve) {
        $env:FIXTURE_PORT = "$Port"
        node (Join-Path $here 'serve\serve-demo.mjs') (Join-Path $out 'world.json') (Join-Path $out 'replay.json')
    }
}
exit $exit

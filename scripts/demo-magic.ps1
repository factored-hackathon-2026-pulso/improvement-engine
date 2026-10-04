<#
.SYNOPSIS
  One command: run the engine's DEMO-0 ten-step thread and WATCH it live in the debug-console (our internal backoffice).
  scripts/demo-magic.ps1 [-PaceMs 600] [-Port 4020] [-NoBrowser] [-NoHold] [-RealCore] [-Jobs 1]
.DESCRIPTION
  1. builds `pulso` if needed (cargo build -p pulso; one job by default) and the console if node exists
     (npm ci when node_modules is missing, then npm run build). Without node it says so and serves the API only.
  2. starts `pulso serve` (loopback only, ephemeral admin token passed through the environment, never printed) and prints the URL.
  3. opens the console in the browser (a nicety; -NoBrowser skips it), then runs `pulso demo`, which streams every step,
     double and gate into the console while it runs (-PaceMs slows it for a human).
  4. prints the doubles[] (what is NOT real) FIRST, then the steps with their labels (same layout as demo/run-demo0.ps1).
  5. holds the server for you to look around (Enter or Ctrl+C), unless -NoHold. Everything it started is stopped on exit,
     Ctrl+C or error; nothing is left running.
  DEMO-0 means: offline Core double, no real model, simulated human, no quality claim. -RealCore is refused honestly
  unless a real Core is reachable, and even then this build does not wire it (see `pulso demo --real-core`).
  Exit code: the demo's (0 ok, 1 job or server failed, 2 refusal or usage).
#>
[CmdletBinding()]
param([int]$PaceMs = 600, [int]$Port = 4020, [switch]$NoBrowser, [switch]$NoHold, [switch]$RealCore, [int]$Jobs = 1)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$root = (Resolve-Path (Join-Path $here '..')).Path
. (Join-Path $here 'demo-magic.lib.ps1')

$seams = Join-Path $root 'seams'
$consoleDir = Join-Path $root 'debug-console'
$targetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $seams 'target' }
$exe = Join-Path $targetDir 'debug\pulso.exe'
$server = $null; $demo = $null

function Stop-Started {
    foreach ($p in @($demo, $server)) {
        if ($p -and -not $p.HasExited) { try { $p.Kill($true) } catch { try { $p.Kill() } catch { } }; try { [void]$p.WaitForExit(5000) } catch { } }
    }
}

try {
    Write-Host "[1/4] building pulso (cargo build -j $Jobs -p pulso; a no-op when up to date)"
    Push-Location $seams
    try { & cargo build -j $Jobs -p pulso 2>&1 | Out-Host; if ($LASTEXITCODE -ne 0) { throw "cargo build failed ($LASTEXITCODE)" } } finally { Pop-Location }
    if (-not (Test-Path -LiteralPath $exe)) { throw "pulso.exe not found at $exe (set CARGO_TARGET_DIR consistently)" }

    $node = [bool](Get-Command node -ErrorAction SilentlyContinue)
    $plan = Get-ConsoleBuildPlan -ConsoleDir $consoleDir -NodeAvailable $node
    switch ($plan) {
        'build' {
            Write-Host '[2/4] building the console (needs node; npm ci only when node_modules is missing)'
            Push-Location $consoleDir
            try {
                if (-not (Test-Path -LiteralPath (Join-Path $consoleDir 'node_modules'))) { & npm ci 2>&1 | Out-Host; if ($LASTEXITCODE -ne 0) { throw 'npm ci failed' } }
                & npm run build 2>&1 | Out-Host; if ($LASTEXITCODE -ne 0) { throw 'npm run build failed' }
            } finally { Pop-Location }
        }
        'present' { Write-Host '[2/4] console build is up to date (debug-console/dist)' }
        'stale-present' { Write-Warning 'console sources are newer than debug-console/dist and node is not installed: serving the OLD build' }
        'api-only' { Write-Warning 'node is not installed and there is no console build: serving the API only (no UI). Install node 22 to see the console.' }
    }

    Write-Host "[3/4] starting pulso serve on 127.0.0.1:$Port (loopback only)"
    $token = New-DemoToken
    $psi = New-Object Diagnostics.ProcessStartInfo
    $psi.FileName = $exe
    $psi.Arguments = "serve --addr 127.0.0.1:$Port --exit-on-stdin-eof"
    $psi.UseShellExecute = $false
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardInput = $true   # held open: if this script is hard-killed the pipe closes and serve exits on its own
    $psi.Environment['PULSO_ADMIN_TOKEN'] = $token
    $server = [Diagnostics.Process]::Start($psi)
    $line = $server.StandardOutput.ReadLineAsync()
    if (-not $line.Wait(20000)) { throw 'pulso serve did not report a listening address within 20 s' }
    $base = Get-ListeningUrl -Line $line.Result
    if (-not $base) { throw "pulso serve did not start (is port $Port busy?). It said: $($line.Result)" }
    $runId = 'run-demo0-' + (Get-Date -Format 'yyyyMMddHHmmss')
    $url = "$base/#/run/$runId"
    Write-Host ''
    Write-Host "  Console (live run): $url"
    Write-Host "  Console (all runs): $base/"
    Write-Host ''
    if ($plan -ne 'api-only' -and -not $NoBrowser) {
        try { Start-Process $url } catch { Write-Warning "could not open a browser ($($_.Exception.Message)); open the URL above" }
        Start-Sleep -Seconds 4
    }

    Write-Host "[4/4] running the demo (DEMO-0, offline Core double), pace $PaceMs ms per step"
    $dargs = @('demo', '--api', ($base -replace '^http://', ''), '--pace-ms', "$PaceMs", '--run-id', $runId)
    if ($RealCore) { $dargs += '--real-core' }
    try { $sha = (& git -C $root rev-parse HEAD 2>$null); if ($sha -match '^[0-9a-f]{40}$') { $dargs += @('--sha', $sha) } } catch { }
    $dpsi = New-Object Diagnostics.ProcessStartInfo
    $dpsi.FileName = $exe
    $dpsi.Arguments = ($dargs | ForEach-Object { if ($_ -match '\s') { '"' + $_ + '"' } else { $_ } }) -join ' '
    $dpsi.UseShellExecute = $false
    $dpsi.Environment['PULSO_ADMIN_TOKEN'] = $token
    $demo = [Diagnostics.Process]::Start($dpsi)
    $demo.WaitForExit()
    $code = $demo.ExitCode
    if ($code -eq 0 -and -not $NoHold) {
        Write-Host ''
        Write-Host "The run stays visible at $url . Press Enter (or Ctrl+C) to stop the server."
        [void][Console]::In.ReadLine()
    }
    exit $code
} finally {
    Stop-Started
}

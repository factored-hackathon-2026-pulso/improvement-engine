<#
.SYNOPSIS
  One command: `pulso run` in PLATFORM mode over a simulated platform product stream, watched live in the debug-console.
  scripts/demo-platform.ps1 [-Port 4021] [-Scenario escalation_rise|null] [-Backfill 18000] [-Horizon 26000] [-Rate 200]
                            [-MeanGapSeconds 900] [-WaitSeconds 240] [-NoBrowser] [-NoHold] [-KeepData] [-SkipConsoleBuild] [-Jobs 1]
.DESCRIPTION
  1. refuses a bad command line before anything starts (guards in demo-platform.lib.ps1: a backfill too short for the sensor's
     14-day cold-start gate, a horizon not beyond it, a port or rate out of range, an unknown scenario, no `uv`).
  2. builds `pulso` (cargo build -j N -p pulso) and the console when node exists (same plan as demo-magic.ps1; -SkipConsoleBuild
     serves an existing debug-console/dist or the API only).
  3. starts the Python simulator (platform-sim/product_stream, `uv run --python 3.12`) writing a platform-shaped SQLite product
     source: first -Backfill events at once, then it keeps appending at -Rate events/s (follow mode).
  4. starts `pulso run` (loopback only): data mode platform, adapter product-sqlite, the real Rust `rust-events` sensor through
     `monitor::tick`, the engine job worker, `thread10::pipeline::run_signals` (one proposal and one ledger verdict per admitted
     signal), events into the console in process. Prints the console URL and opens it (-NoBrowser skips).
  5. waits (up to -WaitSeconds) for the first admitted signal to finish, then prints doubles[] (what is NOT real) FIRST and the
     findings after, and holds the server for you to look around (Enter or Ctrl+C) unless -NoHold.
  6. stops everything it started (both processes get stdin EOF, then are killed if still alive) and removes its temp data (-KeepData keeps it).
  WHAT IS REAL: the Rust sensor over platform-shaped data, the Rust pipeline, the ledger, the console store.
  WHAT IS SIMULATED: the data (simulator), scout and verifier answers (scripted), the Core (offline double: it never makes a
  proposal viable), the human decision, the release and the observation window. No quality claim.
  Exit code: 0 ok; 1 a process failed; 2 refused by a guard.
#>
[CmdletBinding()]
param([int]$Port = 4021, [string]$Scenario = 'escalation_rise', [int]$Backfill = 18000, [int]$Horizon = 26000, [int]$Rate = 200,
    [int]$MeanGapSeconds = 900, [int]$WaitSeconds = 240, [switch]$NoBrowser, [switch]$NoHold, [switch]$KeepData, [switch]$SkipConsoleBuild, [int]$Jobs = 1)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$root = (Resolve-Path (Join-Path $here '..')).Path
. (Join-Path $here 'demo-magic.lib.ps1')
. (Join-Path $here 'demo-platform.lib.ps1')

$seams = Join-Path $root 'seams'
$consoleDir = Join-Path $root 'debug-console'
$simDir = Join-Path $root 'platform-sim'
$targetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $seams 'target' }
$exe = Join-Path $targetDir 'debug\pulso.exe'
$sim = $null; $pulso = $null; $data = $null

function Join-Args([string[]]$a) { ($a | ForEach-Object { if ($_ -match '\s') { '"' + $_ + '"' } else { $_ } }) -join ' ' }

function Stop-Started {
    foreach ($p in @($pulso, $sim)) {
        if ($p -and -not $p.HasExited) { try { $p.StandardInput.Close() } catch { } }
    }
    foreach ($p in @($pulso, $sim)) {
        if ($p -and -not $p.HasExited) {
            try { if (-not $p.WaitForExit(8000)) { $p.Kill($true) } } catch { try { & taskkill.exe /PID $p.Id /T /F 2>&1 | Out-Null } catch { try { $p.Kill() } catch { } } }   # Windows PowerShell 5.1 has no Kill($true): taskkill /T keeps `uv`'s python child from being orphaned
        }
    }
}

function Get-Json([string]$Url) { Invoke-RestMethod -Uri $Url -TimeoutSec 10 }

try {
    try { $plan = Get-DemoPlatformPlan -Backfill $Backfill -Horizon $Horizon -Rate $Rate -MeanGapSeconds $MeanGapSeconds -Port $Port -Scenario $Scenario }
    catch { Write-Error "demo-platform: $($_.Exception.Message)" -ErrorAction Continue; exit 2 }
    if (-not (Get-Command uv -ErrorAction SilentlyContinue)) { Write-Error 'demo-platform: `uv` is not installed (the simulator runs as `uv run --python 3.12 python -m product_stream`)' -ErrorAction Continue; exit 2 }

    Write-Host "[1/5] building pulso (cargo build -j $Jobs -p pulso; a no-op when up to date)"
    Push-Location $seams
    $ErrorActionPreference = 'Continue'   # cargo reports progress on stderr; that is not an error
    try { & cargo build -j $Jobs -p pulso 2>&1 | Out-Host; if ($LASTEXITCODE -ne 0) { throw "cargo build failed ($LASTEXITCODE)" } } finally { Pop-Location; $ErrorActionPreference = 'Stop' }
    if (-not (Test-Path -LiteralPath $exe)) { throw "pulso.exe not found at $exe (set CARGO_TARGET_DIR consistently)" }

    $node = [bool](Get-Command node -ErrorAction SilentlyContinue) -and -not $SkipConsoleBuild
    $cplan = Get-ConsoleBuildPlan -ConsoleDir $consoleDir -NodeAvailable $node
    switch ($cplan) {
        'build' {
            Write-Host '[2/5] building the console (needs node; npm ci only when node_modules is missing)'
            Push-Location $consoleDir
            try {
                $ErrorActionPreference = 'Continue'
                if (-not (Test-Path -LiteralPath (Join-Path $consoleDir 'node_modules'))) { & npm ci 2>&1 | Out-Host; if ($LASTEXITCODE -ne 0) { throw 'npm ci failed' } }
                & npm run build 2>&1 | Out-Host; if ($LASTEXITCODE -ne 0) { throw 'npm run build failed' }
            } finally { Pop-Location; $ErrorActionPreference = 'Stop' }
        }
        'present' { Write-Host '[2/5] console build is up to date (debug-console/dist)' }
        'stale-present' { Write-Warning 'console sources are newer than debug-console/dist and node is not installed: serving the OLD build' }
        'api-only' { Write-Warning 'no console build (node is missing or -SkipConsoleBuild was given): serving the API only (no UI). Install node 22 and drop -SkipConsoleBuild to see the console.' }
    }

    $data = Join-Path ([IO.Path]::GetTempPath()) ('demo-platform-' + (Get-Date -Format 'yyyyMMddHHmmss'))
    New-Item -ItemType Directory -Path $data | Out-Null
    $sqlite = Join-Path $data 'product.db'
    Write-Host "[3/5] starting the simulator: $($plan.Scenario), backfill $($plan.Backfill) events then follow at $($plan.Rate)/s (data: $data)"
    $psi = New-Object Diagnostics.ProcessStartInfo
    $psi.FileName = (Get-Command uv).Source
    $psi.Arguments = Join-Args (Get-SimulatorArgs -Plan $plan -Sqlite $sqlite)
    $psi.WorkingDirectory = $simDir
    $psi.UseShellExecute = $false
    $psi.RedirectStandardInput = $true; $psi.RedirectStandardOutput = $true; $psi.RedirectStandardError = $true
    $sim = [Diagnostics.Process]::Start($psi)
    $null = $sim.StandardOutput.ReadToEndAsync(); $null = $sim.StandardError.ReadToEndAsync()
    $until = (Get-Date).AddSeconds(90)
    while ((Get-Date) -lt $until) {
        if ($sim.HasExited) { throw "the simulator exited early (code $($sim.ExitCode))" }
        if ((Test-Path -LiteralPath $sqlite) -and (Get-Item -LiteralPath $sqlite).Length -gt ($plan.Backfill * 40)) { break }
        Start-Sleep -Milliseconds 500
    }
    if (-not (Test-Path -LiteralPath $sqlite) -or (Get-Item -LiteralPath $sqlite).Length -le ($plan.Backfill * 40)) { throw 'the simulator did not write the backfill within 90 s' }

    Write-Host "[4/5] starting pulso run (platform mode, product-sqlite, loopback only) on 127.0.0.1:$($plan.Port)"
    $dist = Join-Path $consoleDir 'dist'
    $envs = Get-PulsoRunEnvironment -Plan $plan -Sqlite $sqlite -WorkDir (Join-Path $data 'work') -StoreDir (Join-Path $data 'store') -ConsoleDir $(if (Test-Path -LiteralPath (Join-Path $dist 'index.html')) { $dist } else { '' })
    $rpsi = New-Object Diagnostics.ProcessStartInfo
    $rpsi.FileName = $exe
    $rpsi.Arguments = 'run --exit-on-stdin-eof'
    $rpsi.UseShellExecute = $false
    $rpsi.RedirectStandardInput = $true; $rpsi.RedirectStandardOutput = $true
    foreach ($k in $envs.Keys) { $rpsi.Environment[$k] = $envs[$k] }
    $rpsi.Environment['STEPS_RUNNER_EXE'] = Join-Path (Split-Path $exe) 'pulso-synth-runner.exe'
    $pulso = [Diagnostics.Process]::Start($rpsi)
    $base = $null
    $until = (Get-Date).AddSeconds(30)
    while (-not $base -and (Get-Date) -lt $until) {
        $task = $pulso.StandardOutput.ReadLineAsync()
        if (-not $task.Wait(5000)) { continue }
        if ($null -eq $task.Result) { throw "pulso run ended before listening (exit $($pulso.ExitCode))" }
        $base = Get-RunListeningUrl -Line $task.Result
    }
    if (-not $base) { throw "pulso run did not report a listening address within 30 s (is port $($plan.Port) busy?)" }
    $null = $pulso.StandardOutput.ReadToEndAsync()   # keep draining its JSON log so it never blocks on a full pipe
    Write-Host ''
    Write-Host "  Console: $base/"
    Write-Host "  Runs:    $base/#/runs"
    Write-Host ''
    if ($cplan -ne 'api-only' -and -not $NoBrowser) { try { Start-Process "$base/" } catch { Write-Warning "could not open a browser ($($_.Exception.Message)); open the URL above" } }

    Write-Host "[5/5] waiting up to $WaitSeconds s for the first admitted signal (the sensor needs 14 days of history and a replicated effect)"
    $first = $null
    $until = (Get-Date).AddSeconds($WaitSeconds)
    while ((Get-Date) -lt $until -and -not $first) {
        Start-Sleep -Seconds 2
        if ($pulso.HasExited) { throw "pulso run exited (code $($pulso.ExitCode))" }
        try {
            $runs = (Get-Json "$base/internal/v1/debug/runs").items
            $first = $runs | Where-Object { $_.state -eq 'completed' -and $_.title -match '[1-9]\d* signal\(s\) admitted' } | Select-Object -First 1
            Write-Host ("  runs: {0}, with an admitted signal: {1}" -f @($runs).Count, @($runs | Where-Object { $_.title -match '[1-9]\d* signal\(s\) admitted' }).Count)
        } catch { }
    }

    $runs = @((Get-Json "$base/internal/v1/debug/runs").items)
    $profile = Get-Json "$base/internal/v1/debug/profile"
    $findings = @()
    foreach ($r in $runs) {
        $ev = (Get-Json "$base/internal/v1/debug/runs/$($r.run_id)/events").items
        foreach ($e in @($ev | Where-Object { $_.kind -eq 'proposal_verdict' })) {
            $findings += [pscustomobject]@{ run_id = $r.run_id; signal_id = $e.data.signal_id; verdict = $e.data.verdict; reason = $e.data.reason; detail = $e.data.detail }
        }
    }
    $label = if ($runs.Count -gt 0) { ($runs | Select-Object -Last 1).title } else { '' }
    Write-Host ''
    Format-DemoPlatformReport -Doubles @($profile.doubles_detail) -Findings $findings -Label $label -Runs $runs.Count | ForEach-Object { Write-Host $_ }
    if ($first) { Write-Host ''; Write-Host "  First run with a proposal: $base/#/run/$($first.run_id)" }
    elseif ($plan.Scenario -ne 'null') { Write-Warning "no signal was admitted within $WaitSeconds s; raise -WaitSeconds or check the run list at $base/" }

    if (-not $NoHold) {
        Write-Host ''
        Write-Host "The runs stay visible at $base/ . Press Enter (or Ctrl+C) to stop everything."
        [void][Console]::In.ReadLine()
    }
    exit 0
} catch {
    Write-Error "demo-platform: $($_.Exception.Message)" -ErrorAction Continue
    exit 1
} finally {
    Stop-Started
    if ($data -and -not $KeepData -and (Test-Path -LiteralPath $data)) { try { Remove-Item -LiteralPath $data -Recurse -Force -ErrorAction Stop } catch { Write-Warning "could not remove $data" } }
    elseif ($data) { Write-Host "data kept at $data" }
}

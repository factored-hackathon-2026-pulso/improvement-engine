# Pure helpers of scripts/demo-platform.ps1 (dot-sourced; unit-tested by demo-platform.Tests.ps1). No network, no processes.

# Events per simulated case in platform-sim/product_stream (about 10.7); the sensor needs 14 days of event time before it admits
# anything (cold-start gate), so the backfill must span at least that.
$script:EventsPerCase = 10.8
$script:ColdStartDays = 14

function Get-MinEventsForHistory {
    param([Parameter(Mandatory)][int]$MeanGapSeconds)
    # one day of margin over the 14-day gate
    [int][math]::Ceiling(($script:ColdStartDays + 1) * 86400 / $MeanGapSeconds * $script:EventsPerCase)
}

# Validates the command line BEFORE anything is built or started and returns the plan. Every refusal names the parameter.
function Get-DemoPlatformPlan {
    param([int]$Backfill = 18000, [int]$Horizon = 26000, [int]$Rate = 200, [int]$MeanGapSeconds = 900, [int]$Port = 4021, [int]$Seed = 7, [string]$Scenario = 'escalation_rise')
    if ($Port -lt 1024 -or $Port -gt 65535) { throw "-Port $Port is outside 1024..65535 (the console is served on loopback only)" }
    if ($MeanGapSeconds -lt 1) { throw "-MeanGapSeconds must be >= 1" }
    if ($Rate -lt 1 -or $Rate -gt 5000) { throw "-Rate $Rate is outside 1..5000 events per second" }
    if (@('escalation_rise', 'null', 'recurrence_rise', 'volume_drift') -notcontains $Scenario) { throw "-Scenario $Scenario is not one of escalation_rise, null, recurrence_rise, volume_drift" }
    $min = Get-MinEventsForHistory -MeanGapSeconds $MeanGapSeconds
    if ($Backfill -lt $min) { throw "-Backfill $Backfill events span fewer than $($script:ColdStartDays) days of event time at a $MeanGapSeconds s mean gap: the sensor cold-start gate would admit nothing. Use at least $min." }
    if ($Horizon -le $Backfill) { throw "-Horizon $Horizon must be larger than -Backfill $Backfill (the simulator keeps appending until the horizon, then until stopped)" }
    [pscustomobject]@{ Backfill = $Backfill; Horizon = $Horizon; Rate = $Rate; MeanGapSeconds = $MeanGapSeconds; Port = $Port; Seed = $Seed; Scenario = $Scenario; MinEvents = $min }
}

# Arguments of `uv run ... python -m product_stream` in follow mode (backfill first, then appends at -Rate until stdin closes).
function Get-SimulatorArgs {
    param([Parameter(Mandatory)]$Plan, [Parameter(Mandatory)][string]$Sqlite)
    @('run', '--python', '3.12', 'python', '-m', 'product_stream', '--sqlite', $Sqlite, '--scenario', $Plan.Scenario, '--seed', "$($Plan.Seed)",
        '--backfill', "$($Plan.Backfill)", '--follow', '--rate', "$($Plan.Rate)", '--batch', '200', '--horizon-events', "$($Plan.Horizon)",
        '--mean-gap-s', "$($Plan.MeanGapSeconds)", '--stop-on-stdin-eof', '--overwrite')
}

# The environment of `pulso run` for this demo (never a secret; no DSN, no token). PULSO_SOURCE_PROVENANCE=simulated makes the console say so.
function Get-PulsoRunEnvironment {
    param([Parameter(Mandatory)]$Plan, [Parameter(Mandatory)][string]$Sqlite, [Parameter(Mandatory)][string]$WorkDir, [Parameter(Mandatory)][string]$StoreDir, [string]$ConsoleDir)
    $e = [ordered]@{
        PULSO_STORAGE = 'memory'; PULSO_DATA_MODE = 'platform'; PULSO_SOURCE_ADAPTER = 'product-sqlite'; PULSO_SOURCE_ID = 'platform:sim'
        PULSO_SOURCE_SQLITE = $Sqlite; PULSO_SOURCE_PROVENANCE = 'simulated'; PULSO_WORK_DIR = $WorkDir; PULSO_STORE_DIR = $StoreDir
        PULSO_LISTEN_ADDR = "127.0.0.1:$($Plan.Port)"; PULSO_POLL_INTERVAL_MS = '2000'; PULSO_READ_BATCH = '3000'
        PULSO_MODEL_PORT = 'scripted'; PULSO_CORE_PORT = 'offline'
    }
    if ($ConsoleDir) { $e['PULSO_CONSOLE_DIR'] = $ConsoleDir }
    $e
}

# `{"event":"listening","addr":"127.0.0.1:4021"}` (a JSON log line on stdout) -> 'http://127.0.0.1:4021'; anything else -> $null.
function Get-RunListeningUrl {
    param([string]$Line)
    if (-not $Line) { return $null }
    try { $j = $Line | ConvertFrom-Json -ErrorAction Stop } catch { return $null }
    if ($j.event -eq 'listening' -and $j.addr -match '^127\.0\.0\.1:\d+$') { return "http://$($j.addr)" }
    return $null
}

# The closing report: doubles[] (what is NOT real) FIRST, then the findings. `Doubles` are items {id, what}; `Findings` are items
# {run_id, signal_id, verdict, reason, detail}. Returns lines; the caller prints them.
function Format-DemoPlatformReport {
    param([object[]]$Doubles = @(), [object[]]$Findings = @(), [string]$Label = '', [int]$Runs = 0)
    $l = New-Object System.Collections.Generic.List[string]
    $l.Add('NOT REAL (doubles[], listed first; everything below is only as real as this list allows):')
    if (@($Doubles).Count -eq 0) { $l.Add('- (none declared yet: the run has not started)') }
    foreach ($d in @($Doubles)) { $l.Add("- $($d.id): $($d.what)") }
    $l.Add('')
    if ($Label) { $l.Add("RUN LABEL: $Label") }
    $l.Add("FINDINGS ($Runs monitor run(s); one proposal and one ledger verdict per admitted signal; the verdict comes from the gate on the offline Core double, never from a quality claim):")
    if (@($Findings).Count -eq 0) { $l.Add('- no signal was admitted (the sensor admits a cell only after 14 days of history and a replicated, multiplicity-corrected difference)') }
    foreach ($f in @($Findings)) { $l.Add("- $($f.run_id): $($f.signal_id) -> $($f.verdict) ($($f.reason)): $($f.detail)") }
    $l.ToArray()
}

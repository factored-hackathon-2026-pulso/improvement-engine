# Pure helpers of scripts/integrated-rig/*.ps1 (dot-sourced; unit-tested by rig.Tests.ps1).
# Windows PowerShell 5.1 AND pwsh: no `??`, no ternary, no `&&`, no `-AsHashtable`. Nothing here prints a secret: credentials only
# travel inside hashtables handed to child processes, and everything a child prints goes through Protect-Text first.

$rigHere = Split-Path -Parent $MyInvocation.MyCommand.Path
. (Join-Path $rigHere '..\demo-loop\run.lib.ps1')

# ---- settings ------------------------------------------------------------------------------------------------------------------------

# Own stack, own ports. pulso-l3 (shared dev stack), pulso-demo (demo loop) and the other lanes' prefixes are refused.
function Get-RigEnvInt {
    param([string]$Name, [int]$Default)
    $v = [Environment]::GetEnvironmentVariable($Name)
    if ($v -and $v -match '^\d{4,5}$') { return [int]$v }
    $Default
}

# AGT1: a lane may run its own rig beside ENV1: PULSO_STACK_PREFIX and PULSO_RIG_{PG,GW,CORE,ENGINE,PLATFORM,SPA}_PORT override the defaults
# (explicit parameters still win).
function Get-RigSettings {
    param([string]$Prefix = '', [int]$PgPort = 0, [int]$GwPort = 0, [int]$CorePort = 0, [int]$EnginePort = 0, [int]$PlatformPort = 0, [int]$SpaPort = 0)
    if (-not $Prefix) { $Prefix = $(if ($env:PULSO_STACK_PREFIX) { $env:PULSO_STACK_PREFIX } else { 'pulso-env1' }) }
    if (-not $PgPort) { $PgPort = Get-RigEnvInt 'PULSO_RIG_PG_PORT' 55510 }
    if (-not $GwPort) { $GwPort = Get-RigEnvInt 'PULSO_RIG_GW_PORT' 8210 }
    if (-not $CorePort) { $CorePort = Get-RigEnvInt 'PULSO_RIG_CORE_PORT' 8211 }
    if (-not $EnginePort) { $EnginePort = Get-RigEnvInt 'PULSO_RIG_ENGINE_PORT' 4210 }
    if (-not $PlatformPort) { $PlatformPort = Get-RigEnvInt 'PULSO_RIG_PLATFORM_PORT' 8200 }
    if (-not $SpaPort) { $SpaPort = Get-RigEnvInt 'PULSO_RIG_SPA_PORT' 5174 }
    $base = Get-DemoStackSettings -Prefix $Prefix -PgPort $PgPort -GwPort $GwPort -CorePort $CorePort -EnginePort $EnginePort
    if ($Prefix -eq 'pulso-demo') { throw "stack prefix 'pulso-demo' belongs to the demo loop; pick another" }
    if ($PlatformPort -lt 1024 -or $PlatformPort -gt 65535) { throw "port $PlatformPort is outside 1024..65535" }
    $all = @($PgPort, $GwPort, $CorePort, $EnginePort, $PlatformPort, $SpaPort, $base.BatteryPg, $base.BatteryGw, $base.BatteryCore)
    if (@($all | Select-Object -Unique).Count -ne $all.Count) { throw 'the rig ports (including the battery ones the loop reserves) must all be different' }
    $base | Add-Member -NotePropertyName PlatformPort -NotePropertyValue $PlatformPort -PassThru | Add-Member -NotePropertyName SpaPort -NotePropertyValue $SpaPort -PassThru
}

function Get-RigPaths {
    param([Parameter(Mandatory)][string]$Root)
    $dev = Join-Path $Root '.dev-stack'
    $rig = Join-Path $dev 'integrated-rig'
    [pscustomobject]@{
        Dev = $dev; Rig = $rig; Tokens = (Join-Path $dev 'tokens.json'); Secrets = (Join-Path $rig 'secrets.json'); PlatformKeys = (Join-Path $rig 'platform-keys')
        PlatformDb = (Join-Path $rig 'cc_platform.db'); PlatformPid = (Join-Path $rig 'platform.pid'); SpaPid = (Join-Path $rig 'spa.pid'); SpaLog = (Join-Path $rig 'spa.log'); PlatformLog = (Join-Path $rig 'platform.log')
        AnnounceResults = (Join-Path $rig 'announce-results.json'); Story = (Join-Path $rig 'story-report.json'); CaseIds = (Join-Path $rig 'case-ids.json')
    }
}

# ---- memory budget ---------------------------------------------------------------------------------------------------------------------

function Get-FreeRamMb {
    try { return [int]((Get-CimInstance Win32_OperatingSystem).FreePhysicalMemory / 1024) } catch { return -1 }
}

# $true when the budget allows a start. A free value that could not be read (-1) never allows it.
function Test-RamBudget {
    param([int]$FreeMb, [int]$MinMb = 1500)
    ($FreeMb -ge 0) -and ($FreeMb -gt $MinMb)
}

# ---- credentials (generated per run, in memory; the only copy on disk is the gitignored secrets.json) ------------------------------------------

function New-ServiceToken {
    $b = New-Object byte[] 32
    $rng = New-Object Security.Cryptography.RNGCryptoServiceProvider
    $rng.GetBytes($b); $rng.Dispose()
    'svc-' + [Convert]::ToBase64String($b).Replace('+', 'a').Replace('/', 'b').Replace('=', 'c')
}

# Environment of `cc-api`. The three agent-core variables of ADR 0003/0007: CC_AGENT_CORE_URL + CC_AGENT_KEYS_FILE go together (the platform
# signs every agent-core credential, the announce read included, with the keys of that file); CC_INTERNAL_SERVICE_TOKEN opens
# /api/v1/internal/*. A new SQLite file: the platform has no migrations and the seed is created on an empty database.
function Get-PlatformEnvironment {
    param([Parameter(Mandatory)]$Settings, [Parameter(Mandatory)][string]$KeysFile, [Parameter(Mandatory)][string]$ServiceToken, [Parameter(Mandatory)][string]$DbPath)
    $db = 'sqlite+aiosqlite:///' + ($DbPath -replace '\\', '/')
    [ordered]@{
        CC_ENV = 'dev'; CC_SEED_DEMO_DATA = 'true'; CC_DATABASE_URL = $db; CC_HOST = '127.0.0.1'; CC_PORT = "$($Settings.PlatformPort)"
        CC_AGENT_CORE_URL = "http://127.0.0.1:$($Settings.CorePort)"; CC_AGENT_KEYS_FILE = $KeysFile; CC_INTERNAL_SERVICE_TOKEN = $ServiceToken
        CC_CORS_ORIGINS = ('["http://localhost:{0}","http://127.0.0.1:{0}"]' -f $Settings.SpaPort); CC_LOG_FORMAT = 'console'; CC_NOTIFICATION_SWEEP_SECONDS = '0'
    }
}

# agent-core -> platform (`grant_active`): not used by the builder flow, set so the platform route exists end to end.
function Get-AgentCoreGrantsEnvironment {
    param([Parameter(Mandatory)]$Settings, [Parameter(Mandatory)][string]$ServiceToken)
    [ordered]@{ AGENTCORE_GRANTS_URL = "http://127.0.0.1:$($Settings.PlatformPort)"; AGENTCORE_GRANTS_TOKEN = $ServiceToken }
}

# Variables for the `run.ps1` of the demo loop (its own settings read PULSO_STACK_PREFIX and PULSO_DEMO_*_PORT).
function Get-LoopLaneEnvironment {
    param([Parameter(Mandatory)]$Settings)
    [ordered]@{ PULSO_STACK_PREFIX = $Settings.Prefix; PULSO_DEMO_PG_PORT = "$($Settings.PgPort)"; PULSO_DEMO_GW_PORT = "$($Settings.GwPort)"
        PULSO_DEMO_CORE_PORT = "$($Settings.CorePort)"; PULSO_DEMO_ENGINE_PORT = "$($Settings.EnginePort)" }
}

# ---- secrets never printed -----------------------------------------------------------------------------------------------------------------

# $true when none of the secret values (6+ chars) occurs in $Text. The canary test feeds it with everything a script printed.
function Test-OutputClean {
    param([AllowNull()][AllowEmptyString()][string]$Text, [string[]]$Secrets = @())
    if ([string]::IsNullOrEmpty($Text)) { return $true }
    foreach ($s in $Secrets) { if ($s -and $s.Length -ge 6 -and $Text.Contains($s)) { return $false } }
    $true
}

# Names (never values) of the environment variables a hashtable would set: what logs may say about a child environment.
function Get-EnvNames { param([System.Collections.IDictionary]$Env) @($Env.Keys | ForEach-Object { [string]$_ } | Sort-Object) }

# ---- state files ---------------------------------------------------------------------------------------------------------------------------

function Write-JsonFile {
    param([Parameter(Mandatory)][string]$Path, [Parameter(Mandatory)]$Obj, [int]$Depth = 8)
    $dir = Split-Path -Parent $Path
    if ($dir) { $null = New-Item -ItemType Directory -Force -Path $dir }
    [IO.File]::WriteAllText($Path, ($Obj | ConvertTo-Json -Depth $Depth), (New-Object Text.UTF8Encoding($false)))
}

function Stop-PidTree {
    param([string]$PidFile)
    if (-not (Test-Path -LiteralPath $PidFile)) { return $false }
    $id = ([IO.File]::ReadAllText($PidFile)).Trim()
    if ($id -match '^\d+$') { try { & taskkill /PID $id /T /F 2>&1 | Out-Null } catch { } }
    Remove-Item -LiteralPath $PidFile -Force -ErrorAction SilentlyContinue
    $true
}

function Test-Http {
    param([string]$Url, [int]$TimeoutSec = 4)
    try { $r = Invoke-WebRequest -Uri $Url -UseBasicParsing -TimeoutSec $TimeoutSec; return ($r.StatusCode -eq 200) } catch { return $false }
}

# The shell that runs a child .ps1 with the same edition as the caller (pwsh stays pwsh; Windows PowerShell 5.1 stays 5.1).
function Get-ChildShell {
    $name = $(if ($PSVersionTable.PSEdition -eq 'Core') { 'pwsh' } else { 'powershell' })
    $cmd = Get-Command $name -ErrorAction SilentlyContinue
    if ($cmd) { return $cmd.Source }
    (Get-Command powershell -ErrorAction Stop).Source
}

# ---- evidence links (G1) -------------------------------------------------------------------------------------------------------------------

# Deterministic pick of up to $Count ids out of the platform's seeded case ids, keyed by the evidence ref, so the same finding always links
# the same example cases. Only `CASE-` + 26 Crockford characters pass; anything else in the list is dropped, never sent.
function Select-EvidenceLinks {
    param([string[]]$CaseIds = @(), [string]$EvidenceRef = '', [int]$Count = 2)
    $valid = @($CaseIds | Where-Object { $_ -match '^CASE-[0-9A-HJKMNP-TV-Z]{26}$' } | Sort-Object -Unique)
    if ($valid.Count -eq 0) { return @() }
    $sha = [Security.Cryptography.SHA256]::Create()
    $h = $sha.ComputeHash([Text.Encoding]::UTF8.GetBytes("env1|$EvidenceRef"))
    $start = [int]([BitConverter]::ToUInt32($h, 0) % [uint32]$valid.Count)
    $n = [math]::Min([math]::Min($Count, 8), $valid.Count)
    $out = @()
    for ($i = 0; $i -lt $n; $i++) { $out += $valid[($start + $i) % $valid.Count] }
    $out
}

# ---- the real / stand-in / relaxed table -----------------------------------------------------------------------------------------------------

# Rows: @{ Item; Class (real | stand-in | relaxed); Note }. Aligned plain text, no secret can enter (callers pass labels only).
function Format-RealityTable {
    param([Parameter(Mandatory)][object[]]$Rows)
    $w1 = ($Rows | ForEach-Object { ([string]$_.Item).Length } | Measure-Object -Maximum).Maximum
    $w2 = 9
    $l = New-Object System.Collections.Generic.List[string]
    $l.Add(("{0}  {1}  {2}" -f 'what'.PadRight($w1), 'class'.PadRight($w2), 'note'))
    foreach ($r in $Rows) {
        if (@('real', 'stand-in', 'relaxed') -notcontains [string]$r.Class) { throw "class '$($r.Class)' is not real, stand-in or relaxed" }
        $l.Add(("{0}  {1}  {2}" -f ([string]$r.Item).PadRight($w1), ([string]$r.Class).PadRight($w2), $r.Note))
    }
    $l.ToArray()
}

# Fixed content of the table: the labels of this rig, in one place (the same list is pasted into docs/dev/INTEGRATED_RIG.md).
function Get-RigRealityRows {
    @(
        @{ Item = 'Postgres 16, llm-gateway, agent-core serve + registry'; Class = 'real'; Note = 'podman + host process, own prefix and ports; registry-e2e agents (disputas, consultas)' },
        @{ Item = 'Tools of the served agents'; Class = 'stand-in'; Note = 'agent-core e2e demo doubles (PULSO_SERVE_E2E=1); no tool-service' },
        @{ Item = 'Models (Scout, Verifier, Builder, judge)'; Class = 'real'; Note = 'real calls through the gateway; Builder xiaomi/mimo-v2.6-pro, flash fallback' },
        @{ Item = 'Treated cells'; Class = 'stand-in'; Note = 'planted profile: SYNTHETIC invented cells (one planted association); bank profile: cached real bank aggregates' },
        @{ Item = 'Engine loop, proof, dossier, proposal in agent-core'; Class = 'real'; Note = 'pulso.exe, origin auto_detect, state draft, never approved or published here' },
        @{ Item = 'Platform API, DB, seeded people and cases'; Class = 'real'; Note = 'support-platform main, own SQLite recreated at up, CC_SEED_DEMO_DATA' },
        @{ Item = 'Announce route and notifications'; Class = 'real'; Note = 'bearer service token, registry read-back, one notification per active supervisor' },
        @{ Item = 'Key trust between platform, engine and agent-core'; Class = 'real'; Note = 'merged public-key files (merge_keys.py); dev kids, regenerated per up' },
        @{ Item = 'Announce POST sender'; Class = 'relaxed'; Note = 'sent by run_story.ps1 from the engine record (engine-side announce cannot take the demo case ids, gap G1)' },
        @{ Item = 'evidenceLinks'; Class = 'relaxed'; Note = 'ids of SEEDED platform cases chosen deterministically, labelled "casos de ejemplo"; not the cases the finding came from' },
        @{ Item = 'Supervisor login, step-up'; Class = 'stand-in'; Note = 'password demo1234 and dev MFA code of the seeded accounts, CC_ENV=dev only' },
        @{ Item = 'Approve, publish, promote, outcome'; Class = 'relaxed'; Note = 'NOT run in stage 1 (never approve or publish here)' }
    )
}

# ---- cells summary (same as demo-loop/run.ps1) --------------------------------------------------------------------------------------------------
function Get-CellsInfo {
    param([string]$Path)
    $n = 0; $by = @{}
    foreach ($line in [IO.File]::ReadLines($Path)) {
        if (-not $line.Trim()) { continue }
        $n++
        if ($line -match '"metric":"([A-Z0-9]+)"') { $by[$Matches[1]] = 1 + [int]$by[$Matches[1]] }
    }
    $h = [Security.Cryptography.SHA256]::Create(); $fs = [IO.File]::OpenRead($Path); try { $sha = (($h.ComputeHash($fs) | ForEach-Object { $_.ToString('x2') }) -join '').Substring(0, 12) } finally { $fs.Dispose(); $h.Dispose() }
    [pscustomobject]@{ Rows = $n; ByMetric = (($by.Keys | Sort-Object | ForEach-Object { "$_=$($by[$_])" }) -join ' '); Sha = $sha }
}

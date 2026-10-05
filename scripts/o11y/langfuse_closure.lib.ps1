# Pure helpers of scripts/o11y/langfuse_closure.ps1 (dot-sourced; unit-tested by langfuse_closure.Tests.ps1).
# Windows PowerShell 5.1 AND pwsh: no `??`, no ternary, no `&&`, no `-AsHashtable`.
# Nothing here prints a credential VALUE: env files are read into hashtables that only feed redaction needles and child environments.

. (Join-Path $PSScriptRoot '../demo-loop/run.lib.ps1')

$script:LangfuseKeys = @('LANGFUSE_SECRET_KEY', 'LANGFUSE_PUBLIC_KEY', 'LANGFUSE_BASE_URL')
$script:ClosureOrder = @('Models', 'Up', 'Traffic', 'Verify', 'Down')

function Get-LfcPlan {
    param([switch]$Models, [switch]$Up, [switch]$Traffic, [switch]$Verify, [switch]$Down, [switch]$All, [switch]$KeepUp, [switch]$Mock)
    $want = @{ Models = [bool]$Models; Up = [bool]$Up; Traffic = [bool]$Traffic; Verify = [bool]$Verify; Down = [bool]$Down }
    if ($All) { foreach ($k in $script:ClosureOrder) { $want[$k] = $true } }
    if ($KeepUp) {
        if ($Down) { throw '-KeepUp and -Down contradict each other' }
        $want.Down = $false
    }
    $steps = @($script:ClosureOrder | Where-Object { $want[$_] })
    if ($steps.Count -eq 0) { throw 'nothing to do: pass -All, or any of -Models -Up -Traffic -Verify -Down' }
    if ($want.Down -and $want.Up -and -not $All) { throw '-Down and -Up in one command contradict each other (use -All, which tears down at the end)' }
    if ($want.Down -and ($want.Traffic -or $want.Verify) -and -not $All) { throw '-Down cannot be combined with -Traffic or -Verify (use -All)' }
    [pscustomobject]@{ Steps = $steps; Mock = [bool]$Mock; KeepUp = [bool]$KeepUp }
}

# Own stack: nothing shared with the demo stack (pulso-demo), the dev stack (pulso-l3) or another lane.
function Get-LfcSettings {
    param([string]$Prefix = 'pulso-lfc', [int]$PgPort = 55510, [int]$GwPort = 8210, [int]$CorePort = 8211, [int]$EnginePort = 4210, [int]$ForwarderPort = 4328, [int]$MockPort = 4399)
    $ports = @($PgPort, $GwPort, $CorePort, $EnginePort, $ForwarderPort, $MockPort)
    foreach ($p in $ports) { if ($p -lt 1024 -or $p -gt 65535) { throw "port $p is outside 1024..65535" } }
    if (@($ports | Select-Object -Unique).Count -ne $ports.Count) { throw 'the six ports must be different' }
    if ($Prefix -in @('pulso-l3', 'pulso-demo')) { throw "stack prefix '$Prefix' belongs to another stack; pick another" }
    if ($Prefix -notmatch '^[a-z][a-z0-9-]{2,30}$') { throw "stack prefix '$Prefix' must be lowercase letters, digits and hyphens" }
    [pscustomobject]@{ Prefix = $Prefix; PgPort = $PgPort; GwPort = $GwPort; CorePort = $CorePort; EnginePort = $EnginePort; ForwarderPort = $ForwarderPort; MockPort = $MockPort
        ForwarderUrl = "http://127.0.0.1:$ForwarderPort"; StateName = 'lfc' }
}

# Validates the env file by NAMES. Returns { Ok; Missing; Problems }. Values are never returned or printed.
function Test-LangfuseEnvFile {
    param([Parameter(Mandatory)][string]$Path, [switch]$AllowHttpLoopback)
    $r = [pscustomobject]@{ Ok = $false; Missing = @(); Problems = @(); Host = '' }
    if (-not (Test-Path -LiteralPath $Path)) { $r.Problems = @("env file not found: $Path"); $r.Missing = $script:LangfuseKeys; return $r }
    $v = Read-EnvFileValues -Path $Path
    $r.Missing = @($script:LangfuseKeys | Where-Object { -not $v.ContainsKey($_) })
    if ($r.Missing.Count -gt 0) { $r.Problems = @("missing or empty key name(s): $($r.Missing -join ', ')"); return $r }
    $u = $null
    if (-not [Uri]::TryCreate([string]$v['LANGFUSE_BASE_URL'], [UriKind]::Absolute, [ref]$u)) { $r.Problems = @('LANGFUSE_BASE_URL is not an absolute URL'); return $r }
    $r.Host = $u.Host
    $loop = @('127.0.0.1', 'localhost', '[::1]') -contains $u.Host
    if ($u.Scheme -ne 'https' -and -not ($loop -and $AllowHttpLoopback)) { $r.Problems = @('LANGFUSE_BASE_URL must be https for an external host') ; return $r }
    if (-not $loop -and $u.Host -notmatch 'langfuse') { $r.Problems = @("LANGFUSE_BASE_URL host '$($u.Host)' does not look like Langfuse; refusing to send there"); return $r }
    $r.Ok = $true
    $r
}

# Redaction needles for everything this script prints: the env-file values, the Basic header value, plus the usual secret env files.
function Get-LfcNeedles {
    param([hashtable]$LangfuseValues, [hashtable[]]$Others = @())
    $src = New-Object System.Collections.Generic.List[hashtable]
    $extra = @{}
    if ($LangfuseValues -and $LangfuseValues['LANGFUSE_PUBLIC_KEY'] -and $LangfuseValues['LANGFUSE_SECRET_KEY']) {
        $pair = "$($LangfuseValues['LANGFUSE_PUBLIC_KEY']):$($LangfuseValues['LANGFUSE_SECRET_KEY'])"
        $extra['basic'] = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($pair))
    }
    $src.Add($extra)
    if ($LangfuseValues) { $keep = @{}; foreach ($k in $script:LangfuseKeys) { if ($k -ne 'LANGFUSE_BASE_URL' -and $LangfuseValues.ContainsKey($k)) { $keep[$k] = $LangfuseValues[$k] } }; $src.Add($keep) }
    foreach ($o in $Others) { $src.Add($o) }
    @(Get-RedactionNeedles -Sources $src.ToArray())
}

# Environment of the stack processes (gateway container, agent-core serve) so that BOTH export to the local forwarder.
# Non-secret by construction: the Langfuse key never travels here, only the loopback forwarder address.
function Get-LfcStackEnvironment {
    param([Parameter(Mandatory)]$Settings)
    [ordered]@{
        OTEL_EXPORTER_OTLP_ENDPOINT = $Settings.ForwarderUrl
        OTEL_EXPORTER_OTLP_PROTOCOL = 'http/protobuf'
        OTEL_TRACES_EXPORTER = 'otlp'
        PULSO_GW_OTEL_SERVICE_NAME = 'llm-gateway'
        PULSO_CORE_OTEL_SERVICE_NAME = 'agentcore'
        LLM_GATEWAY_TRACE_CONTENT = '1'
        AGENTCORE_TRACE_CONTENT = '1'
        AGENTCORE_TRACE_LANGFUSE = '1'
        PULSO_GW_HOST_NETWORK = '1'
    }
}

# Trace ids printed by engine_trace.py: "story finding=0 trace_id=<32 hex> spans=..".
function Get-StoryTraceIds {
    param([string[]]$Lines)
    $ids = New-Object System.Collections.Generic.List[string]
    foreach ($l in $Lines) { if ($l -match '^story finding=\d+ trace_id=([0-9a-f]{32})\b') { if (-not $ids.Contains($Matches[1])) { $ids.Add($Matches[1]) } } }
    , $ids.ToArray()
}

# Findings of a value-loop outcome that can become a story (every record has an evidence_ref).
function Get-LoopFindingCount {
    param($Loop)
    @(@($Loop.findings) | Where-Object { $_.evidence_ref }).Count
}

# One line, no values: what a failed ping means.
function Get-HealthDiagnosis {
    param([string]$Status)
    if ($Status -match 'HTTP 200') { return 'ok' }
    if ($Status -match 'unreachable') { return 'Langfuse is not reachable from this machine (network, proxy or wrong LANGFUSE_BASE_URL host)' }
    "Langfuse answered '$Status' on /api/public/health (wrong region/host in LANGFUSE_BASE_URL, or an outage)"
}

# A step failure caused by a stack that is not up says so in plain words.
function Get-StackDownHint {
    param([string]$Message)
    if ($Message -match 'unreachable|URLError|not answering|ConnectionRefused|actively refused') {
        return 'agent-core unreachable = the lfc stack is not up. Run: .\scripts\o11y\langfuse_closure.ps1 -Up   (or -All, which brings it up itself)'
    }
    ''
}

# Pure helpers of scripts/demo-loop/run.ps1 (dot-sourced; unit-tested by run.Tests.ps1).
# Windows PowerShell 5.1 AND pwsh: no `??`, no ternary, no `&&`, no `-AsHashtable`, no `Join-String`.
# Nothing here prints a secret: values loaded from env files only travel in hashtables handed to child processes, and every line a child
# prints goes through Protect-Text first.

$script:MetricNames = @{
    'M1' = 'contact_unresolved_rate'; 'M2' = 'complaint_share_of_contacts'; 'M3' = 'complaint_share_of_unresolved'
    'M4' = 'pqr_open_rate'; 'M5' = 'pqr_sla_breach_rate'; 'M6' = 'survey_low_score_rate'; 'M6L' = 'survey_low_score_rate_linked'
    'M6R' = 'survey_low_score_rate_reason'; 'M6U' = 'survey_low_score_rate_unlinked'; 'M7' = 'digital_error_rate'
    'M8' = 'send_to_nonconsenting_rate'; 'M9' = 'tx_decline_rate'; 'M10' = 'handle_time_unresolved_share'
}

$script:StepOrder = @('Up', 'Cells', 'Loop', 'Probes', 'Show', 'Announce', 'Down')

# UTF-8 JSON file (Windows PowerShell 5.1 would read it as ANSI and mangle accents).
function Read-JsonFile {
    param([Parameter(Mandatory)][string]$Path)
    [IO.File]::ReadAllText($Path, (New-Object Text.UTF8Encoding($false))) | ConvertFrom-Json
}

# ---- argument handling --------------------------------------------------------------------------------------------------------------

# Validates the command line BEFORE anything is started and returns the plan. Every refusal names the parameter.
function Get-DemoLoopPlan {
    param([switch]$Up, [switch]$Cells, [switch]$Loop, [switch]$Show, [switch]$Down, [switch]$Probes, [switch]$Announce, [switch]$All,
        [switch]$Synthetic, [int]$MaxFindings = 4, [int]$TimeoutMin = 45, [int]$ProbeReps = 3, [string]$CellsFile = '')
    $want = @{ Up = [bool]$Up; Cells = [bool]$Cells; Loop = [bool]$Loop; Probes = [bool]$Probes; Show = [bool]$Show; Announce = [bool]$Announce; Down = [bool]$Down }
    if ($All) { foreach ($k in 'Up', 'Cells', 'Loop', 'Show') { $want[$k] = $true } }
    $steps = @($script:StepOrder | Where-Object { $want[$_] })
    if ($steps.Count -eq 0) {
        throw 'nothing to do: pass at least one of -Up -Cells -Loop -Probes -Show -Announce -Down (or -All = -Up -Cells -Loop -Show)'
    }
    if ($want.Down -and $want.Up) { throw '-Down and -Up in one command contradict each other' }
    if ($want.Down -and ($want.Loop -or $want.Probes)) { throw '-Down cannot be combined with -Loop or -Probes (they need the stack)' }
    if ($MaxFindings -lt 1 -or $MaxFindings -gt 50) { throw "-MaxFindings $MaxFindings is outside 1..50 (it bounds the model cost of one run)" }
    if ($TimeoutMin -lt 1 -or $TimeoutMin -gt 240) { throw "-TimeoutMin $TimeoutMin is outside 1..240" }
    if ($ProbeReps -lt 1 -or $ProbeReps -gt 5) { throw "-ProbeReps $ProbeReps is outside 1..5" }
    if ($Synthetic -and -not ($want.Cells -or $want.Loop)) { throw '-Synthetic only applies together with -Cells or -Loop' }
    if ($CellsFile -and $Synthetic) { throw '-CellsFile and -Synthetic are two different cell sources: pick one' }
    [pscustomobject]@{
        Steps = $steps; Synthetic = [bool]$Synthetic; MaxFindings = $MaxFindings; TimeoutMin = $TimeoutMin; ProbeReps = $ProbeReps; CellsFile = $CellsFile
        Mode = $(if ($Synthetic) { 'synthetic-planted' } elseif ($CellsFile) { 'bank-file' } else { 'bank' })
    }
}

# Own stack: prefix, ports and non-secret settings. The shared dev stack (pulso-l3) is never touched.
function Get-DemoStackSettings {
    param([string]$Prefix = 'pulso-demo', [int]$PgPort = 55490, [int]$GwPort = 8190, [int]$CorePort = 8191, [int]$EnginePort = 4190)
    foreach ($p in @($PgPort, $GwPort, $CorePort, $EnginePort)) { if ($p -lt 1024 -or $p -gt 65535) { throw "port $p is outside 1024..65535" } }
    if (@($PgPort, $GwPort, $CorePort, $EnginePort | Select-Object -Unique).Count -ne 4) { throw 'the four ports must be different' }
    if ($Prefix -notmatch '^[a-z][a-z0-9-]{2,30}$') { throw "stack prefix '$Prefix' must be lowercase letters, digits and hyphens" }
    if ($Prefix -eq 'pulso-l3') { throw "stack prefix 'pulso-l3' belongs to the shared dev stack; pick another" }
    [pscustomobject]@{ Prefix = $Prefix; PgPort = $PgPort; GwPort = $GwPort; CorePort = $CorePort; EnginePort = $EnginePort
        BatteryPrefix = "$Prefix-bat"; BatteryPg = $PgPort + 1; BatteryGw = $GwPort + 2; BatteryCore = $CorePort + 2 }
}

# Non-secret environment of dev-stack/stack.py (credentials are added by the scrubbed helper, not here).
function Get-StackEnvironment {
    param([Parameter(Mandatory)]$Settings, [Parameter(Mandatory)][string]$AgentCoreDir, [Parameter(Mandatory)][string]$GatewayDir)
    [ordered]@{
        PULSO_STACK_PREFIX = $Settings.Prefix; PULSO_PG_PORT = "$($Settings.PgPort)"; PULSO_GW_PORT = "$($Settings.GwPort)"; PULSO_CORE_PORT = "$($Settings.CorePort)"
        PULSO_AGENT_CORE_DIR = $AgentCoreDir; PULSO_LLM_GATEWAY_DIR = $GatewayDir
        PULSO_REGISTRY_DIR = (Join-Path $AgentCoreDir 'tests\fixtures\registry-e2e'); PULSO_SERVE_E2E = '1'; PULSO_SERVE_AGENTS = 'disputas,consultas'
    }
}

# Environment of `pulso run` for the value loop. Secrets (gateway key, registry token, admin token, platform token) arrive as parameters
# and only ever land in the returned hashtable, which is handed to the child process and nowhere else.
function Get-EngineEnvironment {
    param([Parameter(Mandatory)]$Settings, [Parameter(Mandatory)][string]$CellsPath, [Parameter(Mandatory)][string]$WorkDir, [Parameter(Mandatory)][string]$StoreDir,
        [Parameter(Mandatory)][ValidateSet('bank', 'synthetic')][string]$Source, [int]$MaxFindings = 4,
        [string]$GatewayKey = '', [string]$RegistryToken = '', [string]$AdminToken = '', [string]$PlatformUrl = '', [string]$PlatformToken = '', [string]$PythonCmd = 'python', [string]$BuilderModel = '')
    $e = [ordered]@{
        PULSO_STORAGE = 'memory'; PULSO_DATA_MODE = 'dataset'; PULSO_SOURCE_ADAPTER = 'stub'; PULSO_SOURCE_ID = 'dataset:demo-loop'
        PULSO_WORK_DIR = $WorkDir; PULSO_STORE_DIR = $StoreDir; PULSO_LISTEN_ADDR = "127.0.0.1:$($Settings.EnginePort)"; PULSO_POLL_INTERVAL_MS = '2000'
        PULSO_EXIT_ON_STDIN_EOF = '1'
        PULSO_CELLS_NDJSON = $CellsPath; PULSO_CELLS_SOURCE = $Source; PULSO_LOOP_MAX_FINDINGS = "$MaxFindings"
        PULSO_REGISTRY_ADDR = "127.0.0.1:$($Settings.CorePort)"; PULSO_REGISTRY_VIA = 'api'; PULSO_REGISTRY_ENV = 'local'
        PULSO_EVAL_BEFORE_ANNOUNCE = 'on'; PULSO_REGRESSION_PYTHON = $PythonCmd; PULSO_EVAL_TIMEOUT_SECS = '900'
        PULSO_LLM_GATEWAY = 'enabled'; PULSO_LLM_GATEWAY_ADDR = "127.0.0.1:$($Settings.GwPort)"
        PULSO_LLM_GATEWAY_MODEL = 'xiaomi/mimo-v2.6-flash'; PULSO_LLM_GATEWAY_BUILDER_ESCALATION_MODEL = 'xiaomi/mimo-v2.6-pro'
        PULSO_PROBE_GATEWAY = "http://127.0.0.1:$($Settings.GwPort)"
    }
    if ($Source -eq 'bank') { $e['PULSO_ALLOW_DERIVED_AGGREGATES'] = '1' }
    if ($BuilderModel) { $e['PULSO_LLM_GATEWAY_BUILDER_MODEL'] = $BuilderModel }
    if ($GatewayKey) { $e['PULSO_LLM_GATEWAY_KEY'] = $GatewayKey; $e['GATEWAY_TOKEN_AGENT_CORE'] = $GatewayKey }
    if ($RegistryToken) { $e['PULSO_REGISTRY_TOKEN'] = $RegistryToken }
    if ($AdminToken) { $e['PULSO_ADMIN_TOKEN'] = $AdminToken }
    if ($PlatformUrl -and $PlatformToken) {
        $e['PULSO_PLATFORM_URL'] = $PlatformUrl; $e['PULSO_PLATFORM_SERVICE_TOKEN'] = $PlatformToken; $e['PULSO_ANNOUNCE_TO_PLATFORM'] = 'on'
    } else {
        $e['PULSO_ANNOUNCE_TO_PLATFORM'] = 'off'
    }
    $e
}

# ---- env files and redaction --------------------------------------------------------------------------------------------------------

# KEY=VALUE lines (optionally quoted), same rules as dev-stack/stack.py load_env. Blank values are dropped. Returns a hashtable; never prints.
function Read-EnvFileValues {
    param([Parameter(Mandatory)][string]$Path)
    $out = @{}
    if (-not (Test-Path -LiteralPath $Path)) { return $out }
    foreach ($line in [IO.File]::ReadAllLines($Path)) {
        $t = $line.Trim()
        if (-not $t -or $t.StartsWith('#') -or $t.IndexOf('=') -lt 0) { continue }
        $i = $t.IndexOf('=')
        $k = $t.Substring(0, $i).Trim()
        $v = $t.Substring($i + 1).Trim()
        if ($v.Length -ge 2 -and ($v[0] -eq '"' -or $v[0] -eq "'") -and $v[$v.Length - 1] -eq $v[0]) { $v = $v.Substring(1, $v.Length - 2) }
        if ($k -and $v) { $out[$k] = $v }
    }
    $out
}

# Strings that must never appear in anything printed: every value of at least 6 characters, plus the long fragments inside it (so the
# password of a DSN or one token of a token map is masked even when it shows up alone).
function Get-RedactionNeedles {
    param([hashtable[]]$Sources = @())
    $set = New-Object 'System.Collections.Generic.HashSet[string]'
    foreach ($h in $Sources) {
        if (-not $h) { continue }
        foreach ($v in $h.Values) {
            $s = [string]$v
            if ($s.Length -lt 6) { continue }
            [void]$set.Add($s)
            foreach ($frag in ($s -split '[^A-Za-z0-9._\-+/=]+')) { if ($frag.Length -ge 12) { [void]$set.Add($frag) } }
        }
    }
    @($set | Sort-Object { $_.Length } -Descending)
}

function Protect-Text {
    param([AllowNull()][AllowEmptyString()][string]$Text, [string[]]$Needles = @())
    if ([string]::IsNullOrEmpty($Text)) { return $Text }
    $t = $Text
    foreach ($n in $Needles) { if ($n) { $t = $t.Replace($n, '***') } }
    $t
}

# Runs a child process with $Env added to ITS environment only (never to this process), streams its stdout line by line and returns
# { ExitCode; Output } with every needle masked. stderr is read after exit (also masked). Nothing else is ever echoed.
function Invoke-Scrubbed {
    param([Parameter(Mandatory)][string]$File, [string[]]$Arguments = @(), [System.Collections.IDictionary]$Env = @{}, [string[]]$Needles = @(),
        [string]$WorkDir = '', [switch]$Quiet, [string]$LogPath = '')
    $psi = New-Object Diagnostics.ProcessStartInfo
    $psi.FileName = $File
    $psi.Arguments = ($Arguments | ForEach-Object { if ($_ -match '[\s"]') { '"' + ($_ -replace '"', '\"') + '"' } else { $_ } }) -join ' '
    $psi.UseShellExecute = $false
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.StandardOutputEncoding = [Text.Encoding]::UTF8
    $psi.StandardErrorEncoding = [Text.Encoding]::UTF8
    $psi.CreateNoWindow = $true
    if ($WorkDir) { $psi.WorkingDirectory = $WorkDir }
    foreach ($k in $Env.Keys) { $psi.EnvironmentVariables[[string]$k] = [string]$Env[$k] }
    $p = [Diagnostics.Process]::Start($psi)
    $errTask = $p.StandardError.ReadToEndAsync()
    $lines = New-Object System.Collections.Generic.List[string]
    while ($true) {
        $l = $p.StandardOutput.ReadLine()
        if ($null -eq $l) { break }
        $safe = Protect-Text -Text $l -Needles $Needles
        $lines.Add($safe)
        if (-not $Quiet) { Write-Host $safe }
    }
    $p.WaitForExit()
    $err = Protect-Text -Text $errTask.Result -Needles $Needles
    if ($err) { foreach ($l in ($err -split "`r?`n")) { if ($l) { $lines.Add($l); if (-not $Quiet) { Write-Host $l } } } }
    if ($LogPath) { try { [IO.File]::AppendAllText($LogPath, (($lines.ToArray()) -join "`n") + "`n") } catch { } }
    [pscustomobject]@{ ExitCode = $p.ExitCode; Output = $lines.ToArray() }
}

# ---- report formatting --------------------------------------------------------------------------------------------------------------

$script:Inv = [Globalization.CultureInfo]::InvariantCulture
function Format-Pct { param([object]$Rate) if ($null -eq $Rate) { return '?' }; (([double]$Rate * 100).ToString('0.0', $script:Inv)) + '%' }

function Get-MetricLabel {
    param([string]$Metric)
    if ($script:MetricNames.ContainsKey($Metric)) { "$Metric $($script:MetricNames[$Metric])" } else { $Metric }
}

# "category=Cobro indebido channel=Phone" (keys sorted, as the sensor emits them).
function Format-Cell {
    param($Dims)
    if (-not $Dims) { return '(no dimensions)' }
    $pairs = @($Dims.PSObject.Properties | Sort-Object Name | ForEach-Object { "$($_.Name)=$($_.Value)" })
    $pairs -join ' '
}

# "62.1% vs 30.4% rest (+31.7 pp); holdout 61.0% vs 30.1%"
function Format-Effect {
    param($Signal)
    if (-not $Signal -or -not $Signal.discovery) { return 'n/a' }
    $d = $Signal.discovery
    $pp = ([double]$d.diff * 100).ToString('+0.0;-0.0', $script:Inv)
    $s = "$(Format-Pct $d.rate) vs $(Format-Pct $d.baseline_rate) rest ($pp pp)"
    if ($Signal.holdout) { $s += "; holdout $(Format-Pct $Signal.holdout.rate) vs $(Format-Pct $Signal.holdout.baseline_rate)" }
    $s
}

# Outcome of one finding record: announced | not_announced:<verdict> | proven_not_delivered:<why> | unlinked (<reason>) | blocked (<reason>) | ...
function Get-FindingOutcome {
    param($Record)
    if ($Record.outcome) { return [string]$Record.outcome }
    switch ([string]$Record.status) {
        'unlinked' { return "unlinked ($($Record.reason))" }
        'blocked' { return "blocked ($($Record.reason))" }
        'no_change' { return "no_change ($($Record.reason))" }
        'proposed' {
            if ($Record.delivery -and $Record.delivery.status -eq 'delivered') { return 'draft_delivered (not proven)' }
            return 'proposed (not delivered)'
        }
        default { return [string]$Record.status }
    }
}

function Get-ProposalSlug {
    param($Record)
    if (-not $Record.target_ref) { return '-' }
    $t = [string]$Record.target_ref
    $i = $t.IndexOf(':')
    if ($i -ge 0) { $t = $t.Substring($i + 1) }
    $j = $t.LastIndexOf('/')
    if ($j -ge 0) { $t = $t.Substring($j + 1) }
    $t
}

# The regression verdict story in ONE line. A proof that could not run says why (the step and the code of the first problem), not a story.
function Get-ProofLine {
    param($Record)
    $ev = $Record.evaluation
    if (-not $ev) { return '-' }
    if ($ev.verdict -eq 'infra_failed') {
        $why = ''
        foreach ($a in @($ev.attempts)) { if (-not $why -and $a.problem) { $why = " ($($a.problem.step) $($a.problem.code) HTTP $($a.problem.http))" } }
        if (-not $why -and $ev.base -and $ev.base.problem) { $why = " ($($ev.base.problem.step) $($ev.base.problem.code) HTTP $($ev.base.problem.http))" }
        return ("infra_failed: {0}{1}" -f $ev.reason, $why)
    }
    $st = $null
    if ($ev.story_text) { $st = $ev.story_text.es }
    if ($st) { return ("{0}: {1}" -f $ev.verdict, $st) }
    "$($ev.verdict) ($($ev.reason))"
}

function Format-Cost { param($Record) $c = 0.0; if ($Record.metering -and $null -ne $Record.metering.cost_usd) { $c = [double]$Record.metering.cost_usd }; if ($c -le 0) { $c = 0.0 }; '$' + $c.ToString('0.0000', $script:Inv) }

# Lines for the -Loop step. $Signals is the sensor output signals array (index i <-> finding_{i+1}) or $null when the sensor binary is unavailable.
function Format-LoopReport {
    param([Parameter(Mandatory)]$Loop, $Signals = $null, [string]$Mode = '', [string]$CellsLabel = '')
    $l = New-Object System.Collections.Generic.List[string]
    $s = $Loop.summary
    $l.Add("VALUE LOOP  mode: $Mode   cells: $CellsLabel")
    $tiers = ''
    if ($s.builder_tiers) { $tiers = (@($s.builder_tiers.PSObject.Properties | ForEach-Object { "$($_.Name)=$($_.Value)" }) -join ' ') }
    $l.Add("scout/default model: $($Loop.models), verifier: xiaomi/mimo-v2.6-pro, builder tier(s) of compiled proposals: $tiers   baseline: $($Loop.baseline.label) ($($Loop.baseline.live) live artifacts)   evaluate-before-announce: $($Loop.evaluate_before_announce)")
    $l.Add("corroborated $($s.corroborated), reasoned $($s.reasoned), proposed $($s.proposed), delivered $($s.delivered), announced $($s.announced), not announced $($s.not_announced), unlinked $($s.unlinked), blocked $($s.blocked), cost `$$($s.cost_usd)")
    $l.Add('')
    foreach ($r in @($Loop.findings)) {
        $idx = -1
        if ($r.finding_id -match '^finding_(\d+)$') { $idx = [int]$Matches[1] - 1 }
        $sig = $null
        if ($Signals -and $idx -ge 0 -and $idx -lt @($Signals).Count) { $sig = @($Signals)[$idx] }
        $cell = $(if ($sig) { Format-Cell $sig.dims } else { '(sensor binary not found: cell not shown)' })
        $l.Add("$($r.finding_id)  $(Get-MetricLabel ([string]$r.metric))")
        $l.Add("  cell    : $cell")
        $l.Add("  effect  : $(Format-Effect $sig)")
        $l.Add("  status  : $($r.status) / $($r.reason)")
        $oc = Get-FindingOutcome $r
        $id = ''
        if ($r.delivery -and $r.delivery.proposal_id) { $id = "  proposal $($r.delivery.proposal_id)" }
        $l.Add("  outcome : $oc$id")
        if ($r.target_ref) { $l.Add("  slug    : $(Get-ProposalSlug $r)   ($($r.proposal_kind) of $($r.target_ref))") }
        if ($r.evaluation) { $l.Add("  proof   : $(Get-ProofLine $r)") }
        if ($r.platform_announce) { $l.Add("  platform: $($r.platform_announce)") }
        $l.Add("  cost    : $(Format-Cost $r)")
    }
    $l.ToArray()
}

# The dossier ES of an announced record (what supervisors read), or a line saying there is none.
function Get-DossierEs {
    param($Record)
    if ($Record.evaluation -and $Record.evaluation.dossier -and $Record.evaluation.dossier.es) { return $Record.evaluation.dossier.es }
    $null
}

function Format-DossierView {
    param($Record, $Registry = $null)
    $l = New-Object System.Collections.Generic.List[string]
    $es = Get-DossierEs $Record
    $propId = ''
    if ($Record.delivery -and $Record.delivery.proposal_id) { $propId = [string]$Record.delivery.proposal_id }
    $l.Add("=== $($Record.finding_id)  $(Get-ProposalSlug $Record)  proposal $propId")
    if (-not $es) { $l.Add('(no dossier on this record)'); return $l.ToArray() }
    $l.Add("TITLE: $($es.title)")
    foreach ($line in ([string]$es.description -split "`r?`n")) { $l.Add($line) }
    if ($Registry) {
        $l.Add('')
        $l.Add("REGISTRY (read back from agent-core): state=$($Registry.state) origin=$($Registry.origin) agent=$($Registry.agent) rev=$($Registry.rev) changes=$($Registry.changes) created_by=$($Registry.created_by)")
        $l.Add('  never approved, published or promoted by the engine; a person decides in the platform')
    }
    $l.ToArray()
}

# ---- platform announce (same bounds as registry_writer::announce) ----------------------------------------------------------------------

$script:Crockford = '0123456789ABCDEFGHJKMNPQRSTVWXYZ'

# CASE-<26 Crockford chars> derived from the evidence ref: opaque, resolves to no real case.
function Get-OpaqueLink {
    param([Parameter(Mandatory)][string]$EvidenceRef, [int]$Index = 0)
    $sha = [Security.Cryptography.SHA256]::Create()
    $bytes = $sha.ComputeHash([Text.Encoding]::UTF8.GetBytes("ann1|$EvidenceRef|$Index"))
    $hex = (($bytes | ForEach-Object { $_.ToString('x2') }) -join '')
    $body = New-Object Text.StringBuilder
    for ($i = 0; $i -lt 26; $i++) {
        $b = [Convert]::ToInt32($hex.Substring($i * 2, 2), 16) -band 31
        [void]$body.Append($script:Crockford[$b])
    }
    'CASE-' + $body.ToString()
}

function Test-PersonalShape {
    param([string]$Text)
    if ($Text -match '[^\s@]+@[^\s@]*\.[A-Za-z]{2}') { return $true }
    if ($Text -match '(\d[ \-.]?){9,}') { return $true }
    $false
}

function Limit-Text {
    param([string]$Text, [int]$Max)
    $t = $Text.Trim()
    if ($t.Length -le $Max) { return $t }
    $cut = $t.Substring(0, $Max - 1)
    $sp = $cut.LastIndexOf(' ')
    if ($sp -gt ($Max / 2)) { $cut = $cut.Substring(0, $sp) }
    $cut.TrimEnd() + [string][char]0x2026
}

# Body of POST /api/v1/internal/builder/proposals/announce, or throws with the field named. Texts are cut to the platform bounds.
function New-AnnouncePayload {
    param([Parameter(Mandatory)]$Record)
    $es = Get-DossierEs $Record
    if (-not $es) { throw 'dossier: this record has no Spanish dossier' }
    if ($Record.outcome -ne 'announced') { throw 'dossier: not an announced record' }
    $propId = [string]$Record.delivery.proposal_id
    if (-not $propId -or $propId.Length -gt 64 -or $propId -notmatch '^[A-Za-z0-9_.@-]+$') { throw 'proposalId: 1-64 id characters required' }
    $field = {
        param($name, $raw, $max)
        $t = Limit-Text -Text ([string]$raw) -Max $max
        if (-not $t) { throw "${name}: empty" }
        if (Test-PersonalShape $t) { throw "${name}: personal-data shape (email or 9+ digit run)" }
        $t
    }
    [ordered]@{
        proposalId = $propId
        title = (& $field 'title' $es.title 120)
        problem = (& $field 'problem' $es.sections.problem 600)
        evidence = (& $field 'evidence' $es.sections.evidence 600)
        expectedEffect = (& $field 'expectedEffect' $es.sections.expected_effect 400)
        evidenceLinks = @(Get-OpaqueLink -EvidenceRef ([string]$Record.evidence_ref) -Index 0)
    }
}

# The records of the last run that are announced (outcome === announced).
function Get-AnnouncedRecords { param($Loop) @(@($Loop.findings) | Where-Object { $_.outcome -eq 'announced' }) }

# Probe findings, from schedule_probes.py --report-out. Lines for the -Probes step.
function Format-ProbeReport {
    param($Report)
    $l = New-Object System.Collections.Generic.List[string]
    $l.Add("PROBES  action: $($Report.action)  run_index: $($Report.run_index)  confirmed scenarios: $(@($Report.confirmed_scenarios).Count)  flaky: $(@($Report.flaky).Count)")
    $sigs = @($Report.signals)
    if ($sigs.Count -eq 0) { $l.Add('  no probe finding (all scenario families above their floors)') }
    foreach ($s in $sigs) {
        $d = $s.dims
        $cell = $(if ($d) { Format-Cell $d } else { '' })
        $l.Add("  $($s.metric)  $cell  status=$($s.status)  reason=$($s.reason)  evidence_class=probe_synthetic")
    }
    $l.Add('  (probe cells are synthetic scenarios, never customers; a first run is always candidate, a second run can corroborate)')
    $l.ToArray()
}

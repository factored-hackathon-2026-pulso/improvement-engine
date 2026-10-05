<#
.SYNOPSIS
  Integrated rig, STAGE 1 story at API level: detect -> propose -> prove -> announce -> platform lists it -> supervisor reads it -> eval suite attached
  and evaluated. It NEVER approves, publishes or promotes (those hops are reported as not run).
  scripts/integrated-rig/run_story.ps1 [-Profile planted|bank] [-CellsFile F] [-MaxFindings 1] [-BuilderModel xiaomi/mimo-v2.6-pro]
                                       [-FallbackModel xiaomi/mimo-v2.6-flash] [-TimeoutMin 25] [-NoEval] [-NativeAnnounce]
.DESCRIPTION
  Needs up.ps1 first. Hops (each reported OK / BREAK / not-run, with what a break needs):
    cells      planted (SYNTHETIC invented cells, one planted association) or bank (cached real aggregates via -CellsFile)
    loop       `pulso run` (this script owns its lifecycle) over the cells: Scout/Verifier/Builder through the gateway, regression proof, proposal in agent-core
    evidence   G1: CASE- ids from the platform (GET /internal/evidence/cases with the service token; fallback: seeded open cases)
    announce   POST /api/v1/internal/builder/proposals/announce (rig side, with those ids) and its replay
    verify     story_verify.py: agent-core draft, one notification per seeded supervisor, Automatizacion list (source engine), detail docs
    eval       attach_eval_suite.py (EV1, by script): eval_suite change + validate + freeze + evaluate in agent-core (never approve/publish)
  Credentials: agent-core.env / llm-gateway.env are read here into child environments only; every child line is masked; names, never values, are logged.
  Exit: 0 all executed hops OK, 1 a hop broke, 2 usage, 3 rig not up.
#>
[CmdletBinding()]
param(
    [ValidateSet('planted', 'bank')][string]$Profile = 'planted', [string]$CellsFile = '', [int]$MaxFindings = 1,
    [string]$BuilderModel = 'xiaomi/mimo-v2.6-pro', [string]$FallbackModel = 'xiaomi/mimo-v2.6-flash', [int]$TimeoutMin = 25,
    [switch]$NoEval, [switch]$NativeAnnounce,
    # -Cycle: ONE engine process stays alive from the loop through the release event; after the announce a PERSON approves, publishes and
    # promotes in the platform SPA while this script only WATCHES (read-only); then poller -> engine -> outcome card. No decision automation.
    [switch]$Cycle, [int]$HumanTimeoutMin = 60, [string]$PseudoRelease = '2025-06', [string]$SpaUrl = 'http://127.0.0.1:5174'
)
$ErrorActionPreference = 'Stop'
try { [Console]::OutputEncoding = New-Object Text.UTF8Encoding($false) } catch { }
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$root = (Resolve-Path (Join-Path $here '..\..')).Path
. (Join-Path $here 'rig.lib.ps1')

$factored = Split-Path -Parent (Split-Path -Parent $root)
$settings = Get-RigSettings
$paths = Get-RigPaths -Root $root
$python = (Get-Command python -ErrorAction Stop).Source
$uv = (Get-Command uv -ErrorAction Stop).Source
$acEnvPath = $(if ($env:PULSO_AGENT_CORE_ENV) { $env:PULSO_AGENT_CORE_ENV } else { Join-Path $factored 'agent-core.env' })
$gwEnvPath = $(if ($env:PULSO_LLM_GATEWAY_ENV) { $env:PULSO_LLM_GATEWAY_ENV } else { Join-Path $factored 'llm-gateway.env' })
$acEnv = Read-EnvFileValues -Path $acEnvPath
$gwEnv = Read-EnvFileValues -Path $gwEnvPath
$secretAll = @{}
foreach ($k in $acEnv.Keys) { $secretAll[$k] = $acEnv[$k] }
foreach ($k in $gwEnv.Keys) { $secretAll[$k] = $gwEnv[$k] }
$script:Needles = @(Get-RedactionNeedles -Sources @($secretAll))
function Add-Needles { param([string[]]$Values) $script:Needles = @($script:Needles + @($Values | Where-Object { $_ -and $_.Length -ge 6 })) | Sort-Object { $_.Length } -Descending }
function Say { param([string]$Text = '') Write-Host (Protect-Text -Text $Text -Needles $script:Needles) }

# ---- preconditions ---------------------------------------------------------------------------------------------------------------------
if (-not (Test-Path -LiteralPath $paths.Secrets) -or -not (Test-Path -LiteralPath $paths.Tokens)) { Say 'rig is not up: run scripts/integrated-rig/up.ps1'; exit 3 }
$shell = Get-ChildShell
$h = Invoke-Scrubbed -File $shell -Arguments @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', (Join-Path $here 'health.ps1'), '-Quiet') -Needles $script:Needles -Quiet
if ($h.ExitCode -ne 0) { Say 'rig health check failed: run health.ps1'; exit 3 }
$svc = [string](Read-JsonFile -Path $paths.Secrets).service_token
$tok = Read-JsonFile -Path $paths.Tokens
Add-Needles @($svc, [string]$tok.admin, [string]$tok.builder, [string]$tok.exporter)
$rig = Read-JsonFile -Path (Join-Path $paths.Rig 'rig.json')
$core = "http://127.0.0.1:$($settings.CorePort)"; $plat = "http://127.0.0.1:$($settings.PlatformPort)"; $engineUrl = "http://127.0.0.1:$($settings.EnginePort)"
$runId = Get-Date -Format 'yyyyMMdd-HHmmss'
$work = Join-Path $paths.Rig "work-$runId"; $store = Join-Path $paths.Rig "store-$runId"; $cellsDir = Join-Path $paths.Rig 'cells'
$null = New-Item -ItemType Directory -Force -Path $work, $store, $cellsDir
$hops = New-Object System.Collections.Generic.List[object]
function Hop { param([string]$Name, [string]$Status, [string]$Detail, [string]$Needs = '') $hops.Add([pscustomobject]@{ Hop = $Name; Status = $Status; Detail = $Detail; Needs = $Needs }); Say ("[{0}] {1}: {2}" -f $Status, $Name, $Detail) }
function Stop-Tree { param($Proc) if ($Proc -and -not $Proc.HasExited) { try { & taskkill /PID $Proc.Id /T /F 2>&1 | Out-Null } catch { } } }

Say ("ENV1 stage 1 story  {0}  profile={1} builder={2} (fallback {3})  free RAM {4} MB" -f (Get-Date -Format 'yyyy-MM-dd HH:mm:ss'), $Profile, $BuilderModel, $FallbackModel, (Get-FreeRamMb))

# ---- cells ------------------------------------------------------------------------------------------------------------------------------
$cellsPath = ''
if ($Profile -eq 'planted') {
    $cellsPath = Join-Path $cellsDir 'planted.ndjson'
    $r = Invoke-Scrubbed -File $python -Arguments @((Join-Path $root 'scripts\demo-loop\planted_cells.py'), '--out', $cellsPath) -Needles $script:Needles -WorkDir $root -Quiet
    if ($r.ExitCode -ne 0) { Hop 'cells' 'BREAK' 'planted_cells.py failed'; exit 1 }
    $mode = 'synthetic-planted'; $src = 'synthetic'
} else {
    if (-not $CellsFile -or -not (Test-Path -LiteralPath $CellsFile)) { Hop 'cells' 'BREAK' '-Profile bank needs -CellsFile (cached bank aggregates, never raw data)'; exit 2 }
    $cellsPath = (Resolve-Path -LiteralPath $CellsFile).Path; $mode = 'bank-file'; $src = 'bank'
}
$postPath = ''
if ($Cycle) {
    if ($Profile -ne 'planted') { Hop 'cells' 'BREAK' '-Cycle needs -Profile planted (the post-release table is the planted one with the effect cut)'; exit 2 }
    $postPath = Join-Path $cellsDir 'planted-post.ndjson'
    $r = Invoke-Scrubbed -File $python -Arguments @((Join-Path $here 'post_cells.py'), '--in', $cellsPath, '--out', $postPath, '--release', $PseudoRelease, '--category', 'Cobro indebido', '--cut-pp', '8') -Needles $script:Needles -WorkDir $root -Quiet
    if ($r.ExitCode -ne 0) { Hop 'cells' 'BREAK' 'post_cells.py failed'; exit 1 }
}
$info = Get-CellsInfo -Path $cellsPath
Hop 'cells' 'OK' ("{0}: {1} rows, metrics {2}, sha256:{3}  [{4}]" -f $mode, $info.Rows, $info.ByMetric, $info.Sha, $(if ($Profile -eq 'planted') { 'SYNTHETIC, invented numbers' } else { 'real bank aggregates' }))

# ---- evidence ids (G1) ------------------------------------------------------------------------------------------------------------------
$r = Invoke-Scrubbed -File $python -Arguments @((Join-Path $here 'story_verify.py'), 'cases', '--platform', $plat, '--out', $paths.CaseIds, '--secrets', $paths.Secrets) -Needles $script:Needles -WorkDir $root
$caseIds = @()
if ($r.ExitCode -eq 0 -and (Test-Path -LiteralPath $paths.CaseIds)) { $caseIds = @((Read-JsonFile -Path $paths.CaseIds).ids) }
if ($caseIds.Count -gt 0) { Hop 'evidence' 'OK' ("{0} CASE- ids for evidenceLinks (labelled example cases)" -f $caseIds.Count) } else { Hop 'evidence' 'BREAK' 'no case ids from the platform' 'platform: seeded cases or evidence route' }

# ---- loop: the engine process is owned here ---------------------------------------------------------------------------------------------
function Start-Engine {
    param([string]$Model)
    $gwKey = $(if ($gwEnv['GATEWAY_TOKEN_AGENT_CORE']) { $gwEnv['GATEWAY_TOKEN_AGENT_CORE'] } else { $acEnv['GATEWAY_TOKEN_AGENT_CORE'] })
    $rb = New-Object byte[] 24; $rng = New-Object Security.Cryptography.RNGCryptoServiceProvider; $rng.GetBytes($rb); $rng.Dispose()
    $adm = [Convert]::ToBase64String($rb).Replace('+', 'a').Replace('/', 'b').Replace('=', 'c')
    Add-Needles @($adm, $gwKey)
    $pu = ''; $pt = ''
    if ($NativeAnnounce) { $pu = $plat; $pt = $svc }
    $eenv = Get-EngineEnvironment -Settings $settings -CellsPath $cellsPath -WorkDir $work -StoreDir $store -Source $src -MaxFindings $MaxFindings `
        -GatewayKey $gwKey -RegistryToken ([string]$tok.builder) -AdminToken $adm -PlatformUrl $pu -PlatformToken $pt -PythonCmd $python -BuilderModel $Model
    if ($Cycle) {
        # R1/R2: demo clock. Pseudo release month, the planted POST table (treated cell cut by 8 pp, SYNTHETIC), staging-or-prod release events accepted.
        $eenv['PULSO_OUTCOME_PRE_CELLS'] = $cellsPath; $eenv['PULSO_OUTCOME_POST_CELLS'] = $postPath
        $eenv['PULSO_OUTCOME_PSEUDO_RELEASE'] = $PseudoRelease; $eenv['PULSO_OUTCOME_WINDOW_MONTHS'] = '3'
        $eenv['PULSO_OUTCOME_CMD'] = '"' + $python + '" "' + (Join-Path $root 'scripts\out1\outcome_cli_adapter.py') + '"'
        $eenv['PULSO_OUTCOME_CWD'] = $root; $eenv['PULSO_OUTCOME_DATA_LABEL'] = 'synthetic-planted-effect'; $eenv['PULSO_OUTCOME_TIMEOUT_SECS'] = '180'
    }
    Say ("    engine environment names: " + ((Get-EnvNames -Env $eenv) -join ', '))
    $elog = Join-Path $paths.Rig "engine-$runId.log"
    $psi = New-Object Diagnostics.ProcessStartInfo
    $psi.FileName = $env:ComSpec
    $psi.Arguments = '/c ""' + $rig.pulso_exe + '" run >> "' + $elog + '" 2>&1"'
    $psi.UseShellExecute = $false; $psi.RedirectStandardInput = $true; $psi.RedirectStandardOutput = $true; $psi.RedirectStandardError = $true; $psi.CreateNoWindow = $true; $psi.WorkingDirectory = $root
    foreach ($k in $eenv.Keys) { $psi.EnvironmentVariables[[string]$k] = [string]$eenv[$k] }
    $p = [Diagnostics.Process]::Start($psi)
    $ready = $false
    for ($i = 0; $i -lt 60 -and -not $ready; $i++) { Start-Sleep -Seconds 1; if ($p.HasExited) { break }; $ready = Test-Http "$engineUrl/readyz" }
    [pscustomobject]@{ Proc = $p; Ready = $ready; Admin = $adm; Log = $elog }
}

function Invoke-LoopJob {
    param([string]$Model)
    Remove-Item -LiteralPath (Join-Path $work 'value-loop') -Recurse -Force -ErrorAction SilentlyContinue
    $e = Start-Engine -Model $Model
    try {
        if (-not $e.Ready) { return [pscustomobject]@{ Ok = $false; Why = 'engine did not become ready'; Result = $null } }
        $key = "env1-$runId-" + ($Model -replace '[^a-z0-9]', '')
        $r = Invoke-Scrubbed -File $python -Arguments @((Join-Path $root 'scripts\triggers\agentcore_poller.py'), 'explicit', $key, '--engine-url', $engineUrl, '--state', (Join-Path $work 'trigger-state.json')) `
            -Env @{ PULSO_ENGINE_TOKEN = $e.Admin } -Needles $script:Needles -WorkDir $root -Quiet
        if ($r.ExitCode -ne 0) { return [pscustomobject]@{ Ok = $false; Why = ('trigger refused: ' + ($r.Output -join ' ')); Result = $null } }
        $deadline = (Get-Date).AddMinutes($TimeoutMin); $f = $null; $last = Get-Date
        while ((Get-Date) -lt $deadline) {
            $f = Get-ChildItem -LiteralPath (Join-Path $work 'value-loop') -Filter '*.json' -ErrorAction SilentlyContinue | Select-Object -First 1
            if ($f) { break }
            if ($e.Proc.HasExited) { return [pscustomobject]@{ Ok = $false; Why = "engine exited early (code $($e.Proc.ExitCode)); log $($e.Log)"; Result = $null } }
            if (((Get-Date) - $last).TotalSeconds -ge 60) { Say '    ... loop still working'; $last = Get-Date }
            Start-Sleep -Seconds 3
        }
        if (-not $f) { return [pscustomobject]@{ Ok = $false; Why = "no job result within $TimeoutMin min"; Result = $null } }
        $script:KeepEngine = $(if ($Cycle) { $e } else { $null })
        [pscustomobject]@{ Ok = $true; Why = ''; Result = $f.FullName }
    } finally {
        if (-not ($Cycle -and $script:KeepEngine -and [object]::ReferenceEquals($script:KeepEngine, $e))) {
            try { $e.Proc.StandardInput.Close() } catch { }
            Start-Sleep -Milliseconds 800
            Stop-Tree $e.Proc
        }
    }
}

$loopSw = [Diagnostics.Stopwatch]::StartNew()
$job = Invoke-LoopJob -Model $BuilderModel
$lres = $null; $ann = @()
if ($job.Ok) { $lres = Read-JsonFile -Path $job.Result; $ann = @(Get-AnnouncedRecords -Loop $lres) }
if ((-not $job.Ok -or $ann.Count -eq 0) -and $FallbackModel -and $FallbackModel -ne $BuilderModel) {
    if ($script:KeepEngine) { try { $script:KeepEngine.Proc.StandardInput.Close() } catch { }; Start-Sleep -Milliseconds 800; Stop-Tree $script:KeepEngine.Proc; $script:KeepEngine = $null }
    Say ("    no announced proposal with {0} ({1}); retrying once with the fallback {2}" -f $BuilderModel, $(if ($job.Ok) { 'announced 0' } else { $job.Why }), $FallbackModel)
    $job = Invoke-LoopJob -Model $FallbackModel
    if ($job.Ok) { $lres = Read-JsonFile -Path $job.Result; $ann = @(Get-AnnouncedRecords -Loop $lres) }
}
if (-not $job.Ok) { Hop 'loop' 'BREAK' $job.Why 'engine / agent-core logs under .dev-stack/integrated-rig'; }
else {
    $s = $lres.summary
    Hop 'loop' $(if ($ann.Count -gt 0) { 'OK' } else { 'BREAK' }) ("{0:N0} s; corroborated {1}, proposed {2}, delivered {3}, announced {4}, cost USD {5}; models {6}" -f $loopSw.Elapsed.TotalSeconds, $s.corroborated, $s.proposed, $s.delivered, $s.announced, $s.cost_usd, $lres.models) $(if ($ann.Count -eq 0) { 'a corroborated finding that the Builder can compile and prove; see the finding record' } else { '' })
    foreach ($line in (Format-LoopReport -Loop $lres -Signals $null -Mode $mode -CellsLabel (Split-Path -Leaf $cellsPath))) { Say $line }
}

# ---- announce (rig side, with CASE- ids) -------------------------------------------------------------------------------------------------
$results = @()
foreach ($rec in $ann) {
    $links = @(Select-EvidenceLinks -CaseIds $caseIds -EvidenceRef ([string]$rec.evidence_ref) -Count 2)
    $payload = New-AnnouncePayload -Record $rec -EvidenceLinks $links
    $json = ($payload | ConvertTo-Json -Depth 5) -replace '\\u003c', '<' -replace '\\u003e', '>' -replace '\\u0026', '&' -replace '\\u0027', "'"
    $uri = $plat + '/api/v1/internal/builder/proposals/announce'
    $codes = @()
    foreach ($try in 1, 2) {
        $code = 0
        try {
            $resp = Invoke-WebRequest -Uri $uri -Method Post -UseBasicParsing -TimeoutSec 60 -ContentType 'application/json; charset=utf-8' `
                -Headers @{ Authorization = "Bearer $svc"; 'Idempotency-Key' = "announce:$($payload.proposalId)" } -Body ([Text.Encoding]::UTF8.GetBytes($json))
            $code = [int]$resp.StatusCode
        } catch { try { $code = [int]$_.Exception.Response.StatusCode } catch { $code = 0 } }
        $codes += $code
    }
    $results += [ordered]@{ proposalId = [string]$payload.proposalId; status = $codes[0]; replay_status = $codes[1]; evidenceLinks = $links }
    Say ("    announce {0}: HTTP {1}, replay HTTP {2}, {3} evidence links" -f $payload.proposalId, $codes[0], $codes[1], $links.Count)
}
Write-JsonFile -Path $paths.AnnounceResults -Obj ([ordered]@{ results = $results })
if ($ann.Count -gt 0) {
    $okAnn = (@($results | Where-Object { $_.status -in 200, 201 }).Count -eq $results.Count)
    Hop 'announce' $(if ($okAnn) { 'OK' } else { 'BREAK' }) ("{0} proposal(s); status {1}" -f $results.Count, ((@($results | ForEach-Object { "$($_.status)/$($_.replay_status)" })) -join ' ')) $(if (-not $okAnn) { 'platform: see platform.log (404 token unset, 422 payload, 502 agent-core read of the engine proposal = G6)' } else { '' })
} else { Hop 'announce' 'not-run' 'nothing announced by the loop' }

# ---- verify -------------------------------------------------------------------------------------------------------------------------------
if ($job.Ok -and $ann.Count -gt 0) {
    $r = Invoke-Scrubbed -File $python -Arguments @((Join-Path $here 'story_verify.py'), 'verify', '--platform', $plat, '--core', $core, '--tokens', $paths.Tokens, '--loop-result', $job.Result,
            '--announce', $paths.AnnounceResults, '--out', $paths.Story) -Needles $script:Needles -WorkDir $root
    Hop 'verify' $(if ($r.ExitCode -eq 0) { 'OK' } else { 'BREAK' }) 'see the check table above' $(if ($r.ExitCode -ne 0) { 'the FAIL rows name the owning side' } else { '' })
} else { Hop 'verify' 'not-run' 'no announced proposal' }

# ---- eval suite attach + evaluate (EV1, by script) ----------------------------------------------------------------------------------------------
if (-not $NoEval -and $job.Ok -and $ann.Count -gt 0) {
    $pid2 = [string]$ann[0].delivery.proposal_id
    $agent = ''
    try { $pr = Invoke-RestMethod -Uri "$core/v1/registry/proposals/$pid2" -Headers @{ Authorization = "Bearer $($tok.builder)" } -TimeoutSec 20; $agent = [string]$pr.proposal.agent_id } catch { }
    $suite = Join-Path $root "agent-core-assets\eval-suites\pulso-min\$agent\$agent-min@1.0.0.yaml"
    if ($agent -and (Test-Path -LiteralPath $suite)) {
        $r = Invoke-Scrubbed -File $uv -Arguments @('run', '--project', $rig.agent_core_dir, '--with', 'pyyaml', 'python', (Join-Path $root 'scripts\dev-stack\attach_eval_suite.py'),
                '--suite', $suite, '--base', $core, '--state-dir', $paths.Dev, '--proposal-id', $pid2, '--agent-core', $rig.agent_core_dir) -Needles $script:Needles -WorkDir $root `
            -Env @{ PULSO_AGENT_CORE_DIR = [string]$rig.agent_core_dir }
        Hop 'eval' $(if ($r.ExitCode -eq 0) { 'OK' } else { 'BREAK' }) ("agent {0}, suite pulso-min (R6 stand-in), exit {1}" -f $agent, $r.ExitCode) $(if ($r.ExitCode -ne 0) { 'agent-core evaluate (see output above); a real suite is Codex T2 / G11' } else { '' })
    } else { Hop 'eval' 'BREAK' ("no pulso-min suite for agent '{0}'" -f $agent) 'G11: a real eval suite for this agent (Codex T2)' }
} else { Hop 'eval' 'not-run' 'skipped' }

if ($Cycle -and $script:KeepEngine -and $ann.Count -gt 0) {
    $pidH = [string]$ann[0].delivery.proposal_id
    $eng = $script:KeepEngine
    $spaOk = Test-Http $SpaUrl
    $banner = @(
        '', '================================================================================',
        'ACTION FOR A PERSON (local synthetic stack; the rig only watches, it decides nothing)',
        "  Platform SPA : $SpaUrl   (API $plat)   SPA reachable now: $spaOk",
        '  Account      : Lucia Herrera, lucia.herrera@latambank.example (Supervision), password demo1234',
        '  Second factor: 000000 (the published dev constant of the seeded accounts, CC_ENV=dev only)',
        "  Proposal     : $pidH   (agent consultas, source 'Del motor de mejora')",
        '  Three clicks : Automatizacion > Propuestas > open the proposal;',
        '                 1) Aprobar (enter 000000)   2) Publicar (enter 000000)   3) Pasar a produccion (enter 000000)',
        "  The rig waits up to $HumanTimeoutMin min, then runs: release event -> poller -> engine -> outcome card.",
        '================================================================================', '')
    foreach ($l in $banner) { Say $l }
    Write-JsonFile -Path (Join-Path $paths.Rig 'waiting.json') -Obj ([ordered]@{ spa = $SpaUrl; api = $plat; account = 'lucia.herrera@latambank.example'; step_up = '000000'; proposal_id = $pidH; since = (Get-Date).ToUniversalTime().ToString('o') })
    $r = Invoke-Scrubbed -File $python -Arguments @((Join-Path $here 'story_verify.py'), 'wait-human', '--core', $core, '--tokens', $paths.Tokens, '--proposal-id', $pidH, '--timeout-min', "$HumanTimeoutMin") -Needles $script:Needles -WorkDir $root
    $humanOk = ($r.ExitCode -eq 0)
    Hop 'human: approve + publish + promote prod' $(if ($humanOk) { 'OK' } else { 'BREAK' }) $(if ($humanOk) { 'a person did it in the SPA (observed in agent-core registry events)' } else { "not seen within $HumanTimeoutMin min" }) $(if (-not $humanOk) { 'the person (or a platform/SPA break: see platform.log)' } else { '' })
    if ($humanOk) {
        # release event -> poller -> the SAME engine process (trigger records are in memory)
        $tries = 0; $sent = $false; $txt = ''; $r = $null
        while ($tries -lt 5 -and -not $sent) {
            $tries++
            $r = Invoke-Scrubbed -File $python -Arguments @((Join-Path $root 'scripts\triggers\agentcore_poller.py'), 'poll', '--once', '--core-url', $core, '--engine-url', $engineUrl, '--state', (Join-Path $work 'poller-state.json'),
                    '--token-file', $paths.Tokens, '--token-key', 'exporter', '--only-agent', 'no-run-triggers', '--only-origin', 'auto_detect') -Env @{ PULSO_ENGINE_TOKEN = $eng.Admin } -Needles $script:Needles -WorkDir $root -Quiet
            $txt = $r.Output -join ' '
            if ($r.ExitCode -eq 0 -and $txt -match '[1-9]\d*') { $sent = $true } elseif ($r.ExitCode -ne 0) { break } else { Start-Sleep -Seconds 5 }
        }
        Hop 'release event -> poller -> engine' $(if ($sent) { 'OK' } else { 'BREAK' }) ("poller exit {0}: {1}" -f $r.ExitCode, ($txt.Substring(0, [math]::Min(160, $txt.Length)))) $(if (-not $sent) { 'agent-core: release.* event with proposal_id/origin on /v1/export/registry-events; poller exporter token' } else { '' })
        $card = $null
        if ($sent) {
            $dl = (Get-Date).AddMinutes(6)
            while ((Get-Date) -lt $dl -and -not $card) {
                $cf = Get-ChildItem -LiteralPath (Join-Path $work 'outcome') -Filter '*.json' -ErrorAction SilentlyContinue | Where-Object { $_.Name -notlike '*.cells*' } | Select-Object -First 1
                if ($cf) { $card = Read-JsonFile -Path $cf.FullName } else { Start-Sleep -Seconds 4 }
            }
        }
        if ($card) {
            $okV = (@('improved', 'no_detectable_change', 'worsened', 'inconclusive') -contains [string]$card.verdict)
            Hop 'outcome step' $(if ($okV) { 'OK' } else { 'BREAK' }) ("verdict={0} effect_pp={1} interval_pp={2} period_kind={3} data_label={4} success_claimed={5}" -f $card.verdict, $card.effect_pp, ($card.interval_pp -join '..'), $card.period_kind, $card.data_label, $card.success_claimed) ''
            Write-JsonFile -Path (Join-Path $paths.Rig 'outcome-card.json') -Obj $card -Depth 10
        } else { Hop 'outcome step' 'BREAK' 'no outcome card within 6 min of the release event' 'engine log under .dev-stack/integrated-rig (trigger admitted? proposal_id on the event? estimator)' }
    } else {
        foreach ($n in 'release event -> poller -> engine', 'outcome step') { Hop $n 'not-run' 'waiting for the human hop' '' }
    }
    $eng2 = $script:KeepEngine
} else {
    foreach ($n in 'human approve/publish/promote', 'release event -> poller -> engine', 'outcome step') { Hop $n 'not-run' 'run with -Cycle (a person decides in the SPA; this script never does)' '' }
}

Say ''
Say '--- what is real, stand-in or relaxed'
foreach ($l in (Format-RealityTable -Rows (Get-RigRealityRows))) { Say $l }
Say ''
Say '--- hops'
foreach ($x in $hops) { Say ("  {0,-8} {1}  {2}" -f $x.Status, $x.Hop, $x.Needs) }
Write-JsonFile -Path (Join-Path $paths.Rig "hops-$runId.json") -Obj @($hops)
$bad = @($hops | Where-Object { $_.Status -eq 'BREAK' }).Count
if ($script:KeepEngine) { try { $script:KeepEngine.Proc.StandardInput.Close() } catch { }; Start-Sleep -Milliseconds 800; Stop-Tree $script:KeepEngine.Proc; Say 'engine stopped; the rest of the rig stays up until down.ps1' }
exit $(if ($bad -gt 0) { 1 } else { 0 })

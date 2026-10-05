<#
.SYNOPSIS
  One command that runs the Pulso value loop on a LOCAL stack and prints a readable result.
  scripts/demo-loop/run.ps1 [-Up] [-Cells] [-Loop] [-Probes] [-Show] [-Announce] [-Down] [-All] [-Synthetic]
                            [-MaxFindings 4] [-BuilderModel M] [-CellsFile F] [-TimeoutMin 45] [-ProbeReps 3] [-Purge]
.DESCRIPTION
  Steps run in this fixed order, whichever switches you pass:
    -Up        own local stack (prefix pulso-demo, own ports): postgres, llm-gateway, agent-core with the real demo agents imported.
    -Cells     the treated cell table: the cached bank cells (scripts/aggregate/bank_cells.py output) if present, else run the aggregator.
               -Synthetic uses the labelled SYNTHETIC planted-cell table instead (scripts/demo-loop/planted_cells.py).
    -Loop      starts `pulso run`, posts one explicit trigger, waits for the job and prints per finding: finding, cell, effect, status,
               outcome (announced / not_announced:<verdict> / unlinked + reason), proposal slug, the regression verdict story, cost.
    -Probes    runs the agent battery and the scheduled probes once and prints the probe findings (own battery core, extra containers).
    -Show      prints the dossier ES of every announced proposal and reads the registry state back (state=draft, origin auto_detect).
               Never approves, publishes or promotes anything.
    -Announce  calls the platform announce route when PULSO_PLATFORM_URL and PULSO_PLATFORM_SERVICE_TOKEN (or PULSO_PLATFORM_TOKEN) are set,
               else prints exactly what would be sent.
    -Down      stops this script's own stack (-Purge also drops its volume, image and local state). Other lanes' stacks are never touched.
    -All       = -Up -Cells -Loop -Show.
  -BuilderModel M  Builder tier (default xiaomi/mimo-v2.6-flash; the larger xiaomi/mimo-v2.6-pro writes the patch more reliably).
  Credentials come only from agent-core.env and llm-gateway.env (next to the worktrees folder, or PULSO_AGENT_CORE_ENV / PULSO_LLM_GATEWAY_ENV),
  are loaded into CHILD process environments only, are never printed, and every line a child prints is masked first.
  Exit code: 0 ok, 1 a step failed, 2 usage.
#>
[CmdletBinding()]
param(
    [switch]$Up, [switch]$Cells, [switch]$Loop, [switch]$Probes, [switch]$Show, [switch]$Announce, [switch]$Down, [switch]$All,
    [switch]$Synthetic, [switch]$Purge,
    [int]$MaxFindings = 4, [int]$TimeoutMin = 45, [int]$ProbeReps = 3,
    [string]$CellsFile = '', [string]$BuilderModel = '', [string]$PulsoExe = '', [string]$AgentCoreDir = '', [string]$GatewayDir = '', [string]$DataRoot = '',
    # own stack and state (the Langfuse closure runs a second one beside the demo's): prefix, ports, state folder, gateway built from -GatewayDir
    [string]$StackPrefix = 'pulso-demo', [int]$PgPort = 55490, [int]$GwPort = 8190, [int]$CorePort = 8191, [int]$EnginePort = 4190,
    [string]$StateName = 'demo-loop', [switch]$FreshGateway, [ValidateRange(1, 2)][int]$PlantedCount = 1
)
$ErrorActionPreference = 'Stop'
try { [Console]::OutputEncoding = New-Object Text.UTF8Encoding($false) } catch { }
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$root = (Resolve-Path (Join-Path $here '..\..')).Path
. (Join-Path $here 'run.lib.ps1')

try {
    $plan = Get-DemoLoopPlan -Up:$Up -Cells:$Cells -Loop:$Loop -Show:$Show -Down:$Down -Probes:$Probes -Announce:$Announce -All:$All `
        -Synthetic:$Synthetic -MaxFindings $MaxFindings -TimeoutMin $TimeoutMin -ProbeReps $ProbeReps -CellsFile $CellsFile
} catch {
    [Console]::Error.WriteLine("usage error: $($_.Exception.Message)")
    exit 2
}

$factored = Split-Path -Parent (Split-Path -Parent $root)
$settings = Get-DemoStackSettings -Prefix $StackPrefix -PgPort $PgPort -GwPort $GwPort -CorePort $CorePort -EnginePort $EnginePort
$demoDir = Join-Path $root (Join-Path '.dev-stack' $StateName)
$null = New-Item -ItemType Directory -Force -Path $demoDir
$statePath = Join-Path $demoDir 'state.json'
$logPath = Join-Path $demoDir ("run-{0}.log" -f (Get-Date -Format 'yyyyMMdd-HHmmss'))
$python = (Get-Command python -ErrorAction Stop).Source
if (-not $AgentCoreDir) { $AgentCoreDir = $(if ($env:PULSO_AGENT_CORE_DIR) { $env:PULSO_AGENT_CORE_DIR } else { Join-Path $factored 'worktrees\agent-core-claude-w15' }) }
if (-not $GatewayDir) { $GatewayDir = $(if ($env:PULSO_LLM_GATEWAY_DIR) { $env:PULSO_LLM_GATEWAY_DIR } else { Join-Path $factored 'worktrees\llm-gateway-claude-otel' }) }
if (-not $DataRoot) { $DataRoot = $(if ($env:PULSO_DEMO_DATA_ROOT) { $env:PULSO_DEMO_DATA_ROOT } else { Join-Path $factored 'data' }) }
$acEnvPath = $(if ($env:PULSO_AGENT_CORE_ENV) { $env:PULSO_AGENT_CORE_ENV } else { Join-Path $factored 'agent-core.env' })
$gwEnvPath = $(if ($env:PULSO_LLM_GATEWAY_ENV) { $env:PULSO_LLM_GATEWAY_ENV } else { Join-Path $factored 'llm-gateway.env' })
$podmanConn = $(if ($env:PULSO_PODMAN_CONNECTION) { $env:PULSO_PODMAN_CONNECTION } else { 'pulso-dev-root' })

# secrets: in memory only; the needles mask them in anything a child prints
$acEnv = Read-EnvFileValues -Path $acEnvPath
$gwEnv = Read-EnvFileValues -Path $gwEnvPath
$secretEnv = @{}
foreach ($k in $acEnv.Keys) { $secretEnv[$k] = $acEnv[$k] }
foreach ($k in $gwEnv.Keys) { $secretEnv[$k] = $gwEnv[$k] }
$script:Needles = @(Get-RedactionNeedles -Sources @($secretEnv))
$script:Tokens = $null

function Say {
    param([string]$Text = '')
    $safe = Protect-Text -Text $Text -Needles $script:Needles
    Write-Host $safe
    try { [IO.File]::AppendAllText($logPath, $safe + "`n") } catch { }
}
function Add-Needles { param([string[]]$Values) $script:Needles = @($script:Needles + @($Values | Where-Object { $_ -and $_.Length -ge 6 })) | Sort-Object { $_.Length } -Descending }
function Read-State { if (Test-Path -LiteralPath $statePath) { return (Read-JsonFile -Path $statePath) }; [pscustomobject]@{} }
function Save-State { param($Obj) [IO.File]::WriteAllText($statePath, ($Obj | ConvertTo-Json -Depth 6), (New-Object Text.UTF8Encoding($false))) }
function Set-StateField {
    param([string]$Name, $Value)
    $s = Read-State
    if ($s.PSObject.Properties[$Name]) { $s.$Name = $Value } else { $s | Add-Member -NotePropertyName $Name -NotePropertyValue $Value }
    Save-State $s
}
function Step-Header { param([string]$Name, [string]$Text) Say ''; Say ("=== [{0}] {1}" -f $Name, $Text) }
function Get-Tokens {
    if ($script:Tokens) { return $script:Tokens }
    $p = Join-Path $root '.dev-stack\tokens.json'
    if (-not (Test-Path -LiteralPath $p)) { throw "no .dev-stack/tokens.json: run -Up first" }
    $script:Tokens = Read-JsonFile -Path $p
    Add-Needles @([string]$script:Tokens.admin, [string]$script:Tokens.builder)
    $script:Tokens
}
function Test-Http {
    param([string]$Url)
    try { $r = Invoke-WebRequest -Uri $Url -UseBasicParsing -TimeoutSec 4; return ($r.StatusCode -eq 200) } catch { return $false }
}
function Invoke-Podman {
    param([string[]]$PodmanArgs)
    $r = Invoke-Scrubbed -File 'podman' -Arguments (@('--connection', $podmanConn) + $PodmanArgs) -Needles $script:Needles -Quiet
    $r
}
function Initialize-GatewayImage {
    # stack.py builds the gateway image on the first `up` (heavy). Another lane's image of the same sources is re-tagged instead.
    param([string]$ImageName)
    if ($FreshGateway) { Say "gateway image: built from $GatewayDir (not re-tagged from another lane)"; return }
    $have = Invoke-Podman -PodmanArgs @('image', 'exists', $ImageName)
    if ($have.ExitCode -eq 0) { return }
    foreach ($src in @('pulso-w15-llm-gateway', 'pulso-bld1-llm-gateway', 'pulso-l3-llm-gateway')) {
        if ((Invoke-Podman -PodmanArgs @('image', 'exists', $src)).ExitCode -eq 0) {
            [void](Invoke-Podman -PodmanArgs @('tag', $src, $ImageName))
            Say "gateway image: re-tagged $src as $ImageName (no rebuild)"
            return
        }
    }
    Say 'gateway image: none to re-tag, stack.py will build it (heavy, one-off)'
}
function Find-PulsoExe {
    $cands = @()
    if ($PulsoExe) { $cands += $PulsoExe }
    if ($env:PULSO_EXE) { $cands += $env:PULSO_EXE }
    if ($env:CARGO_TARGET_DIR) { $cands += (Join-Path $env:CARGO_TARGET_DIR 'debug\pulso.exe') }
    foreach ($lane in 'claude-lfc', 'claude-demo1', 'claude-ann1') { $cands += (Join-Path "D:\cargo-targets\$lane" 'debug\pulso.exe') }
    foreach ($c in $cands) { if ($c -and (Test-Path -LiteralPath $c)) { return (Resolve-Path -LiteralPath $c).Path } }
    throw "pulso.exe not found (tried: $($cands -join '; ')). Build it once: cd seams; cargo build -j 1 -p pulso   (or pass -PulsoExe)"
}
function Find-StepsCli {
    $c = @()
    if ($env:PULSO_STEPS_CLI) { $c += $env:PULSO_STEPS_CLI }
    foreach ($lane in 'claude-lfc', 'claude-demo1', 'claude-ann1') { $c += (Join-Path "D:\cargo-targets\$lane" 'debug\steps_cli.exe') }
    foreach ($x in $c) { if ($x -and (Test-Path -LiteralPath $x)) { return $x } }
    $null
}
function Get-CellsInfo {
    param([string]$Path)
    $n = 0; $by = @{}
    foreach ($line in [IO.File]::ReadLines($Path)) {
        if (-not $line.Trim()) { continue }
        $n++
        if ($line -match '"metric":"([A-Z0-9]+)"') { $by[$Matches[1]] = 1 + [int]$by[$Matches[1]] }
    }
    $sha = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.Substring(0, 12).ToLower()
    [pscustomobject]@{ Rows = $n; ByMetric = (($by.Keys | Sort-Object | ForEach-Object { "$_=$($by[$_])" }) -join ' '); Sha = $sha }
}
function Stop-Tree {
    param($Proc)
    if ($Proc -and -not $Proc.HasExited) { try { & taskkill /PID $Proc.Id /T /F 2>&1 | Out-Null } catch { } }
}

$results = New-Object System.Collections.Generic.List[string]
$failed = $false
$totalWatch = [Diagnostics.Stopwatch]::StartNew()
Say ("Pulso demo loop  {0}  steps: {1}  mode: {2}" -f (Get-Date -Format 'yyyy-MM-dd HH:mm:ss'), ($plan.Steps -join ' '), $plan.Mode)
Say "log: $logPath"

foreach ($step in $plan.Steps) {
    $sw = [Diagnostics.Stopwatch]::StartNew()
    $engine = $null
    try {
        switch ($step) {
            'Up' {
                Step-Header 'Up' "own stack '$($settings.Prefix)': postgres :$($settings.PgPort), llm-gateway :$($settings.GwPort), agent-core :$($settings.CorePort)"
                if (-not (Test-Path -LiteralPath (Join-Path $AgentCoreDir 'pyproject.toml'))) { throw "agent-core checkout not found at $AgentCoreDir (-AgentCoreDir)" }
                if (-not (Test-Path -LiteralPath (Join-Path $GatewayDir 'Dockerfile'))) { throw "llm-gateway checkout not found at $GatewayDir (-GatewayDir)" }
                if (-not $acEnv.Count) { throw "no values read from $acEnvPath (names only are ever shown)" }
                Initialize-GatewayImage -ImageName "$($settings.Prefix)-llm-gateway"
                $e = Get-StackEnvironment -Settings $settings -AgentCoreDir $AgentCoreDir -GatewayDir $GatewayDir
                $envAll = @{}; foreach ($k in $secretEnv.Keys) { $envAll[$k] = $secretEnv[$k] }; foreach ($k in $e.Keys) { $envAll[$k] = $e[$k] }
                # tracing settings the CALLER put in the environment win over the env files (agent-core.env carries a Phoenix endpoint)
                foreach ($it in (Get-ChildItem Env:)) { if ($it.Name -match '^(OTEL_|LLM_GATEWAY_TRACE_|AGENTCORE_TRACE_|PULSO_GW_|PULSO_CORE_OTEL)') { $envAll[$it.Name] = $it.Value } }
                $upArgs = @((Join-Path $root 'scripts\dev-stack\stack.py'), 'up'); if ($FreshGateway) { $upArgs += '--rebuild-gateway' }
                $r = Invoke-Scrubbed -File $python -Arguments $upArgs -Env $envAll -Needles $script:Needles -WorkDir $root -LogPath $logPath
                if ($r.ExitCode -ne 0) { throw "stack.py up failed (exit $($r.ExitCode))" }
                $null = Get-Tokens
                Say ("stack up. registry http://127.0.0.1:{0}  gateway http://127.0.0.1:{1}  (agents: disputas, consultas, real registry-e2e artifacts)" -f $settings.CorePort, $settings.GwPort)
            }
            'Cells' {
                Step-Header 'Cells' $(if ($plan.Synthetic) { 'SYNTHETIC planted-cell table (invented numbers, labelled synthetic)' } else { 'treated bank cells (aggregates only, k >= 10)' })
                $cellsDir = Join-Path $demoDir 'cells'
                $null = New-Item -ItemType Directory -Force -Path $cellsDir
                if ($plan.Synthetic) {
                    $out = Join-Path $cellsDir 'planted.ndjson'
                    $r = Invoke-Scrubbed -File $python -Arguments @((Join-Path $here 'planted_cells.py'), '--out', $out, '--plant', "$PlantedCount") -Needles $script:Needles -WorkDir $root
                    if ($r.ExitCode -ne 0) { throw 'planted_cells.py failed' }
                    $mode = 'synthetic-planted'; $path = $out
                } elseif ($plan.CellsFile) {
                    if (-not (Test-Path -LiteralPath $plan.CellsFile)) { throw "-CellsFile $($plan.CellsFile) not found" }
                    $mode = 'bank-file'; $path = (Resolve-Path -LiteralPath $plan.CellsFile).Path
                } else {
                    $path = Join-Path $cellsDir 'bank_cells.ndjson'
                    if (Test-Path -LiteralPath $path) {
                        $mode = 'bank-cached'
                        Say "using the cached treated cells (delete $path to re-aggregate)"
                    } else {
                        $mode = 'bank-aggregated'
                        if (-not (Test-Path -LiteralPath $DataRoot)) { throw "bank data root not found at $DataRoot (-DataRoot) and no cached cells at $path" }
                        Say "no cache: running scripts/aggregate/bank_cells.py over $DataRoot (aggregates only leave it)"
                        $r = Invoke-Scrubbed -File $python -Arguments @((Join-Path $root 'scripts\aggregate\bank_cells.py'), '--data-root', $DataRoot, '--out', $path) -Needles $script:Needles -WorkDir $root -Quiet -LogPath $logPath
                        if ($r.ExitCode -ne 0 -or -not (Test-Path -LiteralPath $path)) { throw 'bank_cells.py failed' }
                    }
                }
                $info = Get-CellsInfo -Path $path
                Say ("cells: {0}  mode={1}  rows={2}  metrics: {3}  sha256:{4}" -f $path, $mode, $info.Rows, $info.ByMetric, $info.Sha)
                Set-StateField 'cells_path' $path; Set-StateField 'cells_mode' $mode
            }
            'Loop' {
                Step-Header 'Loop' "pulso run over the cells (trigger -> job -> sensor -> roles -> proof -> registry), cap $($plan.MaxFindings) findings"
                $st = Read-State
                $cellsPath = $null; $mode = $plan.Mode
                if ($st.PSObject.Properties['cells_path']) { $cellsPath = [string]$st.cells_path; $mode = [string]$st.cells_mode }
                if ($plan.Synthetic) { $cellsPath = Join-Path $demoDir 'cells\planted.ndjson'; $mode = 'synthetic-planted' }
                elseif ($plan.CellsFile) { $cellsPath = (Resolve-Path -LiteralPath $plan.CellsFile).Path; $mode = 'bank-file' }
                if (-not $cellsPath -or -not (Test-Path -LiteralPath $cellsPath)) { throw 'no cells table: add -Cells (or -Synthetic -Cells)' }
                $tok = Get-Tokens
                foreach ($u in @("http://127.0.0.1:$($settings.CorePort)/healthz", "http://127.0.0.1:$($settings.GwPort)/healthz")) {
                    if (-not (Test-Http $u)) { throw "the stack is not answering at $u (run -Up)" }
                }
                $gwKey = $(if ($gwEnv['GATEWAY_TOKEN_AGENT_CORE']) { $gwEnv['GATEWAY_TOKEN_AGENT_CORE'] } else { $acEnv['GATEWAY_TOKEN_AGENT_CORE'] })
                $rb = New-Object byte[] 24; $rng = New-Object Security.Cryptography.RNGCryptoServiceProvider; $rng.GetBytes($rb); $rng.Dispose()
                $adminTok = [Convert]::ToBase64String($rb).Replace('+', 'a').Replace('/', 'b').Replace('=', 'c')
                Add-Needles @($adminTok)
                $platUrl = ''; $platTok = ''
                if ($Announce) {
                    $platUrl = [string]$env:PULSO_PLATFORM_URL
                    $platTok = $(if ($env:PULSO_PLATFORM_SERVICE_TOKEN) { $env:PULSO_PLATFORM_SERVICE_TOKEN } else { [string]$env:PULSO_PLATFORM_TOKEN })
                    Add-Needles @($platTok)
                }
                $runId = Get-Date -Format 'yyyyMMdd-HHmmss'
                $work = Join-Path $demoDir "work-$runId"; $store = Join-Path $demoDir "store-$runId"
                $null = New-Item -ItemType Directory -Force -Path $work, $store
                $src = $(if ($mode -eq 'synthetic-planted') { 'synthetic' } else { 'bank' })
                $eenv = Get-EngineEnvironment -Settings $settings -CellsPath $cellsPath -WorkDir $work -StoreDir $store -Source $src -MaxFindings $plan.MaxFindings `
                    -GatewayKey $gwKey -RegistryToken ([string]$tok.builder) -AdminToken $adminTok -PlatformUrl $platUrl -PlatformToken $platTok -PythonCmd $python -BuilderModel $BuilderModel
                $exe = Find-PulsoExe
                Say "engine: $exe"
                $elog = Join-Path $demoDir "engine-$runId.log"
                $psi = New-Object Diagnostics.ProcessStartInfo
                $psi.FileName = $env:ComSpec
                $psi.Arguments = '/c ""' + $exe + '" run >> "' + $elog + '" 2>&1"'
                $psi.UseShellExecute = $false; $psi.RedirectStandardInput = $true; $psi.CreateNoWindow = $true; $psi.WorkingDirectory = $root
                foreach ($k in $eenv.Keys) { $psi.EnvironmentVariables[[string]$k] = [string]$eenv[$k] }
                $engine = [Diagnostics.Process]::Start($psi)
                $base = "http://127.0.0.1:$($settings.EnginePort)"
                $ready = $false
                for ($i = 0; $i -lt 60 -and -not $ready; $i++) {
                    Start-Sleep -Seconds 1
                    if ($engine.HasExited) { break }
                    $ready = Test-Http "$base/readyz"
                }
                if (-not $ready) {
                    $tail = ''; if (Test-Path $elog) { $tail = (Get-Content -LiteralPath $elog -Tail 8) -join "`n" }
                    throw ("engine did not become ready (exit {0}). Its log tail:`n{1}" -f $(if ($engine.HasExited) { $engine.ExitCode } else { 'running' }), (Protect-Text $tail $script:Needles))
                }
                Say "engine ready on $base (loopback, ephemeral admin token never printed)"
                $key = "demo-loop-$runId"
                $r = Invoke-Scrubbed -File $python -Arguments @((Join-Path $root 'scripts\triggers\agentcore_poller.py'), 'explicit', $key, '--engine-url', $base, '--state', (Join-Path $work 'trigger-state.json')) `
                    -Env @{ PULSO_ENGINE_TOKEN = $adminTok } -Needles $script:Needles -WorkDir $root -Quiet
                if ($r.ExitCode -ne 0) { throw ("trigger refused: " + ($r.Output -join ' ')) }
                Say "trigger posted (explicit, key $key); waiting for the job (timeout $($plan.TimeoutMin) min)"
                $resultFile = $null
                $deadline = (Get-Date).AddMinutes($plan.TimeoutMin)
                $lastNote = Get-Date
                while ((Get-Date) -lt $deadline) {
                    $f = Get-ChildItem -LiteralPath (Join-Path $work 'value-loop') -Filter '*.json' -ErrorAction SilentlyContinue | Select-Object -First 1
                    if ($f) { $resultFile = $f.FullName; break }
                    if ($engine.HasExited) { throw "engine exited early (code $($engine.ExitCode)); see $elog" }
                    if (((Get-Date) - $lastNote).TotalSeconds -ge 60) { Say ("  ... still working, {0:N0} s" -f $sw.Elapsed.TotalSeconds); $lastNote = Get-Date }
                    Start-Sleep -Seconds 3
                }
                if (-not $resultFile) { throw "no job result within $($plan.TimeoutMin) min (engine log: $elog)" }
                $lres = Read-JsonFile -Path $resultFile
                # the engine is stopped when this step ends: keep its debug events and model calls (content) next to the result
                $jobId = [IO.Path]::GetFileNameWithoutExtension($resultFile)
                $loopRun = "value-loop-$jobId"
                try {
                    $snap = Join-Path $work 'snapshot'
                    $null = New-Item -ItemType Directory -Force -Path $snap
                    $evs = @(); $after = 0
                    while ($true) {
                        $pg = Invoke-RestMethod -Uri "$base/internal/v1/debug/runs/$loopRun/events?after_sequence=$after" -TimeoutSec 20
                        $its = @($pg.items)
                        if ($its.Count -eq 0) { break }
                        $evs += $its
                        $after = ($its | ForEach-Object { [int]$_.sequence } | Measure-Object -Maximum).Maximum
                    }
                    $cl = Invoke-RestMethod -Uri "$base/internal/v1/debug/runs/$loopRun/model-calls" -TimeoutSec 20
                    [IO.File]::WriteAllText((Join-Path $snap 'events.json'), (ConvertTo-Json -InputObject @($evs) -Depth 30), (New-Object Text.UTF8Encoding($false)))
                    [IO.File]::WriteAllText((Join-Path $snap 'model-calls.json'), (ConvertTo-Json -InputObject @($cl.items) -Depth 30), (New-Object Text.UTF8Encoding($false)))
                    Set-StateField 'events_path' (Join-Path $snap 'events.json'); Set-StateField 'calls_path' (Join-Path $snap 'model-calls.json'); Set-StateField 'run_id' $loopRun
                    Say ("engine snapshot: {0} events, {1} model calls (run {2})" -f $evs.Count, @($cl.items).Count, $loopRun)
                } catch { Say ("(engine snapshot failed: {0})" -f (Protect-Text $_.Exception.Message $script:Needles)) }
                $signals = $null
                $cli = Find-StepsCli
                if ($cli) {
                    $sensorOut = Join-Path $work 'sensor.json'
                    & $env:ComSpec /c ('""' + $cli + '" cells < "' + $cellsPath + '" > "' + $sensorOut + '" 2>nul"')
                    if (Test-Path -LiteralPath $sensorOut) {
                        $sj = Read-JsonFile -Path $sensorOut
                        $nc = @($sj.signals | Where-Object { $_.status -eq 'corroborated' -and $_.type -ne 'level_risk' }).Count
                        if ($nc -eq [int]$lres.summary.corroborated) { $signals = @($sj.signals) } else { Say "(sensor preview shows $nc corroborated, the engine $($lres.summary.corroborated): cells not labelled)" }
                    }
                }
                Say ''
                foreach ($line in (Format-LoopReport -Loop $lres -Signals $signals -Mode $mode -CellsLabel (Split-Path -Leaf $cellsPath))) { Say $line }
                Set-StateField 'result_path' $resultFile; Set-StateField 'result_mode' $mode; Set-StateField 'result_seconds' ([math]::Round($sw.Elapsed.TotalSeconds, 1))
                if ($signals) { Set-StateField 'sensor_path' (Join-Path $work 'sensor.json') }
            }
            'Probes' {
                Step-Header 'Probes' 'agent battery + scheduled probes, once (own battery core; synthetic scenarios, never customers)'
                $benv = [ordered]@{ PULSO_STACK_PREFIX = $settings.BatteryPrefix; PULSO_STACK_PORT_CORE = "$($settings.BatteryCore)"; PULSO_STACK_PORT_PG = "$($settings.BatteryPg)"
                    PULSO_STACK_PORT_GW = "$($settings.BatteryGw)"; PULSO_AGENT_CORE_DIR = $AgentCoreDir; PULSO_LLM_GATEWAY_DIR = $GatewayDir }
                $envAll = @{}; foreach ($k in $secretEnv.Keys) { $envAll[$k] = $secretEnv[$k] }; foreach ($k in $benv.Keys) { $envAll[$k] = $benv[$k] }
                Initialize-GatewayImage -ImageName "$($settings.BatteryPrefix)-llm-gateway"
                $r = Invoke-Scrubbed -File $python -Arguments @((Join-Path $root 'scripts\battery\demo_core.py'), 'up') -Env $envAll -Needles $script:Needles -WorkDir $root -LogPath $logPath
                if ($r.ExitCode -ne 0) { throw 'battery core did not start' }
                $rep = Join-Path $demoDir 'probes-report.json'
                $r = Invoke-Scrubbed -File $python -Arguments @((Join-Path $root 'scripts\battery\schedule_probes.py'), '--run', '--base-url', "http://127.0.0.1:$($settings.BatteryCore)",
                        '--reps', "$($plan.ProbeReps)", '--state', (Join-Path $demoDir 'probes-state.json'), '--out-jsonl', (Join-Path $demoDir 'probe-trigger.jsonl'),
                        '--report-out', $rep, '--cells-out', (Join-Path $demoDir 'probe-cells.ndjson'), '--once') -Env $envAll -Needles $script:Needles -WorkDir $root -LogPath $logPath
                if ($r.ExitCode -ne 0 -or -not (Test-Path -LiteralPath $rep)) { throw 'schedule_probes.py failed' }
                $report = Read-JsonFile -Path $rep
                foreach ($line in (Format-ProbeReport -Report $report)) { Say $line }
            }
            'Show' {
                Step-Header 'Show' 'dossier ES of every announced proposal + registry state read back (nothing is approved)'
                $st = Read-State
                if (-not $st.PSObject.Properties['result_path'] -or -not (Test-Path -LiteralPath $st.result_path)) { throw 'no loop result yet: run -Loop first' }
                $lres = Read-JsonFile -Path $st.result_path
                $ann = @(Get-AnnouncedRecords -Loop $lres)
                if ($ann.Count -eq 0) { Say ("no announced proposal in the last run (mode: {0}). Unlinked or not-announced findings stay internal." -f $st.result_mode) }
                $tok = Get-Tokens
                foreach ($rec in $ann) {
                    $reg = $null
                    $pid2 = [string]$rec.delivery.proposal_id
                    try {
                        $reg = Invoke-RestMethod -Uri ("http://127.0.0.1:{0}/v1/registry/proposals/{1}" -f $settings.CorePort, $pid2) -Headers @{ Authorization = "Bearer $($tok.builder)" } -TimeoutSec 20
                    } catch { Say ("(registry read-back failed: {0})" -f (Protect-Text $_.Exception.Message $script:Needles)) }
                    $view = $null
                    if ($reg -and $reg.proposal) { $view = [pscustomobject]@{ state = $reg.proposal.state; origin = $reg.proposal.origin; agent = $reg.proposal.agent_id; rev = $reg.proposal.rev; created_by = $reg.proposal.created_by; changes = @($reg.changes).Count } }
                    foreach ($line in (Format-DossierView -Record $rec -Registry $view)) { Say $line }
                    Say ''
                }
                Say 'The engine only proposes. Approval, publication and promotion stay with the supervisors of the platform.'
            }
            'Announce' {
                Step-Header 'Announce' 'support platform: one notification per announced proposal'
                $st = Read-State
                if (-not $st.PSObject.Properties['result_path'] -or -not (Test-Path -LiteralPath $st.result_path)) { throw 'no loop result yet: run -Loop first' }
                $lres = Read-JsonFile -Path $st.result_path
                $ann = @(Get-AnnouncedRecords -Loop $lres)
                $url = [string]$env:PULSO_PLATFORM_URL
                $tokP = $(if ($env:PULSO_PLATFORM_SERVICE_TOKEN) { $env:PULSO_PLATFORM_SERVICE_TOKEN } else { [string]$env:PULSO_PLATFORM_TOKEN })
                Add-Needles @($tokP)
                if ($ann.Count -eq 0) { Say 'no announced proposal in the last run: nothing to announce.' }
                foreach ($rec in $ann) {
                    $payload = New-AnnouncePayload -Record $rec
                    $json = ($payload | ConvertTo-Json -Depth 5) -replace '\\u003c', '<' -replace '\\u003e', '>' -replace '\\u0026', '&' -replace '\\u0027', "'"
                    if ($rec.platform_announce) { Say ("engine already told the platform during the loop: {0}" -f $rec.platform_announce); continue }
                    if ($url -and $tokP) {
                        $uri = $url.TrimEnd('/') + '/api/v1/internal/builder/proposals/announce'
                        try {
                            $resp = Invoke-WebRequest -Uri $uri -Method Post -UseBasicParsing -TimeoutSec 20 -ContentType 'application/json; charset=utf-8' `
                                -Headers @{ Authorization = "Bearer $tokP"; 'Idempotency-Key' = "announce:$($payload.proposalId)" } -Body ([Text.Encoding]::UTF8.GetBytes($json))
                            Say ("announce {0}: HTTP {1}" -f $payload.proposalId, $resp.StatusCode)
                        } catch { Say ("announce {0} failed: {1}" -f $payload.proposalId, (Protect-Text $_.Exception.Message $script:Needles)) }
                    } else {
                        Say 'PULSO_PLATFORM_URL / PULSO_PLATFORM_SERVICE_TOKEN are not set: this is what WOULD be sent (POST /api/v1/internal/builder/proposals/announce):'
                        foreach ($line in ($json -split "`r?`n")) { Say "  $line" }
                        Say '  (evidenceLinks are opaque ids derived from the finding, not real case ids)'
                    }
                }
            }
            'Down' {
                Step-Header 'Down' "stopping the stack '$($settings.Prefix)' only"
                $e = Get-StackEnvironment -Settings $settings -AgentCoreDir $AgentCoreDir -GatewayDir $GatewayDir
                $a = @((Join-Path $root 'scripts\dev-stack\stack.py'), 'down'); if ($Purge) { $a += '--purge' }
                $r = Invoke-Scrubbed -File $python -Arguments $a -Env $e -Needles $script:Needles -WorkDir $root -LogPath $logPath
                if ($r.ExitCode -ne 0) { throw 'stack.py down failed' }
                $benv = [ordered]@{ PULSO_STACK_PREFIX = $settings.BatteryPrefix; PULSO_STACK_PORT_CORE = "$($settings.BatteryCore)"; PULSO_STACK_PORT_PG = "$($settings.BatteryPg)"; PULSO_STACK_PORT_GW = "$($settings.BatteryGw)" }
                $ba = @((Join-Path $root 'scripts\battery\demo_core.py'), 'down'); if ($Purge) { $ba += '--purge' }
                [void](Invoke-Scrubbed -File $python -Arguments $ba -Env $benv -Needles $script:Needles -WorkDir $root -LogPath $logPath)
                # the battery core's own containers (only the ones this script named)
                [void](Invoke-Podman -PodmanArgs @('rm', '-f', "$($settings.BatteryPrefix)-postgres", "$($settings.BatteryPrefix)-llm-gateway"))
                Say "stopped: $($settings.Prefix)-* and $($settings.BatteryPrefix)-* containers and serve processes (volumes kept unless -Purge; other lanes untouched)"
            }
        }
        $results.Add(("{0,-9} ok     {1,7:N1} s" -f $step, $sw.Elapsed.TotalSeconds))
    } catch {
        $failed = $true
        Say ("STEP {0} FAILED: {1}" -f $step, (Protect-Text $_.Exception.Message $script:Needles))
        $results.Add(("{0,-9} FAILED {1,7:N1} s" -f $step, $sw.Elapsed.TotalSeconds))
        break
    } finally {
        if ($engine) { try { $engine.StandardInput.Close() } catch { }; Start-Sleep -Milliseconds 800; Stop-Tree $engine }
    }
}

Say ''
Say '--- step durations'
foreach ($r in $results) { Say "  $r" }
Say ("total {0:N1} s" -f $totalWatch.Elapsed.TotalSeconds)
if ($failed) { exit 1 }
exit 0

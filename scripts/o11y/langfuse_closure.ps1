<#
.SYNOPSIS
  ONE command that closes the Langfuse integration: brings a LOCAL stack up, generates real traffic, sends it to Langfuse Cloud
  through a local forwarder and VERIFIES it by reading Langfuse back.
      .\scripts\o11y\langfuse_closure.ps1 -All
.DESCRIPTION
  Works from any current directory (it changes to the repository root itself). Windows PowerShell 5.1 and pwsh.
  Steps run in this fixed order, whichever switches you pass:
    -Models   checks D:\.codex\factored\langfuse.env (key NAMES only), pings Langfuse /api/public/health, registers the three models
              with prices (xiaomi/mimo-v2.6-flash, xiaomi/mimo-v2.6-pro, z-ai/glm-5.3-flash); existing ones are skipped.
    -Up       starts the loopback forwarder (holds the Langfuse key, in its process only) and the own stack 'pulso-lfc'
              (postgres, llm-gateway built from the PR 4 checkout, agent-core from the local scratch branch = main + PR 48, both exporting
              OTLP to the forwarder with content on).
    -Traffic  2 value-loop stories (SYNTHETIC planted cells, Builder xiaomi/mimo-v2.6-pro) with ONE trace id each shared by the engine,
              gateway and agent-core spans, 2 plain agent runs (disputas es, consultas pt), the run-trace bridge, the scores.
    -Verify   reads Langfuse back (traces, observations, content, the three sources, models with cost, scores): counts only,
              exit 1 with a precise diagnosis when something is missing.
    -Down     stops the forwarder and the pulso-lfc stack (other stacks are never touched).
    -All      = -Models -Up -Traffic -Verify -Down (Down also runs when an earlier step failed; -KeepUp skips it).
  -Mock       send to a local MOCK Langfuse (scripts/o11y/mock_langfuse.py) instead of Langfuse Cloud: a dry run of everything.
  The Langfuse key is read ONLY by python child processes through --env-file; it is never printed, never on a command line, never in
  the environment of this script. Everything a child prints is masked first.
  Exit code: 0 ok, 1 a step failed, 2 usage.
#>
[CmdletBinding()]
param(
    [switch]$Models, [switch]$Up, [switch]$Traffic, [switch]$Verify, [switch]$Down, [switch]$All, [switch]$KeepUp, [switch]$Mock, [switch]$Purge,
    [string]$EnvFile = '', [string]$AgentCoreDir = '', [string]$GatewayDir = '', [int]$WaitSecs = 240, [int]$TimeoutMin = 45
)
$ErrorActionPreference = 'Stop'
try { [Console]::OutputEncoding = New-Object Text.UTF8Encoding($false) } catch { }
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$root = (Resolve-Path (Join-Path $here '..\..')).Path
Set-Location -LiteralPath $root          # absolute-path safe: the caller's current directory does not matter
. (Join-Path $here 'langfuse_closure.lib.ps1')

try { $plan = Get-LfcPlan -Models:$Models -Up:$Up -Traffic:$Traffic -Verify:$Verify -Down:$Down -All:$All -KeepUp:$KeepUp -Mock:$Mock }
catch { [Console]::Error.WriteLine("usage error: $($_.Exception.Message)"); exit 2 }

$factored = Split-Path -Parent (Split-Path -Parent $root)
$S = Get-LfcSettings
$stateDir = Join-Path $root '.dev-stack\lfc'
$null = New-Item -ItemType Directory -Force -Path $stateDir
$logPath = Join-Path $stateDir ("closure-{0}.log" -f (Get-Date -Format 'yyyyMMdd-HHmmss'))
$python = (Get-Command python -ErrorAction Stop).Source
$psExe = (Get-Process -Id $PID).Path
$runPs1 = Join-Path $root 'scripts\demo-loop\run.ps1'
$o11y = Join-Path $root 'scripts\o11y'
if (-not $EnvFile) { $EnvFile = $(if ($env:PULSO_LANGFUSE_ENV) { $env:PULSO_LANGFUSE_ENV } else { Join-Path $factored 'langfuse.env' }) }
if (-not $AgentCoreDir) { $AgentCoreDir = $(if ($env:PULSO_LFC_AGENT_CORE_DIR) { $env:PULSO_LFC_AGENT_CORE_DIR } else { Join-Path $factored 'worktrees\agent-core-claude-lfc' }) }
if (-not $GatewayDir) { $GatewayDir = $(if ($env:PULSO_LFC_GATEWAY_DIR) { $env:PULSO_LFC_GATEWAY_DIR } else { Join-Path $factored 'worktrees\llm-gateway-claude-otel' }) }
$mockEnvPath = Join-Path $stateDir 'mock.env'
$activeEnv = $(if ($plan.Mock) { $mockEnvPath } else { $EnvFile })

# needles: every secret value we may hold is masked in anything printed
$lfValues = @{}
if (-not $plan.Mock) { $lfValues = Read-EnvFileValues -Path $EnvFile }
$others = @{}
foreach ($f in @($(if ($env:PULSO_AGENT_CORE_ENV) { $env:PULSO_AGENT_CORE_ENV } else { Join-Path $factored 'agent-core.env' }), $(if ($env:PULSO_LLM_GATEWAY_ENV) { $env:PULSO_LLM_GATEWAY_ENV } else { Join-Path $factored 'llm-gateway.env' }))) {
    $v = Read-EnvFileValues -Path $f; foreach ($k in $v.Keys) { $others["$f|$k"] = $v[$k] }
}
$script:Needles = @(Get-LfcNeedles -LangfuseValues $lfValues -Others @($others))
function Say {
    param([string]$Text = '')
    $safe = Protect-Text -Text $Text -Needles $script:Needles
    Write-Host $safe
    try { [IO.File]::AppendAllText($logPath, $safe + "`n") } catch { }
}
function Step-Header { param([string]$Name, [string]$Text) Say ''; Say ("=== [{0}] {1}" -f $Name, $Text) }
function Test-Http {
    param([string]$Url)
    try { $r = Invoke-WebRequest -Uri $Url -UseBasicParsing -TimeoutSec 4; return ($r.StatusCode -eq 200) } catch { return $false }
}
function Stop-Tree { param([int]$ProcId) if ($ProcId -gt 0) { try { & taskkill /PID $ProcId /T /F 2>&1 | Out-Null } catch { } } }
function Invoke-Py {
    param([string[]]$Arguments, [System.Collections.IDictionary]$Env = @{}, [switch]$Quiet)
    Invoke-Scrubbed -File $python -Arguments $Arguments -Env $Env -Needles $script:Needles -WorkDir $root -Quiet:$Quiet -LogPath $logPath
}
function Invoke-Runner {
    param([string[]]$Arguments, [System.Collections.IDictionary]$Env = @{})
    $common = @('-StackPrefix', $S.Prefix, '-PgPort', "$($S.PgPort)", '-GwPort', "$($S.GwPort)", '-CorePort', "$($S.CorePort)", '-EnginePort', "$($S.EnginePort)",
        '-StateName', $S.StateName, '-AgentCoreDir', $AgentCoreDir, '-GatewayDir', $GatewayDir)
    Invoke-Scrubbed -File $psExe -Arguments (@('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $runPs1) + $common + $Arguments) -Env $Env -Needles $script:Needles -WorkDir $root -LogPath $logPath
}
function Read-LfcState {
    $p = Join-Path $root '.dev-stack\lfc\state.json'
    if (-not (Test-Path -LiteralPath $p)) { throw 'no lfc state yet: the Traffic loop has not run' }
    Read-JsonFile -Path $p
}
function Start-Background {
    param([string]$Name, [string[]]$Arguments, [System.Collections.IDictionary]$Env = @{})
    $log = Join-Path $stateDir "$Name.log"
    $argLine = ($Arguments | ForEach-Object { if ($_ -match '[\s"]') { '"' + $_ + '"' } else { $_ } }) -join ' '
    $psi = New-Object Diagnostics.ProcessStartInfo
    $psi.FileName = $env:ComSpec
    $psi.Arguments = '/c ""' + $python + '" ' + $argLine + ' >> "' + $log + '" 2>&1"'
    $psi.UseShellExecute = $false; $psi.CreateNoWindow = $true; $psi.WorkingDirectory = $root
    foreach ($k in $Env.Keys) { $psi.EnvironmentVariables[[string]$k] = [string]$Env[$k] }
    $p = [Diagnostics.Process]::Start($psi)
    [IO.File]::WriteAllText((Join-Path $stateDir "$Name.pid"), "$($p.Id)")
    $p
}
function Stop-Background {
    param([string]$Name)
    $f = Join-Path $stateDir "$Name.pid"
    if (Test-Path -LiteralPath $f) { $id = 0; [void][int]::TryParse(([IO.File]::ReadAllText($f)).Trim(), [ref]$id); Stop-Tree $id; Remove-Item -LiteralPath $f -Force -ErrorAction SilentlyContinue }
}
function Start-MockIfNeeded {
    if (-not $plan.Mock) { return }
    [IO.File]::WriteAllText($mockEnvPath, "LANGFUSE_BASE_URL=http://127.0.0.1:$($S.MockPort)`nLANGFUSE_PUBLIC_KEY=pk-lf-mock`nLANGFUSE_SECRET_KEY=sk-lf-mock`n")
    if (Test-Http "http://127.0.0.1:$($S.MockPort)/api/public/health") { return }
    [void](Start-Background -Name 'mock' -Arguments @((Join-Path $o11y 'mock_langfuse.py'), '--port', "$($S.MockPort)"))
    for ($i = 0; $i -lt 20 -and -not (Test-Http "http://127.0.0.1:$($S.MockPort)/api/public/health"); $i++) { Start-Sleep -Milliseconds 500 }
    if (-not (Test-Http "http://127.0.0.1:$($S.MockPort)/api/public/health")) { throw 'the mock Langfuse did not start' }
    Say "MOCK Langfuse on 127.0.0.1:$($S.MockPort) (dry run: nothing leaves this machine)"
}
function Test-LangfuseReachable {
    $chk = Test-LangfuseEnvFile -Path $activeEnv -AllowHttpLoopback:$plan.Mock
    if (-not $chk.Ok) { throw ("langfuse env file: " + ($chk.Problems -join '; ') + " (key names expected: $($script:LangfuseKeys -join ', '))") }
    Say "env file: all 3 key names present (LANGFUSE_SECRET_KEY, LANGFUSE_PUBLIC_KEY, LANGFUSE_BASE_URL); host $($chk.Host); values are never shown"
    $a = @((Join-Path $o11y 'langfuse_verify.py'), 'health', '--env-file', $activeEnv); if (-not $plan.Mock) { $a += '--allow-external' }
    $r = Invoke-Py -Arguments $a -Quiet
    $line = [string]($r.Output | Select-Object -First 1)
    Say ("ping: " + $line)
    if ($r.ExitCode -ne 0) { throw (Get-HealthDiagnosis -Status $line) }
}
function Initialize-ScratchAgentCore {
    # LOCAL scratch branch = origin/main + PR 48 (feat/otel-traceparent-langfuse). Never pushed.
    if (-not (Test-Path -LiteralPath (Join-Path $AgentCoreDir 'pyproject.toml'))) {
        $repo = Join-Path $factored 'tmp\shared\agent-core'
        if (-not (Test-Path -LiteralPath (Join-Path $repo '.git'))) { throw "no agent-core checkout at $AgentCoreDir and none at $repo to create it from (-AgentCoreDir)" }
        Say "creating the local scratch worktree $AgentCoreDir (origin/main + PR 48; never pushed)"
        & git -C $repo fetch -q origin 2>&1 | Out-Null
        & git -C $repo worktree add -b 'scratch/lfc-agentcore' $AgentCoreDir 'origin/main' 2>&1 | ForEach-Object { Say "  $_" }
        & git -C $AgentCoreDir merge --no-edit 'origin/feat/otel-traceparent-langfuse' 2>&1 | ForEach-Object { Say "  $_" }
        if ($LASTEXITCODE -ne 0) { throw 'merging PR 48 into the scratch branch failed' }
    }
    if (-not (Test-Path -LiteralPath (Join-Path $AgentCoreDir 'tests\m09\test_traceparent_extract.py'))) { throw "agent-core at $AgentCoreDir does not contain PR 48 (traceparent + langfuse attributes)" }
    if (-not (Test-Path -LiteralPath (Join-Path $GatewayDir 'docs\adr\0002-langfuse-attributes-and-opt-in-content-capture.md'))) { throw "llm-gateway at $GatewayDir does not contain PR 4 (langfuse attributes + content)" }
}
function Wait-ForwarderDrained {
    $stats = $null; $quiet = 0
    $deadline = (Get-Date).AddSeconds(240)
    while ((Get-Date) -lt $deadline) {
        try { $stats = Invoke-RestMethod -Uri "$($S.ForwarderUrl)/v1/stats" -TimeoutSec 5 } catch { throw 'the forwarder is not answering: the lfc stack is not up (-Up)' }
        if ([int]$stats.pending -eq 0) { $quiet++ } else { $quiet = 0 }
        if ($quiet -ge 3) { break }
        Start-Sleep -Seconds 1
    }
    Say ("forwarder: forwarded {0}, failed {1}, retries {2}, pending {3}" -f $stats.forwarded, $stats.failed, $stats.retries, $stats.pending)
    if ([int]$stats.failed -gt 0) { Say "WARNING: the forwarder could not deliver $($stats.failed) upload(s) to Langfuse; see $(Join-Path $stateDir 'forwarder.log') (status codes only)" }
    if ([int]$stats.pending -gt 0) { throw 'the forwarder still has pending uploads after 240 s' }
}

$results = New-Object System.Collections.Generic.List[string]
$failed = $false
$total = [Diagnostics.Stopwatch]::StartNew()
Say ("Langfuse closure  {0}  steps: {1}  target: {2}" -f (Get-Date -Format 'yyyy-MM-dd HH:mm:ss'), ($plan.Steps -join ' '), $(if ($plan.Mock) { 'MOCK' } else { 'Langfuse Cloud' }))
Say "log: $logPath"
$downDone = $false

function Invoke-DownStep {
    Step-Header 'Down' "stopping the forwarder and the stack '$($S.Prefix)' only"
    Stop-Background 'forwarder'
    $downArgs = @('-Down'); if ($Purge) { $downArgs += '-Purge' }
    $r = Invoke-Runner -Arguments $downArgs
    if ($r.ExitCode -ne 0) { throw 'run.ps1 -Down failed' }
    Stop-Background 'mock'
    Say "stopped: $($S.Prefix)-* containers, agent-core serve, the forwarder (and the mock if one ran)."
}

foreach ($step in $plan.Steps) {
    $sw = [Diagnostics.Stopwatch]::StartNew()
    try {
        switch ($step) {
            'Models' {
                Step-Header 'Models' 'Langfuse reachable, then the three priced model definitions (idempotent)'
                Start-MockIfNeeded
                Test-LangfuseReachable
                $a = @((Join-Path $o11y 'langfuse_verify.py'), 'models', '--env-file', $activeEnv); if (-not $plan.Mock) { $a += '--allow-external' }
                $r = Invoke-Py -Arguments $a
                if ($r.ExitCode -ne 0) { throw 'registering the models failed (status above; HTTP 401 = wrong keys for this project)' }
            }
            'Up' {
                Step-Header 'Up' "forwarder :$($S.ForwarderPort), then stack '$($S.Prefix)': postgres :$($S.PgPort), llm-gateway :$($S.GwPort), agent-core :$($S.CorePort)"
                Start-MockIfNeeded
                Test-LangfuseReachable
                Initialize-ScratchAgentCore
                Stop-Background 'forwarder'
                $fa = @((Join-Path $o11y 'otlp_forwarder.py'), '--port', "$($S.ForwarderPort)", '--env-file', $activeEnv); if (-not $plan.Mock) { $fa += '--allow-external' }
                [void](Start-Background -Name 'forwarder' -Arguments $fa)
                $ok = $false
                for ($i = 0; $i -lt 20 -and -not $ok; $i++) { Start-Sleep -Milliseconds 500; $ok = Test-Http "$($S.ForwarderUrl)/healthz" }
                if (-not $ok) { throw "the forwarder did not start (see $(Join-Path $stateDir 'forwarder.log'))" }
                Say "forwarder up on $($S.ForwarderUrl) (loopback only; holds the key in its own process)"
                $e = Get-LfcStackEnvironment -Settings $S
                $r = Invoke-Runner -Arguments @('-Up', '-FreshGateway') -Env $e
                if ($r.ExitCode -ne 0) { throw 'the stack did not come up (run.ps1 -Up failed, output above)' }
                Say "stack up: gateway http://127.0.0.1:$($S.GwPort) (PR 4 build, content on), agent-core http://127.0.0.1:$($S.CorePort) (main + PR 48, content on); both export OTLP to the forwarder"
            }
            'Traffic' {
                Step-Header 'Traffic' '2 value-loop stories + 2 plain agent runs (SYNTHETIC input), all sent through the forwarder'
                foreach ($u in @("$($S.ForwarderUrl)/healthz", "http://127.0.0.1:$($S.CorePort)/healthz", "http://127.0.0.1:$($S.GwPort)/healthz")) {
                    if (-not (Test-Http $u)) { throw "not answering at $u : agent-core unreachable = the lfc stack is not up (run -Up)" }
                }
                $rel = @{ PULSO_RELEASE = ('lfc-' + (Get-Date -Format 'yyyyMMdd')) }
                $r = Invoke-Runner -Arguments @('-Cells', '-Loop', '-Synthetic', '-PlantedCount', '2', '-MaxFindings', '2', '-BuilderModel', 'xiaomi/mimo-v2.6-pro', '-TimeoutMin', "$TimeoutMin") -Env $rel
                if ($r.ExitCode -ne 0) { throw 'the value loop did not finish (run.ps1 -Loop failed, output above)' }
                $st = Read-LfcState
                foreach ($f in 'result_path', 'events_path', 'calls_path', 'run_id') { if (-not $st.PSObject.Properties[$f]) { throw "the loop left no '$f' (engine snapshot missing)" } }
                $loop = Read-JsonFile -Path $st.result_path
                $n = Get-LoopFindingCount -Loop $loop
                if ($n -lt 1) { throw 'the loop produced no finding: no story to trace' }
                $fwdEnv = @{ PULSO_O11Y_FORWARDER_ENDPOINT = $S.ForwarderUrl }
                $storyLines = New-Object System.Collections.Generic.List[string]
                for ($i = 0; $i -lt $n; $i++) {
                    $dump = Join-Path $stateDir "engine-story-$i.json"
                    $r = Invoke-Py -Env $fwdEnv -Arguments @((Join-Path $o11y 'engine_trace.py'), '--outcome', $st.result_path, '--run-id', [string]$st.run_id, '--events-file', $st.events_path,
                        '--model-calls-file', $st.calls_path, '--finding', "$i", '--target', 'forwarder', '--capture-content', '--environment', 'local', '--dump', $dump)
                    if ($r.ExitCode -ne 0) { throw "engine_trace.py failed for finding $i (output above)" }
                    foreach ($l in $r.Output) { $storyLines.Add($l) }
                }
                $stories = Get-StoryTraceIds -Lines $storyLines.ToArray()
                if ($stories.Count -lt 1) { throw 'engine_trace.py produced no story trace id' }
                Say ("engine stories sent: {0} (one trace id each)" -f $stories.Count)
                $runsOut = Join-Path $stateDir 'agent_runs.json'
                $r = Invoke-Py -Arguments @((Join-Path $o11y 'agent_runs.py'), '--base', "http://127.0.0.1:$($S.CorePort)", '--forwarder', $S.ForwarderUrl, '--out', $runsOut, '--agent-core-dir', $AgentCoreDir)
                if ($r.ExitCode -ne 0) { throw 'the plain agent runs failed (output above)' }
                $runs = Read-JsonFile -Path $runsOut
                $b = Invoke-Py -Env $fwdEnv -Arguments @((Join-Path $o11y 'runtrace_bridge.py'), '--target', 'forwarder', '--core-url', "http://127.0.0.1:$($S.CorePort)", '--token-file',
                    (Join-Path $root '.dev-stack\tokens.json'), '--token-key', 'admin', '--state', (Join-Path $stateDir 'bridge-state.json'), '--once')
                if ($b.ExitCode -ne 0) { Say 'WARNING: the run-trace bridge failed (its agent.run traces are extra; the story traces are not affected)' }
                Wait-ForwarderDrained
                $man = [ordered]@{ stories = @($stories); agent_runs = @($runs.agent_runs); created = (Get-Date -Format 'o') }
                [IO.File]::WriteAllText((Join-Path $stateDir 'traffic.json'), ($man | ConvertTo-Json -Depth 4), (New-Object Text.UTF8Encoding($false)))
                Say ("traffic done: {0} story trace(s), {1} agent-run trace(s); manifest {2}" -f $stories.Count, @($runs.agent_runs).Count, (Join-Path $stateDir 'traffic.json'))
            }
            'Verify' {
                Step-Header 'Verify' 'reading Langfuse back (counts only)'
                $man = Join-Path $stateDir 'traffic.json'
                if (-not (Test-Path -LiteralPath $man)) { throw 'no traffic manifest: run -Traffic first' }
                $a = @((Join-Path $o11y 'langfuse_verify.py'), 'verify', '--env-file', $activeEnv, '--manifest', $man, '--wait-secs', "$WaitSecs"); if (-not $plan.Mock) { $a += '--allow-external' }
                $r = Invoke-Py -Arguments $a
                if ($r.ExitCode -ne 0) { throw 'VERIFY FAILED: see the PROBLEM lines above' }
            }
            'Down' { Invoke-DownStep; $downDone = $true }
        }
        $results.Add(("{0,-8} ok     {1,7:N1} s" -f $step, $sw.Elapsed.TotalSeconds))
    } catch {
        $failed = $true
        $msg = Protect-Text $_.Exception.Message $script:Needles
        Say ("STEP {0} FAILED: {1}" -f $step, $msg)
        $hint = Get-StackDownHint -Message $msg
        if ($hint) { Say "  hint: $hint" }
        $results.Add(("{0,-8} FAILED {1,7:N1} s" -f $step, $sw.Elapsed.TotalSeconds))
        break
    }
}
if ($failed -and ($plan.Steps -contains 'Down') -and -not $downDone) {
    try { Invoke-DownStep } catch { Say ("(teardown failed: {0})" -f (Protect-Text $_.Exception.Message $script:Needles)) }
}

Say ''
Say '--- step durations'
foreach ($r in $results) { Say "  $r" }
Say ("total {0:N1} s" -f $total.Elapsed.TotalSeconds)
Say ''
Say 'Re-run a part:  .\scripts\o11y\langfuse_closure.ps1 -Models | -Up | -Traffic | -Verify | -Down   (-Mock = dry run against a local mock; -KeepUp with -All keeps the stack)'
if ($failed) { exit 1 }
exit 0

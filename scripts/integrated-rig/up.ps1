<#
.SYNOPSIS
  Integrated rig, STAGE 1 (API level, no browser): brings up, in order,
    1. Postgres 16 + llm-gateway (podman, prefix pulso-env1, own ports) and agent-core `serve --registry-api` from agent-core main
       (through scripts/demo-loop/run.ps1 -Up -> scripts/dev-stack/stack.py: registry-e2e agents, engine `builder` kid, dev admin kid),
    2. the platform's agent-core signing keys + the KEY MERGE (merge_keys.py: union of platform keys, engine kid, dev admin kid),
    3. the support-platform API (`uvicorn`, own SQLite recreated, seeded) with CC_AGENT_CORE_URL, CC_AGENT_KEYS_FILE and CC_INTERNAL_SERVICE_TOKEN,
    4. a check that the engine binary resolves. The engine itself (`pulso run`, with the builder identity PULSO_REGISTRY_TOKEN and the
       platform URL/token) lives only as long as one loop job: run_story.ps1 starts and stops it.
  up.ps1 [-MinFreeMb 1500] [-WaitRamMin 0] [-Force] [-ReuseStack] [-Spa] [-PlatformDir D] [-AgentCoreDir D] [-GatewayDir D] [-PulsoExe F]
.DESCRIPTION
  Memory gate: starts only when free RAM > -MinFreeMb (default 1500). -WaitRamMin N waits up to N minutes for it. -Force skips the gate.
  Credentials: agent-core.env and llm-gateway.env are read by scripts/demo-loop/run.ps1 and go only into child process environments. The
  platform service token is generated here, per run, and kept in .dev-stack/integrated-rig/secrets.json (gitignored) for run_story.ps1.
  Nothing secret is ever printed: every child line is masked and only variable NAMES are logged. Exit: 0 ok, 1 a step failed, 2 usage, 3 memory gate.
#>
[CmdletBinding()]
param(
    [int]$MinFreeMb = 1500, [int]$WaitRamMin = 0, [switch]$Force, [switch]$ReuseStack, [switch]$Spa,
    [string]$PlatformDir = '', [string]$AgentCoreDir = '', [string]$GatewayDir = '', [string]$PulsoExe = ''
)
$ErrorActionPreference = 'Stop'
try { [Console]::OutputEncoding = New-Object Text.UTF8Encoding($false) } catch { }
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$root = (Resolve-Path (Join-Path $here '..\..')).Path
. (Join-Path $here 'rig.lib.ps1')

$factored = Split-Path -Parent (Split-Path -Parent $root)
$settings = Get-RigSettings
$paths = Get-RigPaths -Root $root
if (-not $PlatformDir) { $PlatformDir = $(if ($env:PULSO_PLATFORM_DIR) { $env:PULSO_PLATFORM_DIR } else { Join-Path $factored 'tmp\env1\support-platform' }) }
if (-not $AgentCoreDir) { $AgentCoreDir = $(if ($env:PULSO_AGENT_CORE_DIR) { $env:PULSO_AGENT_CORE_DIR } else { Join-Path $factored 'tmp\env1\agent-core' }) }
if (-not $GatewayDir) { $GatewayDir = $(if ($env:PULSO_LLM_GATEWAY_DIR) { $env:PULSO_LLM_GATEWAY_DIR } else { Join-Path $factored 'tmp\shared\llm-gateway' }) }
$backend = Join-Path $PlatformDir 'backend'
$python = (Get-Command python -ErrorAction Stop).Source
$uv = (Get-Command uv -ErrorAction Stop).Source
$shell = Get-ChildShell
$null = New-Item -ItemType Directory -Force -Path $paths.Rig
$script:Needles = @()

function Say { param([string]$Text = '') Write-Host (Protect-Text -Text $Text -Needles $script:Needles) }
function Fail { param([string]$Text, [int]$Code = 1) Say "UP FAILED: $Text"; exit $Code }

if (-not (Test-Path -LiteralPath (Join-Path $backend 'pyproject.toml'))) { Fail "support-platform checkout not found at $PlatformDir (-PlatformDir)" 2 }
if (-not (Test-Path -LiteralPath (Join-Path $AgentCoreDir 'pyproject.toml'))) { Fail "agent-core checkout not found at $AgentCoreDir (-AgentCoreDir)" 2 }

# ---- memory gate --------------------------------------------------------------------------------------------------------------------
$deadline = (Get-Date).AddMinutes($WaitRamMin)
while ($true) {
    $free = Get-FreeRamMb
    if ($Force -or (Test-RamBudget -FreeMb $free -MinMb $MinFreeMb)) { break }
    if ((Get-Date) -ge $deadline) { Fail ("free RAM is {0} MB, the stage needs more than {1} MB (-WaitRamMin N waits, -Force skips)" -f $free, $MinFreeMb) 3 }
    Start-Sleep -Seconds 15
}
Say ("[0] memory gate: {0} MB free (> {1} MB){2}" -f $free, $MinFreeMb, $(if ($Force) { ' (forced)' } else { '' }))

# ---- engine binary -----------------------------------------------------------------------------------------------------------------
if (-not $PulsoExe) { $PulsoExe = $(if ($env:PULSO_EXE) { $env:PULSO_EXE } else { 'D:\cargo-targets\claude-w16\debug\pulso.exe' }) }
if (-not (Test-Path -LiteralPath $PulsoExe)) { Fail "pulso.exe not found at $PulsoExe (build once: cd seams; cargo build -j 1 -p pulso, or pass -PulsoExe)" 2 }

# ---- previous run of THIS rig --------------------------------------------------------------------------------------------------------
[void](Stop-PidTree -PidFile $paths.PlatformPid)
foreach ($f in @($paths.PlatformDb, ($paths.PlatformDb + '-journal'), ($paths.PlatformDb + '-wal'), ($paths.PlatformDb + '-shm'), $paths.AnnounceResults, $paths.Story, $paths.CaseIds)) {
    Remove-Item -LiteralPath $f -Force -ErrorAction SilentlyContinue
}
if (Test-Path -LiteralPath $paths.PlatformKeys) { Remove-Item -LiteralPath $paths.PlatformKeys -Recurse -Force }

# ---- credentials of this run ---------------------------------------------------------------------------------------------------------
$svc = New-ServiceToken
$script:Needles = @($svc)
Write-JsonFile -Path $paths.Secrets -Obj ([ordered]@{ service_token = $svc; created = (Get-Date).ToUniversalTime().ToString('o'); note = 'dev only, regenerated by every up.ps1, gitignored' })

# ---- 1. postgres + gateway + agent-core (demo-loop -Up, own prefix) ---------------------------------------------------------------------
Say "[1] postgres :$($settings.PgPort), llm-gateway :$($settings.GwPort), agent-core :$($settings.CorePort) (prefix $($settings.Prefix))"
$envUp = [ordered]@{}
foreach ($kv in (Get-LoopLaneEnvironment -Settings $settings).GetEnumerator()) { $envUp[$kv.Key] = $kv.Value }
foreach ($kv in (Get-AgentCoreGrantsEnvironment -Settings $settings -ServiceToken $svc).GetEnumerator()) { $envUp[$kv.Key] = $kv.Value }
Say ("    child environment names: " + ((Get-EnvNames -Env $envUp) -join ', '))
if (-not $ReuseStack) { $r = Invoke-Scrubbed -File $shell -Arguments @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', (Join-Path $root 'scripts\demo-loop\run.ps1'), '-Up',
        '-AgentCoreDir', $AgentCoreDir, '-GatewayDir', $GatewayDir) -Env $envUp -Needles $script:Needles -WorkDir $root
if ($r.ExitCode -ne 0) { Fail "demo-loop -Up exited $($r.ExitCode)" } }
foreach ($u in @("http://127.0.0.1:$($settings.CorePort)/healthz", "http://127.0.0.1:$($settings.GwPort)/healthz")) { if (-not (Test-Http $u)) { Fail "not answering: $u" } }

# ---- 2. platform signing keys + KEY MERGE ----------------------------------------------------------------------------------------------
Say '[2] platform agent-core keys (gen_agent_keys, fresh suffix per up) and the merge into the state key files'
$r = Invoke-Scrubbed -File $uv -Arguments @('sync', '--frozen', '--project', $backend) -Needles $script:Needles -WorkDir $backend -Quiet
if ($r.ExitCode -ne 0) { Fail ("uv sync (platform) exited $($r.ExitCode): " + (($r.Output | Select-Object -Last 3) -join ' ')) }
$r = Invoke-Scrubbed -File $uv -Arguments @('run', '--frozen', '--project', $backend, 'python', '-m', 'cc_platform.scripts.gen_agent_keys', '--out', $paths.PlatformKeys, '--suffix', (Get-Date -Format 'MMddHHmmss')) `
    -Needles $script:Needles -WorkDir $backend -Quiet
if ($r.ExitCode -ne 0) { Fail 'gen_agent_keys failed' }
Say '    keys written (private.json stays in the gitignored rig dir; never printed)'
$r = Invoke-Scrubbed -File $python -Arguments @((Join-Path $here 'merge_keys.py'), '--state-dir', $paths.Dev, '--platform-keys', $paths.PlatformKeys) -Needles $script:Needles -WorkDir $root
if ($r.ExitCode -ne 0) { Fail 'merge_keys refused (see above)' }

# ---- 3. platform API ----------------------------------------------------------------------------------------------------------------------
Say "[3] support-platform API :$($settings.PlatformPort) (own SQLite, seeded; CC_AGENT_CORE_URL, CC_AGENT_KEYS_FILE, CC_INTERNAL_SERVICE_TOKEN set)"
$penv = Get-PlatformEnvironment -Settings $settings -KeysFile (Join-Path $paths.PlatformKeys 'private.json') -ServiceToken $svc -DbPath $paths.PlatformDb
Say ("    child environment names: " + ((Get-EnvNames -Env $penv) -join ', '))
$psi = New-Object Diagnostics.ProcessStartInfo
$psi.FileName = $env:ComSpec
$psi.Arguments = '/c ""' + $uv + '" run --frozen --with tzdata --project "' + $backend + '" python -m uvicorn cc_platform.bootstrap.app:create_app --factory --host 127.0.0.1 --port ' + $settings.PlatformPort + ' --no-access-log >> "' + $paths.PlatformLog + '" 2>&1"'
$psi.UseShellExecute = $false; $psi.CreateNoWindow = $true; $psi.WorkingDirectory = $backend
$psi.RedirectStandardInput = $true; $psi.RedirectStandardOutput = $true; $psi.RedirectStandardError = $true  # the child writes its own log file; no inherited pipe may keep a caller waiting
foreach ($k in $penv.Keys) { $psi.EnvironmentVariables[[string]$k] = [string]$penv[$k] }
$proc = [Diagnostics.Process]::Start($psi)
[IO.File]::WriteAllText($paths.PlatformPid, [string]$proc.Id)
$ok = $false
for ($i = 0; $i -lt 90 -and -not $ok; $i++) {
    Start-Sleep -Seconds 1
    if ($proc.HasExited) { break }
    $ok = Test-Http "http://127.0.0.1:$($settings.PlatformPort)/api/v1/health"
}
if (-not $ok) {
    $tail = ''; if (Test-Path $paths.PlatformLog) { $tail = (Get-Content -LiteralPath $paths.PlatformLog -Tail 6) -join "`n" }
    Fail ("the platform did not become healthy. Log tail:`n" + (Protect-Text $tail $script:Needles))
}
Say ('    /api/v1/meta answers: ' + (Test-Http "http://127.0.0.1:$($settings.PlatformPort)/api/v1/meta") + ' (builder availability is checked by story_verify through a supervisor session)')


# ---- 3b. SPA (only with -Spa): lockfile install, build against this API, `vite preview` on 5174 (CORS origin is in CC_CORS_ORIGINS) -----------------
if ($Spa) {
    $fe = Join-Path $PlatformDir 'frontend'
    $npx = (Get-Command npx.cmd -ErrorAction Stop).Source
    Say "[3b] SPA build + preview :$($settings.SpaPort) (pnpm@9 via npx, frozen lockfile; node_modules stay in the platform checkout, never committed)"
    [void](Stop-PidTree -PidFile $paths.SpaPid)
    if (-not (Test-Path -LiteralPath (Join-Path $fe 'node_modules'))) {
        $r = Invoke-Scrubbed -File $npx -Arguments @('--yes', 'pnpm@9', 'install', '--frozen-lockfile') -Needles $script:Needles -WorkDir $fe -Quiet
        if ($r.ExitCode -ne 0) { Fail ("pnpm install failed: " + (($r.Output | Select-Object -Last 4) -join ' ')) }
    }
    $r = Invoke-Scrubbed -File $npx -Arguments @('--yes', 'pnpm@9', 'build') -Env @{ VITE_API_URL = "http://127.0.0.1:$($settings.PlatformPort)" } -Needles $script:Needles -WorkDir $fe -Quiet
    if ($r.ExitCode -ne 0) { Fail ("SPA build failed: " + (($r.Output | Select-Object -Last 6) -join ' ')) }
    $sp = New-Object Diagnostics.ProcessStartInfo
    $sp.FileName = $env:ComSpec
    $sp.Arguments = '/c ""' + $npx + '" --yes pnpm@9 exec vite preview --host 127.0.0.1 --port ' + $settings.SpaPort + ' --strictPort >> "' + $paths.SpaLog + '" 2>&1"'
    $sp.UseShellExecute = $false; $sp.CreateNoWindow = $true; $sp.WorkingDirectory = $fe
    $sp.RedirectStandardInput = $true; $sp.RedirectStandardOutput = $true; $sp.RedirectStandardError = $true
    $spProc = [Diagnostics.Process]::Start($sp)
    [IO.File]::WriteAllText($paths.SpaPid, [string]$spProc.Id)
    $spaOk = $false
    for ($i = 0; $i -lt 40 -and -not $spaOk; $i++) { Start-Sleep -Seconds 1; $spaOk = Test-Http "http://127.0.0.1:$($settings.SpaPort)" }
    if (-not $spaOk) { Fail "the SPA preview did not answer on :$($settings.SpaPort) (see spa.log)" }
    Say "    SPA: http://127.0.0.1:$($settings.SpaPort)"
}

# ---- 4. the engine ----------------------------------------------------------------------------------------------------------------------
Say "[4] engine binary: $PulsoExe (started per loop job by run_story.ps1 on :$($settings.EnginePort); PULSO_REGISTRY_TOKEN = builder principal pulso-engine, kid pulso-engine-dev-1)"
Write-JsonFile -Path (Join-Path $paths.Rig 'rig.json') -Obj ([ordered]@{ prefix = $settings.Prefix; pulso_exe = $PulsoExe; agent_core_dir = $AgentCoreDir; platform_dir = $PlatformDir; gateway_dir = $GatewayDir
    ports = [ordered]@{ postgres = $settings.PgPort; gateway = $settings.GwPort; agent_core = $settings.CorePort; engine = $settings.EnginePort; platform = $settings.PlatformPort }
    started = (Get-Date).ToUniversalTime().ToString('o') })
Say ("UP OK  free RAM now {0} MB.  Next: scripts/integrated-rig/health.ps1, then run_story.ps1; stop with down.ps1" -f (Get-FreeRamMb))
exit 0

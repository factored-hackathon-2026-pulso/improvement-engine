<#
.SYNOPSIS
  Preflight for the local loop on THIS machine (Windows, Linux, macOS; pwsh 7 recommended). Read-only: starts nothing, prints no secret
  (env files are only counted). Exit 0 = no blocker, 1 = at least one BLOCK row.
  pwsh scripts/dev-stack/doctor.ps1 [-NeedBuild]
  -NeedBuild  also require cargo (otherwise cargo is only a warning when the pulso binary already exists).
  Configuration: devconfig.env at the repo root (or PULSO_DEVCONFIG), env vars, sibling checkouts; see docs/dev/QUICKSTART_LOCAL.md.
#>
[CmdletBinding()]
param([switch]$NeedBuild)
$ErrorActionPreference = 'Continue'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$root = (Resolve-Path (Join-Path $here '../..')).Path
. (Join-Path $here 'DevConfig.ps1')
. (Join-Path $here 'Doctor.lib.ps1')
. (Join-Path $here '../integrated-rig/rig.lib.ps1')

$os = Get-DevOS
$factored = Split-Path -Parent (Split-Path -Parent $root)
$cfg = Get-DevConfig -Root $root -Export -Legacy @{
    PULSO_AGENT_CORE_DIR = (Join-Path $factored 'tmp/env1/agent-core'); PULSO_LLM_GATEWAY_DIR = (Join-Path $factored 'tmp/shared/llm-gateway')
    PULSO_PLATFORM_DIR = (Join-Path $factored 'tmp/env1/support-platform')
    PULSO_AGENT_CORE_ENV = (Join-Path $factored 'agent-core.env'); PULSO_LLM_GATEWAY_ENV = (Join-Path $factored 'llm-gateway.env')
    PULSO_EXE = 'D:\cargo-targets\claude-w16\debug\pulso.exe'
}
$v = $cfg.Values
$rows = New-Object System.Collections.ArrayList
function Add-Row { param($Check, $Status, $Detail = '', $Hint = '') [void]$rows.Add((New-DoctorRow $Check $Status $Detail $Hint)) }
function Find-Tool { param([string[]]$Names) foreach ($n in $Names) { $c = Get-Command $n -ErrorAction SilentlyContinue; if ($c) { return $c.Source } }; $null }
function Get-Out { param([string]$File, [string[]]$Arguments) try { ((& $File @Arguments 2>&1) | Out-String).Trim() } catch { '' } }

# OS + shell
Add-Row 'OS' 'OK' ("$os, config file: " + $(if (Test-Path -LiteralPath $(if ($env:PULSO_DEVCONFIG) { $env:PULSO_DEVCONFIG } else { Join-Path $root 'devconfig.env' })) { 'devconfig.env found' } else { 'none (defaults + env)' }))
if ($PSVersionTable.PSVersion.Major -ge 7) { Add-Row 'pwsh' 'OK' $PSVersionTable.PSVersion.ToString() }
elseif ($os -eq 'Windows') { Add-Row 'pwsh' 'WARN' ("Windows PowerShell " + $PSVersionTable.PSVersion) 'works on Windows; PowerShell 7 recommended: winget install Microsoft.PowerShell' }
else { Add-Row 'pwsh' 'BLOCK' $PSVersionTable.PSVersion.ToString() 'install PowerShell 7: https://aka.ms/powershell' }

# podman
$podman = Find-Tool @('podman')
if (-not $podman) { Add-Row 'podman' 'BLOCK' 'not found' $(if ($os -eq 'Linux') { 'install podman from your package manager (native, no machine)' } else { 'install podman, then: podman machine init --rootful --cpus 2 --memory 4096 <name>; podman machine start <name>; set PULSO_PODMAN_MACHINE=<name> in devconfig.env' }) }
else {
    $pa = @(Get-DevPodmanArgs)
    $info = Get-Out $podman (@($pa) + @('info', '--format', '{{.Host.Arch}}'))
    $sel = $(if ($pa.Count) { $pa -join ' ' } else { 'default connection' })
    if ($info -and $info -notmatch 'Error|error|cannot') { Add-Row 'podman' 'OK' "reachable ($sel)" }
    else { Add-Row 'podman' 'BLOCK' "not reachable ($sel)" $(if ($os -eq 'Linux') { 'check `podman info`; rootless is fine' } else { 'podman machine start <name>; PULSO_PODMAN_MACHINE=<name> (rootful machine: connection <name>-root) or PULSO_PODMAN_CONNECTION in devconfig.env' }) }
}

# RAM
$free = Get-DevFreeRamMb
if ($free -lt 0) { Add-Row 'free RAM' 'WARN' 'could not be read' 'up.ps1 -Force skips the memory gate' }
elseif ($free -le 1500) { Add-Row 'free RAM' 'BLOCK' "$free MB (gate: > 1500 MB)" 'close other programs, or up.ps1 -Force at your own risk' }
else { Add-Row 'free RAM' 'OK' "$free MB" }

# python, uv, node, pnpm, cargo
$py = Find-Tool @('python', 'python3')
if (-not $py) { Add-Row 'python 3.11+' 'BLOCK' 'not found' 'install Python 3.11 or newer (the scripts look for python, then python3)' }
else {
    $pv = ConvertFrom-PythonVersion (Get-Out $py @('--version'))
    if (Test-PythonVersionOk $pv) { Add-Row 'python 3.11+' 'OK' $pv.ToString() } else { Add-Row 'python 3.11+' 'BLOCK' $(if ($pv) { $pv.ToString() } else { 'unknown version' }) 'install Python 3.11 or newer' }
    & $py -c 'import yaml' 2>$null
    if ($LASTEXITCODE -eq 0) { Add-Row 'PyYAML' 'OK' 'importable' } else { Add-Row 'PyYAML' 'WARN' 'not importable here' 'pip install pyyaml (the rig also runs it through uv --with pyyaml)' }
}
if (Find-Tool @('uv')) { Add-Row 'uv' 'OK' (Get-Out (Find-Tool @('uv')) @('--version')) } else { Add-Row 'uv' 'BLOCK' 'not found' 'install uv: https://docs.astral.sh/uv/' }
if (Find-Tool @('node')) { Add-Row 'node' 'OK' (Get-Out (Find-Tool @('node')) @('--version')) } else { Add-Row 'node' 'WARN' 'not found (only -Spa needs it)' 'install Node 20+' }
if (Find-Tool @('pnpm')) { Add-Row 'pnpm' 'OK' (Get-Out (Find-Tool @('pnpm')) @('--version')) }
elseif (Find-Tool @('npx', 'npx.cmd')) { Add-Row 'pnpm' 'OK' 'via npx pnpm@9 (what up.ps1 -Spa uses)' }
else { Add-Row 'pnpm' 'WARN' 'not found (only -Spa needs it)' 'corepack enable, or install Node 20+ (npx)' }

# binaries
$exeOk = (Test-Path -LiteralPath $v['PULSO_EXE'])
$cargo = Find-Tool @('cargo')
if ($cargo) { Add-Row 'cargo' 'OK' (Get-Out $cargo @('--version')) }
elseif ($exeOk -and -not $NeedBuild) { Add-Row 'cargo' 'WARN' 'not found (the binary already exists)' 'only needed to rebuild: https://rustup.rs' }
else { Add-Row 'cargo' 'BLOCK' 'not found' 'install Rust: https://rustup.rs' }
if ($exeOk) { Add-Row 'pulso binary' 'OK' ($v['PULSO_EXE'] + " ($($cfg.Sources['PULSO_EXE']))") }
else { Add-Row 'pulso binary' 'BLOCK' ("not found: " + $v['PULSO_EXE']) "build it: see docs/dev/QUICKSTART_LOCAL.md (cargo build -j 1 -p pulso -p steps), or set PULSO_EXE in devconfig.env" }
if (Test-Path -LiteralPath $v['PULSO_STEPS_EXE']) { Add-Row 'steps binary' 'OK' $v['PULSO_STEPS_EXE'] } else { Add-Row 'steps binary' 'WARN' 'not found (optional sensor preview)' 'cargo build -j 1 -p steps' }

# checkouts
foreach ($c in @(@('agent-core checkout', 'PULSO_AGENT_CORE_DIR', 'pyproject.toml'), @('llm-gateway checkout', 'PULSO_LLM_GATEWAY_DIR', 'pyproject.toml'), @('support-platform checkout', 'PULSO_PLATFORM_DIR', 'backend/pyproject.toml'))) {
    $d = $v[$c[1]]
    if ($d -and (Test-Path -LiteralPath (Join-Path $d $c[2]))) { Add-Row $c[0] 'OK' "$d ($($cfg.Sources[$c[1]]))" }
    else { Add-Row $c[0] 'BLOCK' "not found at $d" "clone it next to this repo or set $($c[1]) in devconfig.env" }
}

# env files: counts only
foreach ($c in @(@('agent-core.env', 'PULSO_AGENT_CORE_ENV'), @('llm-gateway.env', 'PULSO_LLM_GATEWAY_ENV'))) {
    $st = Get-EnvFileStatus -Path $v[$c[1]]
    if ($st.Present -and $st.Keys -gt 0) { Add-Row $c[0] 'OK' "$($v[$c[1]]) ($($st.Keys) keys; values never shown)" }
    else { Add-Row $c[0] 'BLOCK' $(if ($st.Present) { 'present but empty: ' + $v[$c[1]] } else { 'not found: ' + $v[$c[1]] }) "create it from your own credentials (names only in the quickstart), or set $($c[1]) in devconfig.env" }
}

# ports
$s = Get-RigSettings
$ports = [ordered]@{ postgres = $s.PgPort; gateway = $s.GwPort; 'agent-core' = $s.CorePort; engine = $s.EnginePort; platform = $s.PlatformPort; spa = $s.SpaPort }
$busy = @($ports.GetEnumerator() | Where-Object { -not (Test-DevPortFree -Port $_.Value) } | ForEach-Object { "$($_.Key) :$($_.Value)" })
if ($busy.Count -eq 0) { Add-Row 'ports free' 'OK' (($ports.GetEnumerator() | ForEach-Object { $_.Value }) -join ' ') }
else { Add-Row 'ports free' 'WARN' ('in use: ' + ($busy -join ', ')) 'a previous run? scripts/integrated-rig/down.ps1; or change PULSO_RIG_*_PORT in devconfig.env' }

Format-DoctorTable -Rows $rows | ForEach-Object { Write-Host $_ }
$code = Get-DoctorExitCode -Rows $rows
Write-Host ''
Write-Host $(if ($code -eq 0) { 'doctor: no blockers' } else { 'doctor: blockers found, fix the BLOCK rows first' })
exit $code

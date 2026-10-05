# Central dev configuration + small OS helpers for the local-loop scripts (dot-sourced; tested by DevConfig.Tests.ps1).
# Windows PowerShell 5.1 AND pwsh (Windows, Linux, macOS): no `??`, no ternary, no `&&`; $IsWindows/$IsLinux/$IsMacOS do not exist on 5.1.
# Nothing here reads or prints a secret: devconfig.env accepts only the names listed in $script:DevConfigNames (paths, ports, podman names).
#
# Resolution order for every name:  devconfig.env  >  process environment  >  default relative to the repo (used when it exists)
#                                   >  legacy default of the calling script (D:\ layout of the original machine, used when it exists)
#                                   >  the relative default again (so the error message names the portable location).

$script:DevConfigNames = @(
    'PULSO_AGENT_CORE_DIR', 'PULSO_LLM_GATEWAY_DIR', 'PULSO_PLATFORM_DIR', 'PULSO_AGENT_CORE_ENV', 'PULSO_LLM_GATEWAY_ENV',
    'PULSO_EXE', 'PULSO_STEPS_EXE', 'CARGO_TARGET_DIR', 'PULSO_PODMAN_MACHINE', 'PULSO_PODMAN_CONNECTION', 'PULSO_STACK_PREFIX',
    'PULSO_RIG_PG_PORT', 'PULSO_RIG_GW_PORT', 'PULSO_RIG_CORE_PORT', 'PULSO_RIG_ENGINE_PORT', 'PULSO_RIG_PLATFORM_PORT', 'PULSO_RIG_SPA_PORT',
    'PULSO_DEMO_PG_PORT', 'PULSO_DEMO_GW_PORT', 'PULSO_DEMO_CORE_PORT', 'PULSO_DEMO_ENGINE_PORT', 'PULSO_DEMO_DATA_ROOT',
    'PULSO_LANGFUSE_ENV', 'PULSO_LFC_AGENT_CORE_DIR', 'PULSO_LFC_GATEWAY_DIR'
)

# ---- OS ------------------------------------------------------------------------------------------------------------------------------

# 'Windows' | 'Linux' | 'macOS'. -Override lets tests (and PULSO_DEV_OS) pin the answer.
function Get-DevOS {
    param([string]$Override = '')
    if (-not $Override) { $Override = $env:PULSO_DEV_OS }
    if ($Override) { return $Override }
    if ($PSVersionTable.PSEdition -ne 'Core') { return 'Windows' }
    if ($IsWindows) { return 'Windows' }
    if ($IsMacOS) { return 'macOS' }
    'Linux'
}

function Get-DevExeSuffix { param([string]$OS = '') if ((Get-DevOS -Override $OS) -eq 'Windows') { '.exe' } else { '' } }
function Get-DevNullDevice { param([string]$OS = '') if ((Get-DevOS -Override $OS) -eq 'Windows') { 'nul' } else { '/dev/null' } }

# ---- devconfig.env ---------------------------------------------------------------------------------------------------------------------

# NAME=value lines; '#' comments; quotes stripped; an EMPTY value is kept (PULSO_PODMAN_MACHINE= means "no machine"). Unknown names are ignored.
function Read-DevConfigFile {
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
        if ($script:DevConfigNames -contains $k) { $out[$k] = $v }
    }
    $out
}

function Get-DevEnvValue {
    param([string]$Name, $Environment)
    if ($null -ne $Environment) { if ($Environment.ContainsKey($Name)) { return [string]$Environment[$Name] } else { return $null } }
    [Environment]::GetEnvironmentVariable($Name)
}

# Returns @{ Values = name -> value; Sources = name -> file|env|default|legacy }.
# -Legacy: name -> the old absolute default of the calling script. -Environment: a hashtable instead of the process env (tests).
# -Export: copy the values that came from the file into the process env (scripts read $env:PULSO_* further down; children inherit them).
function Get-DevConfig {
    param(
        [Parameter(Mandatory)][string]$Root, [hashtable]$Legacy = @{}, $Environment = $null, [string]$OS = '', [switch]$Export
    )
    $os = Get-DevOS -Override $OS
    $parent = Split-Path -Parent $Root
    $file = Get-DevEnvValue -Name 'PULSO_DEVCONFIG' -Environment $Environment
    if (-not $file) { $file = Join-Path $Root 'devconfig.env' }
    $fromFile = Read-DevConfigFile -Path $file
    $values = @{}; $sources = @{}

    $pick = {
        param([string]$Name, [string]$Default, [bool]$IsPath)
        if ($fromFile.ContainsKey($Name) -and ($fromFile[$Name] -ne '' -or $Name -eq 'PULSO_PODMAN_MACHINE')) { $values[$Name] = $fromFile[$Name]; $sources[$Name] = 'file'; return }
        $e = Get-DevEnvValue -Name $Name -Environment $Environment
        if ($e) { $values[$Name] = $e; $sources[$Name] = 'env'; return }
        if (-not $IsPath) { $values[$Name] = $Default; $sources[$Name] = 'default'; return }
        if ($Default -and (Test-Path -LiteralPath $Default)) { $values[$Name] = $Default; $sources[$Name] = 'default'; return }
        if ($Legacy.ContainsKey($Name) -and $Legacy[$Name] -and (Test-Path -LiteralPath $Legacy[$Name])) { $values[$Name] = [string]$Legacy[$Name]; $sources[$Name] = 'legacy'; return }
        $values[$Name] = $Default; $sources[$Name] = 'default'
    }
    $envDir = Join-Path $parent '.pulso-env'
    & $pick 'PULSO_AGENT_CORE_DIR' (Join-Path $parent 'agent-core') $true
    & $pick 'PULSO_LLM_GATEWAY_DIR' (Join-Path $parent 'llm-gateway') $true
    & $pick 'PULSO_PLATFORM_DIR' (Join-Path $parent 'support-platform') $true
    & $pick 'PULSO_AGENT_CORE_ENV' (Join-Path $envDir 'agent-core.env') $true
    & $pick 'PULSO_LLM_GATEWAY_ENV' (Join-Path $envDir 'llm-gateway.env') $true
    & $pick 'CARGO_TARGET_DIR' (Join-Path $Root 'target') $false
    $debug = Join-Path $values['CARGO_TARGET_DIR'] 'debug'
    $sfx = Get-DevExeSuffix -OS $os
    & $pick 'PULSO_EXE' (Join-Path $debug ('pulso' + $sfx)) $true
    & $pick 'PULSO_STEPS_EXE' (Join-Path $debug ('steps_cli' + $sfx)) $true
    # unset-able names without a path default (callers keep their own defaults when the value is empty)
    foreach ($n in $script:DevConfigNames) {
        if ($values.ContainsKey($n)) { continue }
        & $pick $n '' $false
    }
    if ($Export) { foreach ($k in $fromFile.Keys) { if ($fromFile[$k] -ne '' -or $k -eq 'PULSO_PODMAN_MACHINE') { [Environment]::SetEnvironmentVariable($k, $fromFile[$k]) } } }
    @{ Values = $values; Sources = $sources }
}

# `podman` arguments that select the machine/connection: @('--connection', 'x') or @() (native podman / default connection).
# PULSO_PODMAN_CONNECTION wins; else PULSO_PODMAN_MACHINE (empty = default connection, name = '<name>-root'); else the old pulso-dev-root on
# Windows only (the original machine), nothing on Linux/macOS.
function Get-DevPodmanArgs {
    param($Environment = $null, [string]$OS = '')
    $os = Get-DevOS -Override $OS
    $conn = Get-DevEnvValue -Name 'PULSO_PODMAN_CONNECTION' -Environment $Environment
    if ($conn) { return @('--connection', $conn) }
    $m = Get-DevEnvValue -Name 'PULSO_PODMAN_MACHINE' -Environment $Environment
    if ($null -ne $m) {
        if ($m -eq '') { return @() }
        return @('--connection', ($m + '-root'))
    }
    if ($os -eq 'Windows') { return @('--connection', 'pulso-dev-root') }
    @()
}

# ---- memory ----------------------------------------------------------------------------------------------------------------------------

function ConvertFrom-ProcMeminfo {
    param([string]$Text)
    if ($Text -match '(?m)^MemAvailable:\s+(\d+)\s*kB') { return [int]([int64]$Matches[1] / 1024) }
    -1
}

function ConvertFrom-VmStat {
    param([string]$Text)
    if ($Text -notmatch 'page size of (\d+) bytes') { return -1 }
    $size = [int64]$Matches[1]
    $pages = 0; $found = $false
    foreach ($k in 'free', 'inactive', 'speculative') {
        if ($Text -match ('(?m)^Pages ' + $k + ':\s+(\d+)')) { $pages += [int64]$Matches[1]; $found = $true }
    }
    if (-not $found) { return -1 }
    [int]($pages * $size / 1MB)
}

function Read-DevMeminfo { [IO.File]::ReadAllText('/proc/meminfo') }
function Read-DevVmStat { (& vm_stat 2>$null) -join "`n" }

# Free (available) RAM in MB; -1 when it cannot be read. Windows: CIM; Linux: /proc/meminfo MemAvailable; macOS: vm_stat.
function Get-DevFreeRamMb {
    param([string]$OS = '')
    $os = Get-DevOS -Override $OS
    try {
        if ($os -eq 'Windows') { return [int]((Get-CimInstance Win32_OperatingSystem).FreePhysicalMemory / 1024) }
        if ($os -eq 'Linux') { return (ConvertFrom-ProcMeminfo -Text (Read-DevMeminfo)) }
        return (ConvertFrom-VmStat -Text (Read-DevVmStat))
    } catch { return -1 }
}

# ---- processes and ports ---------------------------------------------------------------------------------------------------------------

# Shell line run by a child process (stdin/stdout redirection): cmd.exe /c on Windows, /bin/sh -c elsewhere. '{NULL}' = the null device.
function Get-DevShellCommand {
    param([Parameter(Mandatory)][string]$Line, [string]$OS = '', [string]$ComSpec = '')
    $os = Get-DevOS -Override $OS
    $line = $Line.Replace('{NULL}', (Get-DevNullDevice -OS $os))
    if ($os -eq 'Windows') {
        if (-not $ComSpec) { $ComSpec = $env:ComSpec }
        return [pscustomobject]@{ FileName = $ComSpec; ArgumentString = ('/c "' + $line + '"'); ArgumentList = @() }
    }
    [pscustomobject]@{ FileName = '/bin/sh'; ArgumentString = ''; ArgumentList = @('-c', $line) }
}

# Applies Get-DevShellCommand to a ProcessStartInfo (Windows PowerShell has no ArgumentList, and never needs it).
function Set-DevShellCommand {
    param([Parameter(Mandatory)]$Psi, [Parameter(Mandatory)][string]$Line)
    $c = Get-DevShellCommand -Line $Line
    $Psi.FileName = $c.FileName
    if ($c.ArgumentList.Count -gt 0) { foreach ($a in $c.ArgumentList) { [void]$Psi.ArgumentList.Add($a) } } else { $Psi.Arguments = $c.ArgumentString }
}

# Runs a shell line to completion (redirections inside the line), output discarded by the line itself.
function Invoke-DevShellLine {
    param([Parameter(Mandatory)][string]$Line)
    $c = Get-DevShellCommand -Line $Line
    if ($c.ArgumentList.Count -gt 0) { & $c.FileName @($c.ArgumentList) } else { & $c.FileName $c.ArgumentString.Substring(0, 2) $c.ArgumentString.Substring(3) }
}

function Stop-DevProcessTree {
    param([int]$ProcessId)
    if ($ProcessId -le 0) { return }
    if ((Get-DevOS) -eq 'Windows') { try { & taskkill /PID $ProcessId /T /F 2>&1 | Out-Null } catch { }; return }
    try {
        foreach ($c in @(& pgrep -P $ProcessId 2>$null)) { if ($c -match '^\d+$') { Stop-DevProcessTree -ProcessId ([int]$c) } }
        & kill -TERM $ProcessId 2>&1 | Out-Null
    } catch { }
}

# $true when nothing listens on the loopback port.
function Test-DevPortFree {
    param([int]$Port)
    $l = $null
    try { $l = New-Object Net.Sockets.TcpListener([Net.IPAddress]::Loopback, $Port); $l.Start(); return $true }
    catch { return $false }
    finally { if ($l) { try { $l.Stop() } catch { } } }
}

# python / pwsh executable names differ: Linux and macOS usually have `python3` and no `python`.
function Get-DevPython {
    foreach ($n in 'python', 'python3') { $c = Get-Command $n -ErrorAction SilentlyContinue; if ($c) { return $c.Source } }
    throw 'python not found on PATH (python or python3)'
}

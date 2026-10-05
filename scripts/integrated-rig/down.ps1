<#
.SYNOPSIS
  Integrated rig: stops the platform API and this rig's own stack (prefix pulso-env1 containers, agent-core serve). Other lanes' stacks
  are never touched. -Purge also drops the Postgres volume, the gateway image tag and the local state of the stack.
  scripts/integrated-rig/down.ps1 [-Purge]
#>
[CmdletBinding()]
param([switch]$Purge)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$root = (Resolve-Path (Join-Path $here '../..')).Path
. (Join-Path $here 'rig.lib.ps1')

$factored = Split-Path -Parent (Split-Path -Parent $root)
$devCfg = Get-DevConfig -Root $root -Export -Legacy @{ PULSO_AGENT_CORE_DIR = (Join-Path $factored 'tmp/env1/agent-core'); PULSO_LLM_GATEWAY_DIR = (Join-Path $factored 'tmp/shared/llm-gateway') }
$settings = Get-RigSettings
$paths = Get-RigPaths -Root $root
[void](Stop-PidTree -PidFile $paths.SpaPid)
$stopped = Stop-PidTree -PidFile $paths.PlatformPid
Write-Host ("platform API: {0}" -f $(if ($stopped) { 'stopped' } else { 'was not running (no pid file)' }))
$agentCore = $devCfg.Values['PULSO_AGENT_CORE_DIR']
$gateway = $devCfg.Values['PULSO_LLM_GATEWAY_DIR']
$shell = Get-ChildShell
$envDown = Get-LoopLaneEnvironment -Settings $settings
$args2 = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', (Join-Path $root 'scripts/demo-loop/run.ps1'), '-Down', '-AgentCoreDir', $agentCore, '-GatewayDir', $gateway)
if ($Purge) { $args2 += '-Purge' }
$r = Invoke-Scrubbed -File $shell -Arguments $args2 -Env $envDown -WorkDir $root
if ($r.ExitCode -ne 0) { Write-Host "demo-loop -Down exited $($r.ExitCode)"; exit 1 }
foreach ($p in @($settings.PlatformPort, $settings.CorePort, $settings.GwPort, $settings.PgPort, $settings.EnginePort)) {
    $busy = -not (Test-DevPortFree -Port $p)
    if ($busy) { Write-Host "WARNING: port $p still has a listener" }
}
Write-Host ("rig down. free RAM {0} MB" -f (Get-FreeRamMb))
exit 0

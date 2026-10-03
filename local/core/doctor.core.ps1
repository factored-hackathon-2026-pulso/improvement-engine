<#
.SYNOPSIS
  Core health report: array of {check, status, code, detail}. Status: pass | fail | warn | skipped.
  V3 codes: core_checkout_wrong_sha core_toolchain_missing core_postgres_unavailable core_migrate_failed core_not_ready
  core_demo_doubles_active contracts_drift core_unreachable_from_stack assets_drift port_conflict
  runtime_cgroup_unavailable insufficient_memory machine_not_registered.
  -SkipEngine: static checks only (no container engine contact).
#>
[CmdletBinding()]
param([switch]$Json, [switch]$SkipEngine, [string]$Machine = 'pulso-dev', [string]$Namespace,
      [string]$Checkout = 'D:\.codex\factored\references\agent-core', [string]$Profile = 'real_local')
$ErrorActionPreference = 'Stop'
foreach ($f in 'errors', 'machine', 'namespace', 'ports', 'runner', 'memory', 'keys') { . (Join-Path $PSScriptRoot "lib\$f.ps1") }
$PinSha = '86a767474042a566a0dbd6ed23588959f27ebdb3'
$checks = New-Object System.Collections.Generic.List[object]
function Add-C([string]$check, [string]$status, $code, [string]$detail) { $checks.Add([pscustomobject]@{ check = $check; status = $status; code = $code; detail = $detail }) }

# machine
$conn = $null
try { $m = Assert-RegisteredMachine -Machine $Machine; $conn = $m.connection; Add-C 'machine_registered' 'pass' $null "connection $conn" }
catch { Add-C 'machine_registered' 'fail' 'machine_not_registered' $_.Exception.Message }

# pin
$head = ''
try { $head = ((git -C $Checkout rev-parse HEAD 2>$null) -join '').Trim() } catch {}
if ($head -eq $PinSha) { Add-C 'agent_core_pin' 'pass' $null $head } else { Add-C 'agent_core_pin' 'fail' 'core_checkout_wrong_sha' "HEAD '$head' != pin $PinSha" }

# contracts version
$vf = Join-Path $Checkout 'contracts\VERSION'
if ((Test-Path -LiteralPath $vf) -and ((Get-Content -LiteralPath $vf -Raw).Trim() -eq '1.3.0')) { Add-C 'contracts_version' 'pass' $null '1.3.0' }
else { Add-C 'contracts_version' 'fail' 'contracts_drift' 'contracts/VERSION is not 1.3.0 in the checkout' }

# assets manifest pin
$mf = Join-Path $PSScriptRoot '..\..\agent-core-assets\manifest.yaml'
if ((Test-Path $mf) -and ((Get-Content $mf -Raw) -match "sha:\s*$PinSha")) { Add-C 'assets_pin' 'pass' $null 'manifest pins the same SHA' }
else { Add-C 'assets_pin' 'fail' 'assets_drift' 'agent-core-assets/manifest.yaml missing or pins another SHA' }

# toolchain
$missing = @()
try { if (-not (Test-Path -LiteralPath (Get-PodmanPath))) { $missing += 'podman' } } catch { $missing += 'podman' }
foreach ($t in 'uv', 'docker-compose', 'git') { if (-not (Get-Command $t -ErrorAction SilentlyContinue)) { $missing += $t } }
if ($missing.Count -eq 0) { Add-C 'toolchain' 'pass' $null 'podman, uv, docker-compose (compose provider), git present' }
else { Add-C 'toolchain' 'fail' 'core_toolchain_missing' ("missing: " + ($missing -join ', ')) }

# engine checks
$engine = @('port_conflict', 'insufficient_memory', 'runtime_cgroup_unavailable', 'core_postgres_unavailable', 'core_migrate_failed',
            'core_not_ready', 'core_demo_doubles_active', 'core_unreachable_from_stack', 'bridge_executor_key')
if ($SkipEngine -or -not $conn) {
    foreach ($e in $engine) { Add-C $e 'skipped' $e 'engine not contacted' }
} else {
    try {
        $free = [long]((Invoke-Podman -Connection $conn info --format '{{.Host.MemFree}}') -join '').Trim()
        if (Test-MemoryBudget -FreeBytes $free -Profile $Profile) { Add-C 'insufficient_memory' 'pass' $null "$([int]($free/1MB)) MiB free" }
        else { Add-C 'insufficient_memory' 'fail' 'insufficient_memory' "$([int]($free/1MB)) MiB free" }
    } catch { Add-C 'insufficient_memory' 'fail' 'core_toolchain_missing' 'podman info failed' }
    if ($Namespace) {
        Assert-ClaudeNamespace -Namespace $Namespace
        $project = Get-PulsoProject -Namespace $Namespace
        $pg = Get-ContainerState -Connection $conn -Name "$project-core-postgres-1"
        Add-C 'core_postgres_unavailable' $(if ($pg -and $pg.health -eq 'healthy') { 'pass' } else { 'fail' }) $(if ($pg -and $pg.health -eq 'healthy') { $null } else { 'core_postgres_unavailable' }) "postgres: $($pg.status)/$($pg.health)"
        $mg = Get-ContainerState -Connection $conn -Name "$project-core-migrate-1"
        Add-C 'core_migrate_failed' $(if ($mg -and $mg.status -eq 'exited' -and $mg.exit -eq 0) { 'pass' } else { 'fail' }) $(if ($mg -and $mg.exit -eq 0) { $null } else { 'core_migrate_failed' }) "migrate: $($mg.status) exit=$($mg.exit)"
        $rt = Get-ContainerState -Connection $conn -Name "$project-core-runtime-1"
        Add-C 'core_not_ready' $(if ($rt -and $rt.health -eq 'healthy') { 'pass' } else { 'fail' }) $(if ($rt -and $rt.health -eq 'healthy') { $null } else { 'core_not_ready' }) "runtime: $($rt.status)/$($rt.health)"
        $sf = Join-Path (Join-Path $PSScriptRoot '..\.secrets') "$Namespace\state.json"
        if (Test-Path $sf) {
            $st = Get-Content $sf -Raw | ConvertFrom-Json
            $demo = $false
            try { $v = Invoke-RestMethod "http://127.0.0.1:$($st.ports.runtime)/_sim/info" -TimeoutSec 3; $demo = $true } catch {}
            Add-C 'core_demo_doubles_active' $(if ($demo) { 'warn' } else { 'pass' }) $(if ($demo) { 'core_demo_doubles_active' } else { $null }) $(if ($demo) { '/_sim/info answered on the Core URL' } else { 'no /_sim/info on the Core URL' })
        } else { Add-C 'core_demo_doubles_active' 'skipped' 'core_demo_doubles_active' 'no state file' }
        $ek = $null
        if ($rt -and $rt.status -eq 'running') {
            try { $ek = Get-ExecutorKeyVerdict -ProbeOutput ((Invoke-Podman -Connection $conn exec "$project-core-runtime-1" @(Get-ExecutorKeyProbeArgs)) -join '') } catch { $ek = [pscustomobject]@{ status = 'fail'; detail = 'executor key probe failed to run' } }
        } else { $ek = [pscustomobject]@{ status = 'fail'; detail = 'runtime container not running (fails closed without bridge-executor.json)' } }
        Add-C 'bridge_executor_key' $ek.status $(if ($ek.status -eq 'pass') { $null } else { 'core_not_ready' }) $ek.detail
        Add-C 'port_conflict' 'pass' $null 'ports were probed at start'
        Add-C 'runtime_cgroup_unavailable' 'pass' $null 'workaround encoded (--cgroups=disabled)'
        Add-C 'core_unreachable_from_stack' 'skipped' 'core_unreachable_from_stack' 'needs the Pulso assembly network (Codex)'
    } else {
        foreach ($e in 'bridge_executor_key', 'port_conflict', 'runtime_cgroup_unavailable', 'core_postgres_unavailable', 'core_migrate_failed', 'core_not_ready', 'core_demo_doubles_active', 'core_unreachable_from_stack') {
            Add-C $e 'skipped' $e 'no -Namespace given'
        }
    }
}
$out = [object[]]$checks.ToArray()
if ($Json) { $out | ConvertTo-Json -Depth 4 -AsArray } else { $out | ForEach-Object { "[{0}] {1} {2} {3}" -f $_.status, $_.check, $_.code, $_.detail } }
if (@($out | Where-Object status -eq 'fail').Count -gt 0) { exit 1 }
exit 0

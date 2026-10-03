<#
.SYNOPSIS
  Starts the standalone Core stack on a registered Claude machine (plan 17.3.8 / 17.9.1).
  start.ps1 [-Namespace claude-<id>] -Profile real_local|fixture [-Machine pulso-dev] [-PortBase N] [-Image ref] [-DryRun]
  -DryRun renders and prints the plan (ports, project, commands) without touching the container engine.
  Exit codes: 2 machine_not_registered/namespace_not_owned, 3 insufficient_memory, 4 port_conflict, 5 core_not_ready,
  6 core_migrate_failed, 7 core_postgres_unavailable, 8 runtime_cgroup_unavailable, 9 core_seed_failed.
#>
[CmdletBinding()]
param(
    [string]$Namespace,
    [ValidateSet('real_local', 'fixture')][string]$Profile = 'real_local',
    [string]$Machine = 'pulso-dev',
    [int]$PortBase = 0,
    [string]$Image,
    [string]$SecretsRoot,
    [switch]$DryRun
)
$ErrorActionPreference = 'Stop'
$lib = Join-Path $PSScriptRoot 'lib'
foreach ($f in 'errors', 'machine', 'namespace', 'ports', 'evidence', 'secrets', 'runner', 'memory', 'humanissuer') { . (Join-Path $lib "$f.ps1") }
if (-not $SecretsRoot) { $SecretsRoot = Join-Path $PSScriptRoot '..\.secrets' }
try {
    if (-not $Namespace) { $Namespace = New-PulsoNamespace }
    Assert-ClaudeNamespace -Namespace $Namespace
    $m = Assert-RegisteredMachine -Machine $Machine
    $conn = $m.connection
    $project = Get-PulsoProject -Namespace $Namespace

    if (-not $DryRun) {
        $existing = @(Get-ExistingProjects -Connection $conn)
        Assert-NamespaceOwnership -Namespace $Namespace -Existing $existing
        Assert-MemoryBudget -Connection $conn -Profile $Profile
    }
    $services = @('postgres', 'runtime', 'exporter', 'sim-registry', 'sim-bridge', 'sim-ingest')
    $inUse = if ($DryRun) { @(Get-PortsInUse) } else { @(Get-PortsInUse -Connection $conn) }
    # Restarting the same namespace keeps its own ports (they are not a conflict with itself).
    $prevState = Join-Path (Join-Path $SecretsRoot $Namespace) 'state.json'
    if (-not $DryRun -and $PortBase -le 0 -and (Test-Path -LiteralPath $prevState)) {
        $prev = Get-Content -LiteralPath $prevState -Raw | ConvertFrom-Json
        $PortBase = [int]$prev.ports.postgres
        $inUse = @($inUse | Where-Object { $_ -notin @($prev.ports.PSObject.Properties.Value) })
    }
    $plan = Resolve-PortPlan -Namespace $Namespace -Services $services -PortBase $PortBase -InUse $inUse

    $digest = 'unknown'
    if (-not $Image) {
        if ($DryRun) { $Image = 'localhost/pulso-core-runtime:dry-run' } else {
            $Image = Resolve-RuntimeImage -Connection $conn
        }
    }
    if (-not $DryRun) { $digest = 'sha256:' + (Invoke-Podman -Connection $conn image inspect $Image --format '{{.Id}}').Trim() }
    $simImage = 'localhost/pulso-platform-sim:dry-run'
    if (-not $DryRun) { $simImage = Ensure-SimImage -Connection $conn -BaseImage $Image }
    # Local human issuer (sandbox-only double): its image is content-addressed and built here; keys are generated INSIDE the
    # stack (human-issuer-keygen) so no private key ever touches the host or the repo.
    $humanImage = 'localhost/pulso-local-identity:dry-run'
    if (-not $DryRun) { $humanImage = Ensure-HumanIssuerImage -Connection $conn }
    $coreEnv = Initialize-LocalSecrets -Root $SecretsRoot -Namespace $Namespace
    $portsEnv = Write-PortsEnv -Root $SecretsRoot -Namespace $Namespace -Ports $plan -Image $Image -ImageDigest $digest -SimImage $simImage -HumanIssuerImage $humanImage

    $model = Get-ComposeModel -Namespace $Namespace -Profile $Profile -EnvFiles @($coreEnv, $portsEnv)
    $runtimeProfile = if ($Profile -eq 'fixture') { 'contract_mock' } else { 'from /internal/v1/version' }
    $doubles = if ($Profile -eq 'fixture') { @('platform-sim', 'core-synth') } else { @('platform-sim', 'human-issuer') }

    Write-Output "namespace=$Namespace project=$project profile=$Profile machine=$($m.name)"
    Write-Output "runtime_profile=$runtimeProfile"
    Write-Output ("doubles: " + ($doubles -join ', '))
    Write-Output "engine: podman --connection $conn (cgroups=disabled, pids-limit=0 via runner labels)"
    foreach ($k in $plan.Keys) { Write-Output "publish 127.0.0.1:$($plan[$k]) -> $k" }
    foreach ($svc in (Get-StartOrder -Model $model)) { Write-Output "service $svc" }

    if ($DryRun) { Write-Output 'start: dry-run ok (nothing started)'; exit 0 }

    Start-CoreStack -Model $model -Connection $conn -Project $project -Namespace $Namespace
    $state = [ordered]@{ namespace = $Namespace; project = $project; profile = $Profile; machine = $m.name; connection = $conn
                         ports = $plan; image = $Image; expected_image_digest = $digest; started_at = [DateTime]::UtcNow.ToString('o') }
    $state | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path (Split-Path $coreEnv -Parent) 'state.json') -Encoding utf8
    Write-Output "start: ok runtime=http://127.0.0.1:$($plan['runtime'])"
    exit 0
} catch { Exit-Pulso $_ }

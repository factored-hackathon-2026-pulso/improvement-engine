# Preflight: free memory vs steady use + 25% (plan 17.3.8). No silent profile downgrade.
$script:SteadyMiB = @{ real_local = 1536; fixture = 700 }

function Test-MemoryBudget {
    param([long]$FreeBytes, [Parameter(Mandatory)][string]$Profile)
    $needMiB = [math]::Ceiling($script:SteadyMiB[$Profile] * 1.25)
    ($FreeBytes / 1MB) -ge $needMiB
}

function Assert-MemoryBudget {
    param([Parameter(Mandatory)][string]$Connection, [Parameter(Mandatory)][string]$Profile)
    $free = [long]((Invoke-Podman -Connection $Connection info --format '{{.Host.MemFree}}') -join '').Trim()
    if (-not (Test-MemoryBudget -FreeBytes $free -Profile $Profile)) {
        throw "pulso:insufficient_memory: free $([int]($free / 1MB)) MiB below steady use + 25% for profile $Profile"
    }
}

# Projects (and their owning team label) already on the engine: used to refuse another owner's namespace.
function Get-ExistingProjects {
    param([Parameter(Mandatory)][string]$Connection)
    $rows = Invoke-Podman -Connection $Connection ps -a --format '{{.Label "com.docker.compose.project"}}|{{.Label "com.pulso.team"}}|{{.Label "com.pulso.namespace"}}' 2>$null
    foreach ($r in @($rows)) {
        $p = ([string]$r).Split('|')
        if ($p.Count -ge 3 -and $p[0]) { [pscustomobject]@{ project = $p[0]; team = $p[1]; namespace = $p[2] } }
    }
}

function Resolve-RuntimeImage {
    param([Parameter(Mandatory)][string]$Connection)
    $rows = @(Invoke-Podman -Connection $Connection images --format '{{.Repository}}:{{.Tag}}' --filter 'reference=localhost/pulso-core-runtime' 2>$null)
    if ($rows.Count -eq 0) { throw 'pulso:core_not_ready: no localhost/pulso-core-runtime image; run core-bridge/scripts/build-image.ps1 first' }
    [string]$rows[0]
}

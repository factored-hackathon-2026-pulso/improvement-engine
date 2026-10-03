# Machine/connection registry assertions (plan 17.9.1). Every podman call passes --connection explicitly.
$script:CoreRoot = (Resolve-Path (Join-Path (Split-Path -Parent $MyInvocation.MyCommand.Path) '..')).Path

function Get-PulsoMachineRegistry {
    param([string]$Path = (Join-Path $script:CoreRoot 'machine.registry.json'))
    Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json
}

function Assert-RegisteredMachine {
    [CmdletBinding()]
    param([string]$Machine, [string]$RegistryPath)
    $reg = if ($RegistryPath) { Get-PulsoMachineRegistry -Path $RegistryPath } else { Get-PulsoMachineRegistry }
    $hit = @($reg.machines | Where-Object { $_.name -eq $Machine })
    if (-not $Machine -or $hit.Count -ne 1) {
        throw "pulso:machine_not_registered: '$Machine' is not declared in local/core/machine.registry.json"
    }
    $hit[0]
}

function Get-PodmanPath {
    if ($env:PULSO_PODMAN) { return $env:PULSO_PODMAN }
    (Get-PulsoMachineRegistry).podman
}

# Single choke point for podman: always an explicit --connection (never the global default).
function Invoke-Podman {
    param([Parameter(Mandatory)][string]$Connection, [Parameter(ValueFromRemainingArguments)][string[]]$PodmanArgs)
    & (Get-PodmanPath) --connection $Connection @PodmanArgs
}

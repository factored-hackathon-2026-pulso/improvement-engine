<#
.SYNOPSIS  Stops (does not remove) the containers of a Claude namespace. Idle stacks are stopped, not removed (plan 17.3.8).
#>
[CmdletBinding()]
param([Parameter(Mandatory)][string]$Namespace, [string]$Machine = 'pulso-dev', [switch]$DryRun)
$ErrorActionPreference = 'Stop'
foreach ($f in 'errors', 'machine', 'namespace') { . (Join-Path $PSScriptRoot "lib\$f.ps1") }
try {
    Assert-ClaudeNamespace -Namespace $Namespace
    $conn = (Assert-RegisteredMachine -Machine $Machine).connection
    $filters = @('--filter', "label=com.pulso.namespace=$Namespace", '--filter', 'label=com.pulso.team=claude')
    Write-Output "stop: podman --connection $conn ps $($filters -join ' ')"
    if ($DryRun) { exit 0 }
    $ids = @(Invoke-Podman -Connection $conn ps -q @filters)
    if ($ids.Count -gt 0) { Invoke-Podman -Connection $conn stop @ids | Out-Null }
    Write-Output "stop: ok ($($ids.Count) containers)"
    exit 0
} catch { Exit-Pulso $_ }

<#
.SYNOPSIS
  The only destructive command: removes containers, volumes and network of ONE Claude namespace, selected by the
  labels com.pulso.team=claude AND com.pulso.namespace=<ns> (never by name pattern or id list). Requires -Confirm.
#>
[CmdletBinding()]
param([Parameter(Mandatory)][string]$Namespace, [string]$Machine = 'pulso-dev', [switch]$Confirm, [switch]$DryRun,
      [switch]$KeepSecrets)
$ErrorActionPreference = 'Stop'
foreach ($f in 'errors', 'machine', 'namespace') { . (Join-Path $PSScriptRoot "lib\$f.ps1") }
try {
    Assert-ClaudeNamespace -Namespace $Namespace
    if (-not $Confirm) { throw 'pulso:confirm_required: reset is destructive; pass -Confirm' }
    $conn = (Assert-RegisteredMachine -Machine $Machine).connection
    $f = @('--filter', "label=com.pulso.namespace=$Namespace", '--filter', 'label=com.pulso.team=claude')
    Write-Output "reset: scope label=com.pulso.namespace=$Namespace label=com.pulso.team=claude on connection $conn"
    if ($DryRun) { Write-Output 'reset: dry-run (nothing removed)'; exit 0 }
    $ids = @(Invoke-Podman -Connection $conn ps -aq @f)
    if ($ids.Count -gt 0) { Invoke-Podman -Connection $conn rm -f @ids | Out-Null }
    $vols = @(Invoke-Podman -Connection $conn volume ls -q @f)
    if ($vols.Count -gt 0) { Invoke-Podman -Connection $conn volume rm -f @vols | Out-Null }
    $nets = @(Invoke-Podman -Connection $conn network ls -q @f)
    if ($nets.Count -gt 0) { Invoke-Podman -Connection $conn network rm -f @nets | Out-Null }
    if (-not $KeepSecrets) {
        $dir = Join-Path (Join-Path $PSScriptRoot '..\.secrets') $Namespace
        if (Test-Path -LiteralPath $dir) { [IO.Directory]::Delete((Resolve-Path $dir).Path, $true) }
    }
    Write-Output "reset: ok containers=$($ids.Count) volumes=$($vols.Count) networks=$($nets.Count)"
    exit 0
} catch { Exit-Pulso $_ }

<#
.SYNOPSIS
  One-command dev stack (IN0). Thin wrapper over local/core (start/stop/reset/doctor): it adds a fixed namespace and
  the doctor aggregate (scripts/env/dev-doctor.ps1); it does not duplicate stack logic.
  scripts/dev.ps1 up|doctor|down|reset [-Profile fixture|real_local] [-Namespace claude-dev] [-Machine pulso-dev] [-DryRun]
  doctor: every compose-model service must be running+healthy (one-shots exited 0) and the engine doctor must have no failed
  check; otherwise exit 1 listing "<service>: <reason>". Offline test hooks: -StatesFile (observed states JSON) with
  -ExpectedServices name[:oneshot],... avoid any container engine contact.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory, Position = 0)][ValidateSet('up', 'doctor', 'down', 'reset')][string]$Action,
    [ValidateSet('real_local', 'fixture')][string]$Profile = 'fixture',
    [string]$Namespace = 'claude-dev',
    [string]$Machine = 'pulso-dev',
    [switch]$DryRun,
    [string]$StatesFile,
    [string]$ExpectedServices
)
$ErrorActionPreference = 'Stop'
$core = Join-Path $PSScriptRoot '..\local\core'
foreach ($f in 'errors', 'machine', 'namespace', 'runner') { . (Join-Path $core "lib\$f.ps1") }
. (Join-Path $PSScriptRoot 'env\dev-doctor.ps1')
$ps = (Get-Process -Id $PID).Path
function Invoke-Core([string]$script, [string[]]$a) { & $ps -NoProfile -File (Join-Path $core $script) @a | Out-Host; return $LASTEXITCODE }
try {
    Assert-ClaudeNamespace -Namespace $Namespace
    $conn = (Assert-RegisteredMachine -Machine $Machine).connection
    $project = Get-PulsoProject -Namespace $Namespace
    switch ($Action) {
        'up' {
            $a = @('-Namespace', $Namespace, '-Profile', $Profile, '-Machine', $Machine)
            if ($DryRun) { $a += @('-DryRun', '-SecretsRoot', (Join-Path $env:TEMP 'pulso-dev-dryrun')) }
            exit (Invoke-Core 'start.ps1' $a)
        }
        'down' { exit (Invoke-Core 'stop.ps1' @('-Namespace', $Namespace, '-Machine', $Machine)) }
        'reset' { exit (Invoke-Core 'reset.ps1' @('-Namespace', $Namespace, '-Machine', $Machine, '-Confirm')) }
        'doctor' {
            $coreChecks = @(); $skipped = @()
            if ($StatesFile) {
                $expected = @($ExpectedServices.Split(',') | ForEach-Object { $p = $_.Split(':'); [pscustomobject]@{ name = $p[0]; oneShot = ($p.Count -gt 1 -and $p[1] -eq 'oneshot') } })
                $raw = Get-Content -LiteralPath $StatesFile -Raw | ConvertFrom-Json
                $states = @{}; foreach ($e in $expected) { $states[$e.name] = $raw.($e.name) }
            } else {
                $model = Get-ComposeModel -Namespace $Namespace -Profile $Profile
                $expected = @($model.services.PSObject.Properties | ForEach-Object { [pscustomobject]@{ name = $_.Name; oneShot = (Test-DevOneShot -Service $_.Value) } })
                $states = @{}; foreach ($e in $expected) { $states[$e.name] = Get-ContainerState -Connection $conn -Name "$project-$($e.name)-1" }
                $json = & $ps -NoProfile -File (Join-Path $core 'doctor.core.ps1') -Json -Namespace $Namespace -Machine $Machine -Profile $Profile 2>$null | Out-String
                try { $all = @($json | ConvertFrom-Json); $skipped = @(Get-DevSkippedChecks -Checks $all -Profile $Profile); $coreChecks = @(Select-DevCoreChecks -Checks @($json | ConvertFrom-Json) -Profile $Profile) } catch { $coreChecks = @([pscustomobject]@{ check = 'engine_doctor'; status = 'fail'; code = 'engine_doctor_unreadable'; detail = 'doctor.core.ps1 gave no JSON' }) }
            }
            Write-Output "profile: $Profile (real_local not verified by this doctor run)"
            if ($skipped.Count) { Write-Output "skipped (not applicable to $Profile): $($skipped -join ', ')" }
            $v = Get-DevDoctorVerdict -Expected $expected -States $states -CoreChecks $coreChecks
            if ($v.ok) { Write-Output "doctor: green ($(@($expected).Count) services, namespace $Namespace)"; exit 0 }
            foreach ($f in $v.failures) { Write-Output "$($f.service): $($f.reason) ($($f.detail))" }
            Write-Output "doctor: red ($(@($v.failures).Count) failure(s))"; exit 1
        }
    }
} catch { Exit-Pulso $_ }

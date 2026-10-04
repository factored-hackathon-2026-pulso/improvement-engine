# Pester 3.x (host version). Offline: the aggregate is a pure function over observed container states.
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
. (Join-Path $here '..\dev-doctor.ps1')

function New-State($status, $exit = 0, $health = '') { [pscustomobject]@{ status = $status; exit = $exit; health = $health } }
$expected = @(
    [pscustomobject]@{ name = 'core-postgres'; oneShot = $false }
    [pscustomobject]@{ name = 'core-migrate'; oneShot = $true }
    [pscustomobject]@{ name = 'core-runtime'; oneShot = $false }
)
function Healthy { @{ 'core-postgres' = (New-State 'running' 0 'healthy'); 'core-migrate' = (New-State 'exited' 0); 'core-runtime' = (New-State 'running' 0 'healthy') } }

Describe 'Get-DevDoctorVerdict' {
    It 'passes when every long-running service is healthy and every one-shot exited 0' {
        $v = Get-DevDoctorVerdict -Expected $expected -States (Healthy)
        $v.ok | Should Be $true
        @($v.failures).Count | Should Be 0
    }
    It 'fails naming service_stopped for a stopped service' {
        $s = Healthy; $s['core-postgres'] = New-State 'exited' 137
        $v = Get-DevDoctorVerdict -Expected $expected -States $s
        $v.ok | Should Be $false
        $f = @($v.failures)[0]
        $f.service | Should Be 'core-postgres'
        $f.reason | Should Be 'service_stopped'
    }
    It 'fails naming service_missing when the container does not exist' {
        $s = Healthy; $s['core-runtime'] = $null
        $v = Get-DevDoctorVerdict -Expected $expected -States $s
        (@($v.failures) | Where-Object service -eq 'core-runtime').reason | Should Be 'service_missing'
    }
    It 'fails naming service_unhealthy for a running but unhealthy service' {
        $s = Healthy; $s['core-runtime'] = New-State 'running' 0 'unhealthy'
        $v = Get-DevDoctorVerdict -Expected $expected -States $s
        (@($v.failures) | Where-Object service -eq 'core-runtime').reason | Should Be 'service_unhealthy'
    }
    It 'fails naming oneshot_failed for a one-shot with a non-zero exit' {
        $s = Healthy; $s['core-migrate'] = New-State 'exited' 1
        $v = Get-DevDoctorVerdict -Expected $expected -States $s
        (@($v.failures) | Where-Object service -eq 'core-migrate').reason | Should Be 'oneshot_failed'
    }
    It 'reports every failing service, not only the first' {
        $s = Healthy; $s['core-postgres'] = New-State 'exited' 1; $s['core-runtime'] = $null
        @((Get-DevDoctorVerdict -Expected $expected -States $s).failures).Count | Should Be 2
    }
    It 'fails when the engine doctor reports a failed check, naming its code' {
        $core = @([pscustomobject]@{ check = 'insufficient_memory'; status = 'fail'; code = 'insufficient_memory'; detail = 'low' })
        $v = Get-DevDoctorVerdict -Expected $expected -States (Healthy) -CoreChecks $core
        $v.ok | Should Be $false
        @($v.failures)[0].reason | Should Be 'insufficient_memory'
    }
    It 'ignores skipped and warn engine checks' {
        $core = @([pscustomobject]@{ check = 'x'; status = 'skipped'; code = 'x'; detail = '' }, [pscustomobject]@{ check = 'y'; status = 'warn'; code = 'y'; detail = '' })
        (Get-DevDoctorVerdict -Expected $expected -States (Healthy) -CoreChecks $core).ok | Should Be $true
    }
    It 'fails when nothing is expected (an empty stack is not green)' {
        (Get-DevDoctorVerdict -Expected @() -States @{}).ok | Should Be $false
    }
}

Describe 'Select-DevCoreChecks' {
    $all = @('core_not_ready', 'human_issuer_ready', 'human_issuer_internal_only', 'bridge_executor_key', 'core_demo_doubles_active', 'core_unreachable_from_stack', 'insufficient_memory', 'core_postgres_unavailable') |
        ForEach-Object { [pscustomobject]@{ check = $_; status = 'fail'; code = $_; detail = '' } }
    It 'fixture drops runtime-and-issuer checks that the fixture stack does not run' {
        $names = @(Select-DevCoreChecks -Checks $all -Profile 'fixture') | ForEach-Object check
        $names -contains 'core_not_ready' | Should Be $false
        $names -contains 'human_issuer_ready' | Should Be $false
        $names -contains 'bridge_executor_key' | Should Be $false
        $names -contains 'insufficient_memory' | Should Be $true
        $names -contains 'core_postgres_unavailable' | Should Be $true
    }
    It 'real_local keeps every check' {
        @(Select-DevCoreChecks -Checks $all -Profile 'real_local').Count | Should Be 8
    }
}
Describe 'Test-DevOneShot' {
    It 'a restart-no service without a healthcheck is one-shot; with a healthcheck it is long-running' {
        Test-DevOneShot -Service ([pscustomobject]@{ restart = 'no' }) | Should Be $true
        Test-DevOneShot -Service ([pscustomobject]@{ restart = 'no'; healthcheck = [pscustomobject]@{ test = 'x' } }) | Should Be $false
        Test-DevOneShot -Service ([pscustomobject]@{ restart = 'unless-stopped' }) | Should Be $false
    }
}

Describe 'Get-DevSkippedChecks' {
    It 'names the engine checks the fixture profile does not apply, so they are reported as skipped not passed' {
        $all = @('core_not_ready', 'insufficient_memory') | ForEach-Object { [pscustomobject]@{ check = $_; status = 'pass' } }
        @(Get-DevSkippedChecks -Checks $all -Profile 'fixture') | Should Be @('core_not_ready')
        @(Get-DevSkippedChecks -Checks $all -Profile 'real_local').Count | Should Be 0
    }
}

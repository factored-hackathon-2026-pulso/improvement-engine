# Dev doctor aggregate (IN0): pure verdict over observed container states plus the engine doctor's checks.
# Expected: @{ name; oneShot }. States: hashtable name -> @{ status; exit; health } or $null (container absent).
# Reasons: service_missing, service_stopped, service_unhealthy, oneshot_failed, stack_empty, or an engine check code.
function Get-DevDoctorVerdict {
    param([object[]]$Expected = @(), [hashtable]$States = @{}, [object[]]$CoreChecks = @())
    $failures = New-Object System.Collections.Generic.List[object]
    function Add-F($service, $reason, $detail) { $failures.Add([pscustomobject]@{ service = $service; reason = $reason; detail = $detail }) }
    if (@($Expected).Count -eq 0) { Add-F '*' 'stack_empty' 'no expected services' }
    foreach ($e in @($Expected)) {
        $st = $States[$e.name]
        if (-not $st) { Add-F $e.name 'service_missing' 'container does not exist'; continue }
        if ($e.oneShot) {
            if ($st.status -ne 'exited' -or $st.exit -ne 0) { Add-F $e.name 'oneshot_failed' "status=$($st.status) exit=$($st.exit)" }
        } elseif ($st.status -ne 'running') { Add-F $e.name 'service_stopped' "status=$($st.status) exit=$($st.exit)" }
        elseif ($st.health -and $st.health -ne 'healthy') { Add-F $e.name 'service_unhealthy' "health=$($st.health)" }
    }
    foreach ($c in @($CoreChecks)) { if ($c.status -eq 'fail') { Add-F $c.check $(if ($c.code) { $c.code } else { 'check_failed' }) $c.detail } }
    [pscustomobject]@{ ok = ($failures.Count -eq 0); failures = [object[]]$failures.ToArray() }
}

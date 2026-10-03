# Live stack on the registered Podman machine. Opt-in: `$env:PULSO_LIVE='1'`. Needs a built runtime image
# (core-bridge/scripts/build-image.ps1) in $env:PULSO_LIVE_IMAGE. Creates and resets its own claude-<ms> namespace.
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$core = (Resolve-Path (Join-Path $here '..')).Path
$ps = (Get-Process -Id $PID).Path
. (Join-Path $core 'lib\machine.ps1')

function Run($script, [string[]]$a) { $o = & $ps -NoProfile -File (Join-Path $core $script) @a 2>&1 | Out-String; [pscustomobject]@{ code = $LASTEXITCODE; out = $o } }

if ($env:PULSO_LIVE -ne '1') {
    Describe 'live stack (skipped: set PULSO_LIVE=1)' { It 'is not run by default' -Skip { } }
    return
}
$ns = 'claude-' + [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
$img = $env:PULSO_LIVE_IMAGE

Describe 'live real_local stack' {
    It 'clean clone start reaches ready' { (Run 'start.ps1' @('-Namespace', $ns, '-Profile', 'real_local', '-Image', $img)).code | Should Be 0 }
    It 'smoke passes and target is derived real_local' {
        $r = Run 'smoke.ps1' @('-Namespace', $ns, '-ReportPath', (Join-Path $env:TEMP "$ns-report.json"))
        $r.code | Should Be 0
        $r.out | Should Match 'target=real_local'
    }
    It 'second start: seed writes nothing' {
        $conn = (Assert-RegisteredMachine -Machine 'pulso-dev').connection
        $q = 'select concat_ws(''|'', (select count(*) from reg_releases), (select count(*) from reg_events), (select seeded_at from pulso_seed_state))'
        $before = (Invoke-Podman -Connection $conn exec "pulso-$ns-core-postgres-1" psql -U postgres -d core_runtime -Atc $q) -join ''
        (Run 'start.ps1' @('-Namespace', $ns, '-Profile', 'real_local', '-Image', $img)).code | Should Be 0
        $after = (Invoke-Podman -Connection $conn exec "pulso-$ns-core-postgres-1" psql -U postgres -d core_runtime -Atc $q) -join ''
        $before | Should Match '^5\|5\|'
        $after | Should Be $before
        ((Invoke-Podman -Connection $conn logs "pulso-$ns-core-seed-1") -join ' ') | Should Match 'unchanged'
    }
    It 'refuses to start the namespace on a port base held by another listener' {
        $l = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Parse('127.0.0.1'), 18851); $l.Start()
        try { (Run 'start.ps1' @('-Namespace', "$ns-pc", '-Profile', 'fixture', '-PortBase', '18850', '-Image', $img)).code | Should Be 4 } finally { $l.Stop() }
    }
    It 'reset removes only this namespace' {
        (Run 'reset.ps1' @('-Namespace', $ns, '-Confirm')).code | Should Be 0
        $conn = (Assert-RegisteredMachine -Machine 'pulso-dev').connection
        @(Invoke-Podman -Connection $conn ps -aq --filter "label=com.pulso.namespace=$ns").Count | Should Be 0
    }
}

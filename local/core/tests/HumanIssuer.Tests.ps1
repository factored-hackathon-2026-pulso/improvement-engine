# local-identity as a Core-stack service (plan 17.3.2 / 16.13.2): internal only, local profile only, doubles[] honest.
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$core = (Resolve-Path (Join-Path $here '..')).Path
. (Join-Path $core 'lib\keys.ps1')
. (Join-Path $core 'lib\evidence.ps1')
. (Join-Path $core 'lib\humanissuer.ps1')

function Get-Model {
    $env:PULSO_NS = 'claude-model'; $env:PULSO_PROJECT = 'pulso-claude-model'
    $a = @('-f', (Join-Path $core 'compose.core.yaml'), '-f', (Join-Path $core 'compose.standalone.yaml'),
        '--profile', 'real_local', '--profile', 'fixture', 'config', '--format', 'json')
    (& docker-compose @a 2>&1 | Out-String) | ConvertFrom-Json
}
$m = Get-Model
$h = $m.services.'human-issuer'

Describe 'human-issuer service' {
    It 'exists, is a labelled double and answers to the human-issuer alias on the core network' {
        $h | Should Not BeNullOrEmpty
        $h.labels.'com.pulso.role' | Should Be 'double'
        $h.labels.'com.pulso.team' | Should Be 'claude'
        (@($h.networks.'core-net'.aliases) -contains 'human-issuer') | Should Be $true
    }
    It 'is internal only: no host publication and port 8083 only exposed' {
        @($h.ports | Where-Object { $_ }).Count | Should Be 0
        (@($h.expose) -contains '8083') | Should Be $true
    }
    It 'runs read-only, non-root, with the local profile switch set by compose and keys on read-only volumes' {
        $h.read_only | Should Be $true
        "$($h.user)" | Should Be '10002'
        $h.environment.LOCAL_IDENTITY_PROFILE | Should Be 'local'
        foreach ($v in @($h.volumes | Where-Object { $_.target -eq '/run/hi-keys' })) { $v.read_only | Should Be $true }
    }
    It 'is part of the real_local profile only (the fixture profile has no runtime to approve against)' {
        @($h.profiles) | Should Be @('real_local')
    }
    It 'starts after its own keygen, which never shares a volume with Core private keys' {
        $h.depends_on.'human-issuer-keygen'.condition | Should Be 'service_completed_successfully'
        $kg = $m.services.'human-issuer-keygen'
        (@($kg.volumes | ForEach-Object { $_.source }) -contains 'core-keys') | Should Be $false
    }
}

Describe 'Core receives only the PUBLIC human key' {
    It 'core-keygen mounts the public volume read-only and never the private issuer volume' {
        $kg = $m.services.'core-keygen'
        $pub = @($kg.volumes | Where-Object { $_.source -eq 'human-issuer-public' })
        $pub.Count | Should Be 1
        $pub[0].read_only | Should Be $true
        (@($kg.volumes | ForEach-Object { $_.source }) -contains 'human-issuer-keys') | Should Be $false
        $kg.depends_on.'human-issuer-keygen'.condition | Should Be 'service_completed_successfully'
    }
    It 'the runtime and the exporter never mount the human issuer volumes' {
        foreach ($n in 'core-runtime', 'core-exporter', 'platform-sim') {
            foreach ($v in @($m.services.$n.volumes)) { ($v.source -like 'human-issuer*') | Should Be $false }
        }
    }
}

Describe 'doubles honesty and CAP-63' {
    It 'Merge-Doubles reports the human-issuer container' {
        (@(Merge-Doubles -Runtime @() -Containers @('human-issuer')) -contains 'container:human-issuer') | Should Be $true
    }
    It 'Get-RemoteSimulationVerdict passes on a clean tree and fails on a planted local-sim kid' {
        $tmp = Join-Path ([IO.Path]::GetTempPath()) ('cap63-' + [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Path (Join-Path $tmp 'deploy') | Out-Null
        try {
            Set-Content (Join-Path $tmp 'deploy\staging.json') '{"kid":"ok"}'
            (Get-RemoteSimulationVerdict -Root $tmp).status | Should Be 'pass'
            Set-Content (Join-Path $tmp 'deploy\prod.json') '{"kid":"local-sim-human-1","auth":{"simulated":true}}'
            $v = Get-RemoteSimulationVerdict -Root $tmp
            $v.status | Should Be 'fail'
            $v.detail | Should Match 'prod.json'
        } finally { Remove-Item -Recurse -Force $tmp }
    }
    It 'Get-HumanIssuerExposureVerdict fails when the container publishes a host port' {
        (Get-HumanIssuerExposureVerdict -PublishedPorts '').status | Should Be 'pass'
        (Get-HumanIssuerExposureVerdict -PublishedPorts '127.0.0.1:18999->8083/tcp').status | Should Be 'fail'
    }
}

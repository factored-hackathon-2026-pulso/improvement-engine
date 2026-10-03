$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$core = (Resolve-Path (Join-Path $here '..')).Path

function Get-Model {
    $env:PULSO_NS = 'claude-model'; $env:PULSO_PROJECT = 'pulso-claude-model'
    $env:PULSO_ENV_FILE = (Join-Path $core 'profiles\real_local.env.example')
    $a = @('-f', (Join-Path $core 'compose.core.yaml'), '-f', (Join-Path $core 'compose.standalone.yaml'),
        '--profile', 'real_local', '--profile', 'fixture', 'config', '--format', 'json')
    (& docker-compose @a 2>&1 | Out-String) | ConvertFrom-Json
}
$m = Get-Model

Describe 'compose model' {
    It 'renders (docker-compose config)' { $m | Should Not BeNullOrEmpty }
    It 'has no container_name anywhere' {
        foreach ($n in $m.services.PSObject.Properties.Name) { ($m.services.$n.PSObject.Properties.Name -contains 'container_name') | Should Be $false }
    }
    It 'labels every service com.pulso.team=claude' {
        foreach ($n in $m.services.PSObject.Properties.Name) { $m.services.$n.labels.'com.pulso.team' | Should Be 'claude' }
    }
    It 'labels doubles com.pulso.role=double' {
        $m.services.'platform-sim'.labels.'com.pulso.role' | Should Be 'double'
    }
    It 'publishes only on 127.0.0.1 and never fixed 5432/8000' {
        foreach ($n in $m.services.PSObject.Properties.Name) {
            foreach ($p in @($m.services.$n.ports)) { if ($p) { $p.host_ip | Should Be '127.0.0.1'; ($p.published -in '5432', '8000') | Should Be $false } }
        }
    }
    It 'carries the cgroup workaround labels on every service (cgroups=disabled, pids_limit 0)' {
        foreach ($n in $m.services.PSObject.Properties.Name) {
            $m.services.$n.labels.'com.pulso.runner.cgroups' | Should Be 'disabled'
            $m.services.$n.labels.'com.pulso.runner.pids_limit' | Should Be '0'
        }
    }
    It 'scopes DSN env names: runtime gets registry/eval, exporter only its own, no demo flag' {
        $rt = $m.services.'core-runtime'.environment.PSObject.Properties.Name
        ($rt -contains 'AGENTCORE_REGISTRY_DSN') | Should Be $true
        ($rt -contains 'AGENTCORE_EVAL_DSN') | Should Be $true
        ($rt -contains 'CORE_EXPORT_DATABASE_URL') | Should Be $false
        ($rt -contains 'AGENTCORE_ALLOW_DEMO') | Should Be $false
        $ex = $m.services.'core-exporter'.environment.PSObject.Properties.Name
        ($ex -contains 'CORE_EXPORT_DATABASE_URL') | Should Be $true
        ($ex -contains 'AGENTCORE_REGISTRY_DSN') | Should Be $false
        ($ex -contains 'PULSO_SERVICE_KEY_REF') | Should Be $true
    }
    It 'orders start: migrate after postgres healthy, runtime after seed, exporter after runtime' {
        $m.services.'core-migrate'.depends_on.'core-postgres'.condition | Should Be 'service_healthy'
        $m.services.'core-runtime'.depends_on.'core-seed'.condition | Should Be 'service_completed_successfully'
        $m.services.'core-exporter'.depends_on.'core-runtime'.condition | Should Be 'service_healthy'
    }
    It 'has D.5 aliases' {
        (@($m.services.'core-postgres'.networks.'core-net'.aliases) -contains 'postgres-core') | Should Be $true
        (@($m.services.'core-runtime'.networks.'core-net'.aliases) -contains 'pulso-core-runtime') | Should Be $true
    }
    It 'embeds no secret literal in the compose files' {
        $hits = Get-ChildItem $core -Filter 'compose*.yaml' | Select-String -Pattern '(PASSWORD|SECRET|TOKEN|KEY)\w*:\s*["'']?[A-Za-z0-9+/]{12,}'
        @($hits).Count | Should Be 0
    }
}

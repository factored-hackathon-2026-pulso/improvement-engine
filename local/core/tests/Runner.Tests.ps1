$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$core = (Resolve-Path (Join-Path $here '..')).Path
. (Join-Path $core 'lib\machine.ps1')
. (Join-Path $core 'lib\runner.ps1')
. (Join-Path $core 'lib\memory.ps1')
. (Join-Path $core 'lib\secrets.ps1')

$envf = Join-Path $env:TEMP 'pulso-runner-test.env'
Set-Content $envf "PULSO_CORE_IMAGE=localhost/pulso-core-runtime:t`nPULSO_SIM_IMAGE=localhost/pulso-platform-sim:t`nPOSTGRES_PASSWORD=x`n"
$model = Get-ComposeModel -Namespace 'claude-rt' -Profile 'real_local' -EnvFiles @($envf)

Describe 'runner: compose model to podman create' {
    It 'orders services by depends_on (postgres before migrate before grants before seed before runtime)' {
        $o = Get-StartOrder -Model $model
        ($o.IndexOf('core-postgres') -lt $o.IndexOf('core-migrate')) | Should Be $true
        ($o.IndexOf('core-migrate') -lt $o.IndexOf('core-grants')) | Should Be $true
        ($o.IndexOf('core-grants') -lt $o.IndexOf('core-seed')) | Should Be $true
        ($o.IndexOf('core-seed') -lt $o.IndexOf('core-runtime')) | Should Be $true
        ($o.IndexOf('core-runtime') -lt $o.IndexOf('core-exporter')) | Should Be $true
    }
    It 'encodes the cgroup workaround on every create' {
        foreach ($s in (Get-StartOrder -Model $model)) {
            $a = ConvertTo-PodmanCreateArgs -Model $model -Service $s -Project 'pulso-claude-rt'
            ($a -contains '--cgroups=disabled') | Should Be $true
            ($a -contains '--pids-limit=0') | Should Be $true
        }
    }
    It 'names containers <project>-<service>-1 on the project network with team/namespace labels' {
        $a = ConvertTo-PodmanCreateArgs -Model $model -Service 'core-runtime' -Project 'pulso-claude-rt'
        ($a -contains 'pulso-claude-rt-core-runtime-1') | Should Be $true
        ($a -contains 'pulso-claude-rt_core-net') | Should Be $true
        ($a -contains 'com.pulso.team=claude') | Should Be $true
        ($a -contains 'com.pulso.namespace=claude-rt') | Should Be $true
    }
    It 'never leaks the runner-only copy label to the engine' {
        $a = ConvertTo-PodmanCreateArgs -Model $model -Service 'core-seed' -Project 'pulso-claude-rt'
        (($a -join ' ') -match 'com.pulso.runner.copy') | Should Be $false
    }
    It 'unescapes compose $$ for the engine' {
        $a = ConvertTo-PodmanCreateArgs -Model $model -Service 'core-grants' -Project 'pulso-claude-rt'
        (($a -join ' ') -match '\$\$') | Should Be $false
        (($a -join ' ') -match '\$CORE_ADMIN_DSN') | Should Be $true
    }
    It 'publishes ports on 127.0.0.1 only' {
        $a = ConvertTo-PodmanCreateArgs -Model $model -Service 'core-runtime' -Project 'pulso-claude-rt'
        $i = [array]::IndexOf($a, '-p')
        $a[$i + 1] | Should Match '^127\.0\.0\.1:\d+:8000$'
    }
    It 'passes through the digest-pinned postgres image' {
        $a = ConvertTo-PodmanCreateArgs -Model $model -Service 'core-postgres' -Project 'pulso-claude-rt'
        (($a -join ' ') -match 'postgres@sha256:[0-9a-f]{64}') | Should Be $true
    }
}

Describe 'memory preflight' {
    It 'refuses when free memory is below steady use + 25%' {
        Test-MemoryBudget -FreeBytes (1500MB) -Profile 'real_local' | Should Be $false
        Test-MemoryBudget -FreeBytes (800MB) -Profile 'fixture' | Should Be $false
    }
    It 'accepts with enough free memory' {
        Test-MemoryBudget -FreeBytes (2500MB) -Profile 'real_local' | Should Be $true
        Test-MemoryBudget -FreeBytes (1200MB) -Profile 'fixture' | Should Be $true
    }
}

Describe 'secrets' {
    It 'appends only missing names and never rewrites existing values' {
        $root = Join-Path $env:TEMP ('pulso-sec-' + [guid]::NewGuid().ToString('N'))
        $f = Initialize-LocalSecrets -Root $root -Namespace 'claude-s1'
        $before = Get-Content $f -Raw
        $null = Initialize-LocalSecrets -Root $root -Namespace 'claude-s1'
        (Get-Content $f -Raw) | Should Be $before
        $before | Should Match 'AGENTCORE_KEYS_FINGERPRINT=local1:'
        $before | Should Match '(?m)^AGENTCORE_LLM_GATEWAY_TOKEN=[A-Za-z0-9]{32,}$'
        [IO.Directory]::Delete($root, $true)
    }
}

Describe 'start failure mapping' {
    It 'maps a bind race to port_conflict and cgroup errors to runtime_cgroup_unavailable' {
        (Get-StartFailureCode -Service 'core-postgres' -Output 'Error: rootlessport listen tcp 127.0.0.1:18330: bind: address already in use') | Should Match '^pulso:port_conflict'
        (Get-StartFailureCode -Service 'x' -Output 'crun: controller `pids` is not available') | Should Match '^pulso:runtime_cgroup_unavailable'
        (Get-StartFailureCode -Service 'core-migrate' -Output 'boom') | Should Match '^pulso:core_migrate_failed'
    }
}

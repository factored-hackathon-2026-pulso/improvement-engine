# Translates the rendered compose model (docker-compose config --format json) into `podman create` calls so the
# cgroup workaround (--cgroups=disabled --pids-limit=0, carried as x-pulso) can be applied: compose cannot express it.
$script:CoreDir = (Resolve-Path (Join-Path (Split-Path -Parent $MyInvocation.MyCommand.Path) '..')).Path

function Get-ComposeModel {
    param([Parameter(Mandatory)][string]$Namespace, [Parameter(Mandatory)][string]$Profile, [string[]]$EnvFiles = @())
    $env:PULSO_NS = $Namespace; $env:PULSO_PROJECT = "pulso-$Namespace"
    $a = @('-f', (Join-Path $script:CoreDir 'compose.core.yaml'), '-f', (Join-Path $script:CoreDir 'compose.standalone.yaml'))
    foreach ($e in $EnvFiles) { $a += @('--env-file', $e) }
    $a += @('--profile', $Profile, 'config', '--format', 'json')
    $json = & docker-compose @a 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0) { throw "pulso:compose_invalid: $($json.Trim())" }
    $json | ConvertFrom-Json
}

function Get-StartOrder {
    param([Parameter(Mandatory)]$Model)
    $names = @($Model.services.PSObject.Properties.Name); $done = New-Object System.Collections.Generic.List[string]
    $guard = 0
    while ($done.Count -lt $names.Count -and $guard++ -lt 100) {
        foreach ($n in $names) {
            if ($done.Contains($n)) { continue }
            $deps = @(); if ($Model.services.$n.depends_on) { $deps = @($Model.services.$n.depends_on.PSObject.Properties.Name) }
            $pending = @($deps | Where-Object { $_ -in $names -and -not $done.Contains($_) })
            if ($pending.Count -eq 0) { $done.Add($n) }
        }
    }
    @($done)
}

function ConvertTo-PodmanCreateArgs {
    param([Parameter(Mandatory)]$Model, [Parameter(Mandatory)][string]$Service, [Parameter(Mandatory)][string]$Project)
    $s = $Model.services.$Service
    $L = $s.labels
    $a = @('create', '--name', "$Project-$Service-1", '--network', "${Project}_core-net")
    if ($L.'com.pulso.runner.cgroups' -eq 'disabled') { $a += '--cgroups=disabled' }
    if ($null -ne $L.'com.pulso.runner.pids_limit') { $a += "--pids-limit=$($L.'com.pulso.runner.pids_limit')" }
    foreach ($l in $L.PSObject.Properties) { if ($l.Name -ne 'com.pulso.runner.copy') { $a += @('--label', "$($l.Name)=$($l.Value)") } }
    $a += @('--label', "com.docker.compose.project=$Project", '--label', "com.docker.compose.service=$Service")
    if ($s.networks -and $s.networks.'core-net' -and $s.networks.'core-net'.aliases) { foreach ($al in $s.networks.'core-net'.aliases) { $a += @('--network-alias', $al) } }
    $a += @('--network-alias', $Service)
    if ($s.environment) { foreach ($e in $s.environment.PSObject.Properties) { $a += @('-e', "$($e.Name)=$($e.Value)") } }
    foreach ($v in @($s.volumes)) { if ($v) { $a += @('-v', ("${Project}_$($v.source):$($v.target)" + $(if ($v.read_only) { ':ro' } else { '' }))) } }
    foreach ($p in @($s.ports)) { if ($p) { $a += @('-p', "$($p.host_ip):$($p.published):$($p.target)") } }
    if ($s.mem_limit) { $a += @('--memory', "$($s.mem_limit)") }
    if ($s.user) { $a += @('--user', "$($s.user)") }
    if ($s.healthcheck) {
        $h = $s.healthcheck
        $a += @('--health-cmd', (ConvertTo-Json @($h.test) -Compress))
        if ($h.interval) { $a += @('--health-interval', $h.interval) }
        if ($h.timeout) { $a += @('--health-timeout', $h.timeout) }
        if ($h.retries) { $a += @('--health-retries', "$($h.retries)") }
        if ($h.start_period) { $a += @('--health-start-period', $h.start_period) }
    }
    if ($s.entrypoint) { $a += @('--entrypoint', (ConvertTo-Json @($s.entrypoint) -Compress)) }
    $a += $s.image
    if ($s.command) { $a += @($s.command) }
    # docker-compose config keeps $$ escapes; the engine must see the single $ (compose does this itself at run time).
    , @($a | ForEach-Object { "$_".Replace('$$', '$') })
}

function Get-ContainerState {
    param([string]$Connection, [string]$Name)
    $raw = Invoke-Podman -Connection $Connection inspect $Name --format '{{.State.Status}}|{{.State.ExitCode}}|{{if .State.Health}}{{.State.Health.Status}}{{end}}' 2>$null
    if ($LASTEXITCODE -ne 0 -or -not $raw) { return $null }
    $p = ([string]$raw).Trim().Split('|')
    [pscustomobject]@{ status = $p[0]; exit = [int]$p[1]; health = $p[2] }
}

function Wait-Dependency {
    param([string]$Connection, [string]$Project, [string]$Dep, [string]$Condition, [int]$TimeoutSec = 240)
    $deadline = (Get-Date).AddSeconds($TimeoutSec)
    $name = "$Project-$Dep-1"
    while ((Get-Date) -lt $deadline) {
        $st = Get-ContainerState -Connection $Connection -Name $name
        if ($st) {
            if ($Condition -eq 'service_healthy' -and $st.health -eq 'healthy') { return }
            if ($Condition -eq 'service_completed_successfully' -and $st.status -eq 'exited') {
                if ($st.exit -eq 0) { return }
                throw (Get-FailureCode -Service $Dep -Detail "exit $($st.exit)")
            }
            if ($st.status -in 'exited', 'stopped' -and $Condition -eq 'service_healthy') { throw (Get-FailureCode -Service $Dep -Detail "exited $($st.exit)") }
        }
        Start-Sleep -Milliseconds 1500
    }
    throw (Get-FailureCode -Service $Dep -Detail "timeout waiting for $Condition")
}

function Get-StartFailureCode {
    param([string]$Service, [string]$Output)
    if ($Output -match 'address already in use|port is already allocated|bind:') { return "pulso:port_conflict: $Service could not bind its host port (taken after the probe); nothing else was touched" }
    if ($Output -match 'controller .pids. is not available|cgroup') { return "pulso:runtime_cgroup_unavailable: $Service could not start" }
    Get-FailureCode -Service $Service -Detail 'start failed'
}

function Get-FailureCode {
    param([string]$Service, [string]$Detail)
    switch ($Service) {
        'core-postgres' { "pulso:core_postgres_unavailable: $Detail" }
        'core-migrate' { "pulso:core_migrate_failed: $Detail" }
        'core-seed' { "pulso:core_seed_failed: $Detail" }
        { $_ -in 'core-runtime', 'core-exporter', 'platform-sim' } { "pulso:core_not_ready: $Service $Detail" }
        default { "pulso:core_not_ready: $Service $Detail" }
    }
}

function Copy-IntoContainer {
    # Stages the files under their destination paths in a temp tree, then copies that tree onto the container root
    # (creates missing directories; a remote engine cannot bind-mount Windows paths).
    param([string]$Connection, [string]$Container, $Copy)
    $items = @($Copy | Where-Object { $_ })
    if ($items.Count -eq 0) { return }
    $repoRoot = (Resolve-Path (Join-Path $script:CoreDir '..\..')).Path
    $stage = Join-Path ([IO.Path]::GetTempPath()) ('pulso-stage-' + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $stage | Out-Null
    try {
        foreach ($c in $items) {
            $src = if ($c.src -like 'repo:*') { Join-Path $repoRoot $c.src.Substring(5) } else { Join-Path $script:CoreDir $c.src }
            $target = Join-Path $stage ($c.dest.TrimStart('/') -replace '/', '\')
            New-Item -ItemType Directory -Force -Path (Split-Path $target -Parent) | Out-Null
            if (Test-Path -LiteralPath $src -PathType Container) {
                Copy-Item -LiteralPath $src -Destination $target -Recurse -Force
            } else { Copy-Item -LiteralPath $src -Destination $target -Force }
        }
        Invoke-Podman -Connection $Connection cp "$stage\." "${Container}:/" | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "pulso:core_not_ready: copying files into $Container failed" }
    } finally { [IO.Directory]::Delete($stage, $true) }
}

function Start-CoreStack {
    param([Parameter(Mandatory)]$Model, [Parameter(Mandatory)][string]$Connection, [Parameter(Mandatory)][string]$Project, [Parameter(Mandatory)][string]$Namespace)
    $labels = @('--label', 'com.pulso.team=claude', '--label', "com.pulso.namespace=$Namespace")
    $net = "${Project}_core-net"
    Invoke-Podman -Connection $Connection network inspect $net 2>$null | Out-Null
    if ($LASTEXITCODE -ne 0) { Invoke-Podman -Connection $Connection network create @labels $net | Out-Null }
    foreach ($v in $Model.volumes.PSObject.Properties.Name) {
        $vn = "${Project}_$v"
        Invoke-Podman -Connection $Connection volume inspect $vn 2>$null | Out-Null
        if ($LASTEXITCODE -ne 0) { Invoke-Podman -Connection $Connection volume create @labels $vn | Out-Null }
    }
    foreach ($svc in (Get-StartOrder -Model $Model)) {
        $s = $Model.services.$svc
        $name = "$Project-$svc-1"
        if ($s.depends_on) {
            foreach ($d in $s.depends_on.PSObject.Properties) { if ($d.Name -notin $Model.services.PSObject.Properties.Name) { continue }; Wait-Dependency -Connection $Connection -Project $Project -Dep $d.Name -Condition $d.Value.condition }
        }
        $st = Get-ContainerState -Connection $Connection -Name $name
        $oneShot = ($s.restart -eq 'no')
        if ($st -and $st.status -eq 'running') { continue }
        if ($st -and $oneShot -and $st.status -eq 'exited' -and $st.exit -eq 0 -and $svc -notin 'core-grants') {
            Invoke-Podman -Connection $Connection rm -f $name | Out-Null; $st = $null
        }
        if ($st) { Invoke-Podman -Connection $Connection rm -f $name | Out-Null }
        $args = ConvertTo-PodmanCreateArgs -Model $Model -Service $svc -Project $Project
        $out = Invoke-Podman -Connection $Connection @args 2>&1
        if ($LASTEXITCODE -ne 0) { throw "pulso:core_not_ready: create $svc failed: $(([string]$out).Substring(0, [Math]::Min(300, ([string]$out).Length)))" }
        $copy = if ($s.labels.'com.pulso.runner.copy') { $s.labels.'com.pulso.runner.copy' | ConvertFrom-Json } else { @() }
        Copy-IntoContainer -Connection $Connection -Container $name -Copy $copy
        $out = Invoke-Podman -Connection $Connection start $name 2>&1
        if ($LASTEXITCODE -ne 0) {
            throw (Get-StartFailureCode -Service $svc -Output ([string]$out))
        }
    }
}

# The doubles image is the runtime image plus test-only deps (local/core/doubles/Dockerfile). Tag is derived from the base id.
function Ensure-SimImage {
    param([Parameter(Mandatory)][string]$Connection, [Parameter(Mandatory)][string]$BaseImage)
    $id = (Invoke-Podman -Connection $Connection image inspect $BaseImage --format '{{.Id}}').Trim()
    $tag = "localhost/pulso-platform-sim:$($id.Substring(0, 12))"
    Invoke-Podman -Connection $Connection image inspect $tag 2>$null | Out-Null
    if ($LASTEXITCODE -ne 0) {
        Invoke-Podman -Connection $Connection build --build-arg "BASE=$BaseImage" -f (Join-Path $script:CoreDir 'doubles\Dockerfile') -t $tag (Join-Path $script:CoreDir 'doubles') | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "pulso:core_not_ready: building the doubles image failed" }
    }
    $tag
}

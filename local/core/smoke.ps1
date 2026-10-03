<#
.SYNOPSIS
  Smoke of a running Core stack: /readyz, version/pin probe, seed verification, scripted task, exporter round trip.
  Writes an honest integration report (target is DERIVED: real_local only when /_sim/info is absent, sha == pin,
  image digest == expected and /readyz passes; otherwise evidence_target_unproven). Steps that cannot run are `not_run`
  with a reason, never `pass`.
  smoke.ps1 -Namespace claude-x [-Exec]            (reads local/.secrets/<ns>/state.json; -Exec probes via podman exec)
  smoke.ps1 -Namespace claude-x -BaseUrl http://127.0.0.1:N [-Token t]   (direct HTTP, e.g. a stub)
#>
[CmdletBinding()]
param([Parameter(Mandatory)][string]$Namespace, [string]$BaseUrl, [string]$Token, [switch]$Exec, [int]$TimeoutSec = 60,
      [string]$ReportPath, [string]$Machine = 'pulso-dev', [switch]$SkipSeedCheck, [switch]$SkipExporter,
      [string]$ExpectedImageDigest)
$ErrorActionPreference = 'Stop'
foreach ($f in 'errors', 'machine', 'namespace', 'evidence', 'runner') { . (Join-Path $PSScriptRoot "lib\$f.ps1") }
$started = [DateTime]::UtcNow
$suites = New-Object System.Collections.Generic.List[object]
$commands = New-Object System.Collections.Generic.List[string]
function Add-Check([string]$name, [string]$status, [string]$detail) { $suites.Add([ordered]@{ name = $name; status = $status; detail = $detail }) }
try {
    Assert-ClaudeNamespace -Namespace $Namespace
    $m = Assert-RegisteredMachine -Machine $Machine
    $conn = $m.connection
    $stateFile = Join-Path (Join-Path $PSScriptRoot '..\.secrets') "$Namespace\state.json"
    $state = if (Test-Path -LiteralPath $stateFile) { Get-Content -LiteralPath $stateFile -Raw | ConvertFrom-Json } else { $null }
    if (-not $BaseUrl) {
        if (-not $state) { throw "pulso:core_not_ready: no -BaseUrl and no state for $Namespace (run start.ps1)" }
        $BaseUrl = "http://127.0.0.1:$($state.ports.runtime)"; $Exec = $true
    }
    if (-not $ExpectedImageDigest -and $state) { $ExpectedImageDigest = $state.expected_image_digest }
    $project = Get-PulsoProject -Namespace $Namespace

    # 1. readiness
    $ready = $false; $deadline = (Get-Date).AddSeconds($TimeoutSec)
    while ((Get-Date) -lt $deadline -and -not $ready) {
        try { $r = Invoke-WebRequest -Uri "$BaseUrl/readyz" -TimeoutSec 3 -UseBasicParsing; $ready = ($r.StatusCode -eq 200) } catch { Start-Sleep -Milliseconds 700 }
    }
    if (-not $ready) { Add-Check 'readyz' 'fail' 'not 200 before timeout'; throw "pulso:core_not_ready: $BaseUrl/readyz did not return 200 within ${TimeoutSec}s" }
    Add-Check 'readyz' 'pass' '200'
    $commands.Add("GET $BaseUrl/readyz")

    # 2. version / pin probe
    if ($Exec) {
        $commands.Add("podman --connection $conn exec $project-core-runtime-1 python /init/probe_version.py")
        $raw = (Invoke-Podman -Connection $conn exec "$project-core-runtime-1" python /init/probe_version.py) -join ''
    } else {
        $h = @{}; if ($Token) { $h['Authorization'] = "Bearer $Token" }
        $commands.Add("GET $BaseUrl/internal/v1/version")
        $raw = (Invoke-WebRequest -Uri "$BaseUrl/internal/v1/version" -Headers $h -TimeoutSec 5 -UseBasicParsing).Content
        if ($raw -is [byte[]]) { $raw = [Text.Encoding]::UTF8.GetString($raw) }
    }
    $ver = $raw | ConvertFrom-Json
    Add-Check 'version_pin' $(if ($ver.agent_core_sha -eq '86a767474042a566a0dbd6ed23588959f27ebdb3') { 'pass' } else { 'fail' }) "sha=$($ver.agent_core_sha) contracts=$($ver.contracts_version)"

    # 3. double detection
    $sim = $false
    try { $r = Invoke-WebRequest -Uri "$BaseUrl/_sim/info" -TimeoutSec 3 -UseBasicParsing; $sim = ($r.StatusCode -eq 200 -and $r.Content -match 'double') } catch { $sim = $false }

    # 4. seed verification
    if ($SkipSeedCheck -or -not $Exec) { Add-Check 'seed_verification' 'not_run' 'no engine access in this mode' } else {
        $q = (Invoke-Podman -Connection $conn exec "$project-core-postgres-1" psql -U postgres -d core_runtime -Atc "select count(*) from pulso_seed_state") -join ''
        Add-Check 'seed_verification' $(if ([int]$q.Trim() -ge 1) { 'pass' } else { 'fail' }) "pulso_seed_state rows=$($q.Trim())"
    }
    # 5. scripted task: needs the L3 task route plus a model/broker; never claimed
    Add-Check 'scripted_task' 'not_run' 'no scripted task fixture: core-tasks/invoke needs L3 wiring and a broker double with a model script'
    # 6. exporter round trip
    if ($SkipExporter -or -not $Exec) { Add-Check 'exporter_round_trip' 'not_run' 'skipped' } else {
        $st = Get-ContainerState -Connection $conn -Name "$project-core-exporter-1" 2>$null
        Add-Check 'exporter_round_trip' $(if ($st -and $st.status -eq 'running') { 'pass' } else { 'fail' }) "exporter container status=$($st.status)"
    }

    $containers = @()
    if ($state -and $Exec) {
        $containers = @(Invoke-Podman -Connection $conn ps --filter 'label=com.pulso.role=double' --filter "label=com.pulso.namespace=$Namespace" --format '{{.Label "com.docker.compose.service"}}' 2>$null)
    }
    $doubles = Merge-Doubles -Runtime @($ver.doubles) -Containers $containers
    $target = if ($ver.runtime_profile -eq 'contract_mock') { 'mock' } else {
        Test-RealLocalEvidence -SimInfoPresent $sim -AgentCoreSha $ver.agent_core_sha -ImageDigest $ver.image_digest -ExpectedImageDigest $ExpectedImageDigest -Ready $ready }
    $failed = @($suites | Where-Object status -eq 'fail').Count
    $manifest = Join-Path $PSScriptRoot '..\..\agent-core-assets\manifest.yaml'
    $report = [ordered]@{
        schema_version = 1; target = $target; runtime_profile = $ver.runtime_profile; agent_core_sha = $ver.agent_core_sha
        contracts_version = $ver.contracts_version
        pin_manifest_sha256 = $(if (Test-Path $manifest) { (Get-FileHash $manifest -Algorithm SHA256).Hash.ToLower() } else { $null })
        image_digest = $(if ($ver.image_digest -and $ver.image_digest -ne 'unknown') { $ver.image_digest } else { $null })
        head_sha = ((git -C $PSScriptRoot rev-parse HEAD 2>$null) -join '')
        namespace = $Namespace; machine = [ordered]@{ name = $m.name; connection = $conn }
        doubles = @($doubles)
        suites = @($suites | ForEach-Object { [ordered]@{ name = $_.name; total = 1; passed = [int]($_.status -eq 'pass'); failed = [int]($_.status -eq 'fail')
                    not_run = [int]($_.status -eq 'not_run'); skipped = @(); detail = $_.detail } })
        commands = @($commands); started_at = $started.ToString('o'); finished_at = [DateTime]::UtcNow.ToString('o')
    }
    if ($ReportPath) { $report | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $ReportPath -Encoding utf8 }
    foreach ($s in $suites) { Write-Output "  [$($s.status)] $($s.name): $($s.detail)" }
    Write-Output "target=$target runtime_profile=$($ver.runtime_profile) doubles=$(@($doubles).Count)"
    if ($failed -gt 0) { Write-Output "smoke: fail ($failed failed)"; exit 1 }
    Write-Output 'smoke: pass'
    exit 0
} catch { Exit-Pulso $_ }

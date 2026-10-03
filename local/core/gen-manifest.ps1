<#
.SYNOPSIS  Generates local/core/manifests/local-service-manifest.json from the compose model (plan 17.9.4 LocalServiceManifest).
#>
[CmdletBinding()] param([string]$Namespace = 'claude-manifest', [string]$Out)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'lib\runner.ps1')
if (-not $Out) { $Out = Join-Path $PSScriptRoot 'manifests\local-service-manifest.json' }
$m = $null
$tmpEnv = Join-Path ([IO.Path]::GetTempPath()) 'pulso-manifest.env'
Set-Content -LiteralPath $tmpEnv -Value "PULSO_CORE_IMAGE=localhost/pulso-core-runtime:manifest`n"
$env:PULSO_NS = $Namespace
$a = @('-f', (Join-Path $PSScriptRoot 'compose.core.yaml'), '-f', (Join-Path $PSScriptRoot 'compose.standalone.yaml'), '--env-file', $tmpEnv,
       '--profile', 'real_local', '--profile', 'fixture', 'config', '--format', 'json')
$m = (& docker-compose @a | Out-String | ConvertFrom-Json)
$health = @{ 'core-runtime' = '/readyz'; 'platform-sim' = '/_sim/info' }
$services = foreach ($n in $m.services.PSObject.Properties.Name) {
    $s = $m.services.$n
    $internal = if ($s.ports) { [int]@($s.ports)[0].target } else { $null }
    $deps = @(); if ($s.depends_on) { $deps = @($s.depends_on.PSObject.Properties.Name) }
    $aliases = @(); if ($s.networks.'core-net'.aliases) { $aliases = @($s.networks.'core-net'.aliases) }
    [ordered]@{
        name = $n; dns = $(if ($aliases.Count -gt 0) { $aliases[0] } else { $n })
        aliases = $aliases
        entrypoint = @($s.entrypoint) + @($s.command) | Where-Object { $_ }
        image_digest = $(if ("$($s.image)" -match '@(sha256:[0-9a-f]{64})$') { $Matches[1] } else { $null })
        image = "$($s.image)" -replace '^.*:manifest$', '${PULSO_CORE_IMAGE}'
        internal_port = $internal; health_path = $health[$n]; readiness_path = $(if ($n -eq 'core-runtime') { '/readyz' } else { $null })
        env_names = @($s.environment.PSObject.Properties.Name | Sort-Object); depends_on = $deps
        profiles = @($s.profiles); role = $s.labels.'com.pulso.role'
    }
}
$manifest = [ordered]@{
    schema_version = 1; namespace = '<set at start>'; services = @($services)
    migrate_commands = @('agentcore migrate --app-role core_app  (env AGENTCORE_REGISTRY_DSN, AGENTCORE_EVAL_DSN: admin names, service core-migrate)')
    seed_commands = @('python /init/seed_assets.py  (service core-seed; idempotent via pulso_seed_state digest marker)')
    limits_config_ref = 'local/core/compose.core.yaml#services.*.mem_limit'
    notes = 'Generated; do not edit. Contains env NAMES only. Doubles carry role=double.'
}
New-Item -ItemType Directory -Force -Path (Split-Path $Out -Parent) | Out-Null
$manifest | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $Out -Encoding utf8
Write-Output "manifest: $($services.Count) services -> $Out"

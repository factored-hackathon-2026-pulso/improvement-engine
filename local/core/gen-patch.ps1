<#
.SYNOPSIS  Regenerates local/core/fragments/patch.json (plan 17.9.4) with the current fragment sha256 values.
#>
[CmdletBinding()] param()
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
function Hash($rel) { (Get-FileHash -LiteralPath (Join-Path $repo $rel) -Algorithm SHA256).Hash.ToLower() }
$entries = @(
    [ordered]@{ target = 'local/compose.yaml'; op = 'add_service'; anchor = 'name'
                fragment_path = 'local/core/fragments/include.fragment.yaml'; fragment_sha256 = (Hash 'local/core/fragments/include.fragment.yaml')
                requires_owner_ack = $true
                note = 'include: the Core stack under the same project-name rule; also make core-egress non-internal (CLQ-34, TBV)' },
    [ordered]@{ target = 'local/compose.yaml'; op = 'add_service'; anchor = 'services'
                fragment_path = 'local/core/fragments/otel-collector.yaml'; fragment_sha256 = (Hash 'local/core/fragments/otel-collector.yaml')
                requires_owner_ack = $true; note = 'optional OTel collector; owner pins the image digest' },
    [ordered]@{ target = '.github/workflows/ci.yml'; op = 'add_job'; anchor = 'jobs'
                fragment_path = 'local/core/fragments/ci.fragment.yml'; fragment_sha256 = (Hash 'local/core/fragments/ci.fragment.yml')
                requires_owner_ack = $true; note = 'jobs call core-bridge/scripts/ci.ps1 -Job <name> (script owned by the bridge package)' }
)
[ordered]@{ schema_version = 1; delivered_by = 'claude'; entries = $entries } | ConvertTo-Json -Depth 6 |
    Set-Content -LiteralPath (Join-Path $repo 'local\core\fragments\patch.json') -Encoding utf8
Write-Output "patch.json: $($entries.Count) entries"

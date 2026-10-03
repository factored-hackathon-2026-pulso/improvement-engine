# Local secrets: generated once into the git-ignored local/.secrets/<ns>/core.env, never printed, never overwritten.
function New-RandomSecret {
    param([int]$Length = 32)
    $bytes = New-Object byte[] $Length
    [Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($bytes)
    $alpha = 'abcdefghijkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789'
    -join ($bytes | ForEach-Object { $alpha[$_ % $alpha.Length] })
}

function New-KeyEnvValue {
    $bytes = New-Object byte[] 32
    [Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($bytes)
    'local1:' + [Convert]::ToBase64String($bytes)
}

# Creates core.env once; later runs only append names that an older file lacks (never rewrites existing values).
function Initialize-LocalSecrets {
    param([Parameter(Mandatory)][string]$Root, [Parameter(Mandatory)][string]$Namespace)
    $dir = Join-Path $Root $Namespace
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    $file = Join-Path $dir 'core.env'
    $wanted = [ordered]@{
        POSTGRES_PASSWORD = { New-RandomSecret }; CORE_APP_PASSWORD = { New-RandomSecret }
        CORE_EVAL_APP_PASSWORD = { New-RandomSecret }; EXPORTER_RO_PASSWORD = { New-RandomSecret }
        PULSO_TENANT_ID = { 'tenant-local' }
        AGENTCORE_KEYS_FINGERPRINT = { New-KeyEnvValue }; AGENTCORE_KEYS_TOKEN_MAP = { New-KeyEnvValue }
        # Local placeholder: no real JEV key exists on this machine; JEV calls are doubles/unavailable.
        AGENTCORE_JEV_API_KEY = { New-RandomSecret 40 }
        # Consumer token the runtime presents to the llm-gateway (git-ignored core.env, never in the repo or logs).
        AGENTCORE_LLM_GATEWAY_TOKEN = { New-RandomSecret 48 }
    }
    $existing = @(); if (Test-Path -LiteralPath $file) { $existing = @(Get-Content -LiteralPath $file | ForEach-Object { ($_ -split '=', 2)[0] }) }
    $add = @(); foreach ($k in $wanted.Keys) { if ($k -notin $existing) { $add += "$k=$(& $wanted[$k])" } }
    if ($add.Count -gt 0) {
        $prefix = if ($existing.Count -gt 0) { [IO.File]::ReadAllText($file) } else { '' }
        [IO.File]::WriteAllText($file, $prefix + (($add -join "`n") + "`n"), (New-Object Text.UTF8Encoding($false)))
    }
    $file
}
function Write-PortsEnv {
    param([Parameter(Mandatory)][string]$Root, [Parameter(Mandatory)][string]$Namespace, [hashtable]$Ports, [string]$Image, [string]$ImageDigest = 'unknown', [string]$SimImage = 'localhost/pulso-platform-sim:dry-run')
    $dir = Join-Path $Root $Namespace
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    $file = Join-Path $dir 'ports.env'
    $m = @{ postgres = 'PORT_POSTGRES'; runtime = 'PORT_RUNTIME'; exporter = 'PORT_EXPORTER'
            'sim-registry' = 'PORT_SIM_REGISTRY'; 'sim-bridge' = 'PORT_SIM_BRIDGE'; 'sim-ingest' = 'PORT_SIM_INGEST' }
    $lines = @($Ports.Keys | ForEach-Object { "$($m[$_])=$($Ports[$_])" })
    $lines += "PULSO_CORE_IMAGE=$Image"
    $lines += "PULSO_IMAGE_DIGEST=$ImageDigest"
    $lines += "PULSO_SIM_IMAGE=$SimImage"
    [IO.File]::WriteAllText($file, (($lines -join "`n") + "`n"), (New-Object Text.UTF8Encoding($false)))
    $file
}

<#
.SYNOPSIS
  Applies local/core/fragments/patch.json to a TEMPORARY copy of the Codex-owned targets and validates the result
  (plan 17.9.4): fragment sha256, YAML parse, `docker-compose config` of the patched root compose, network/DNS names,
  no secrets. Never writes outside the temp directory. -TamperForTest corrupts one expected hash to prove the gate fails.
  Exit 0 and `fragments: ok` on success.
#>
[CmdletBinding()]
param([switch]$TamperForTest)
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$patch = Get-Content (Join-Path $repo 'local\core\fragments\patch.json') -Raw | ConvertFrom-Json
$errors = New-Object System.Collections.Generic.List[string]
$tmp = Join-Path ([IO.Path]::GetTempPath()) ('pulso-frag-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $tmp | Out-Null
try {
    # temp copy of the targets and of our tree (without secrets)
    foreach ($t in @($patch.entries.target | Sort-Object -Unique)) {
        $src = Join-Path $repo $t
        if (Test-Path -LiteralPath $src) {
            $dst = Join-Path $tmp $t; New-Item -ItemType Directory -Force -Path (Split-Path $dst -Parent) | Out-Null
            Copy-Item -LiteralPath $src -Destination $dst
        } else { $errors.Add("target_missing: $t") }
    }
    Copy-Item -Recurse -Path (Join-Path $repo 'local\core') -Destination (Join-Path $tmp 'local\core')

    $first = $true
    foreach ($e in $patch.entries) {
        $frag = Join-Path $repo $e.fragment_path
        $expected = $e.fragment_sha256
        if ($TamperForTest -and $first) { $expected = ('0' * 64); $first = $false }
        $actual = (Get-FileHash -LiteralPath $frag -Algorithm SHA256).Hash.ToLower()
        if ($actual -ne $expected) { $errors.Add("fragment_sha256 mismatch for $($e.fragment_path)"); continue }
        $target = Join-Path $tmp $e.target
        if (-not (Test-Path -LiteralPath $target)) { continue }
        $lines = New-Object System.Collections.Generic.List[string]
        $lines.AddRange([string[]]@(Get-Content -LiteralPath $target))
        $idx = -1
        for ($i = 0; $i -lt $lines.Count; $i++) { if ($lines[$i] -match "^$([regex]::Escape($e.anchor)):") { $idx = $i; break } }
        if ($idx -lt 0) { $errors.Add("anchor_not_found: '$($e.anchor):' in $($e.target)"); continue }
        $lines.InsertRange($idx + 1, [string[]]@(Get-Content -LiteralPath $frag))
        Set-Content -LiteralPath $target -Value $lines -Encoding utf8
    }

    # YAML lint of every patched target and fragment
    $py = (Get-Command python -ErrorAction SilentlyContinue)
    foreach ($f in (@($patch.entries.target | Sort-Object -Unique | ForEach-Object { Join-Path $tmp $_ }) | Where-Object { Test-Path $_ })) {
        if ($py) { & python -c "import sys,yaml; yaml.safe_load(open(sys.argv[1], encoding='utf-8'))" $f 2>&1 | Out-Null; if ($LASTEXITCODE -ne 0) { $errors.Add("yaml_invalid: $f") } }
    }

    # patched root compose must render with the include (podman compose provider = docker-compose here)
    $rootCompose = Join-Path $tmp 'local\compose.yaml'
    if ((Test-Path $rootCompose) -and (Get-Command docker-compose -ErrorAction SilentlyContinue)) {
        $env:PULSO_POSTGRES_PASSWORD = 'verify-only'; $env:COMPOSE_PROFILES = 'real_local,fixture'
        $json = & docker-compose -f $rootCompose config --format json 2>&1 | Out-String
        if ($LASTEXITCODE -ne 0) { $errors.Add("compose_config_failed: $($json.Trim())") } else {
            $model = $json | ConvertFrom-Json
            $svcNames = @($model.services.PSObject.Properties.Name)
            foreach ($n in 'core-postgres', 'core-runtime', 'core-exporter', 'platform-sim') { if ($n -notin $svcNames) { $errors.Add("service_missing_after_patch: $n") } }
            # DNS/network names: every depends_on target exists, every service joins an existing network
            foreach ($n in $svcNames) {
                $s = $model.services.$n
                if ($s.depends_on) { foreach ($d in $s.depends_on.PSObject.Properties.Name) { if ($d -notin $svcNames) { $errors.Add("unknown_dependency: $n -> $d") } } }
                if ($s.networks) { foreach ($net in $s.networks.PSObject.Properties.Name) { if (-not $model.networks.PSObject.Properties[$net]) { $errors.Add("unknown_network: $n -> $net") } } }
                # Pulso services must not receive Core DSN names (17.3.8 service-scoped env)
                if ($n -in 'postgres', 'localstack', 'pulso-api', 'pulso-worker' -and $s.environment) {
                    foreach ($k in $s.environment.PSObject.Properties.Name) { if ($k -match '^(AGENTCORE_|CORE_)') { $errors.Add("core_env_leak: $n has $k") } }
                }
            }
        }
    }

    # secrets scan over the fragments and our compose files
    $scan = @(Get-ChildItem (Join-Path $repo 'local\core\fragments') -File) + @(Get-ChildItem (Join-Path $repo 'local\core') -Filter 'compose*.yaml')
    foreach ($f in $scan) {
        if ($f.Name -eq 'patch.json') { continue }
        $hit = Select-String -LiteralPath $f.FullName -Pattern 'BEGIN [A-Z ]*PRIVATE KEY|AKIA[0-9A-Z]{16}|(password|secret|token)\w*\s*[:=]\s*["'']?[A-Za-z0-9+/]{16,}' -CaseSensitive:$false
        if ($hit) { $errors.Add("secret_like_value: $($f.Name):$($hit[0].LineNumber)") }
    }
} finally { [IO.Directory]::Delete($tmp, $true) }

if ($errors.Count -gt 0) { $errors | ForEach-Object { Write-Output "fragments: FAIL $_" }; exit 1 }
Write-Output 'fragments: ok'
exit 0

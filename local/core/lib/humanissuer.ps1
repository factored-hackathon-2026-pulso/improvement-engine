# Local human issuer (local-identity, sandbox-only DOUBLE, plan 16.13.2 / 17.3.2): image build, CAP-63 scan, exposure verdicts.
$script:LibDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$script:RepoRoot = (Resolve-Path (Join-Path $script:LibDir '..\..\..')).Path

# Content-addressed tag: the image is rebuilt only when local-identity/ changed (tests, caches and venvs excluded).
function Get-LocalIdentityContentTag {
    param([string]$Dir = (Join-Path $script:RepoRoot 'local-identity'))
    $skip = '[\\/](\.venv|__pycache__|\.pytest_cache|\.mypy_cache|\.ruff_cache|tests)([\\/]|$)'
    $rows = Get-ChildItem -LiteralPath $Dir -Recurse -File | Where-Object { $_.FullName.Substring($Dir.Length) -notmatch $skip } |
        Sort-Object FullName | ForEach-Object { $_.FullName.Substring($Dir.Length).Replace('\', '/') + ':' + (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash }
    $sha = [Security.Cryptography.SHA256]::Create().ComputeHash([Text.Encoding]::UTF8.GetBytes(($rows -join "`n")))
    (-join ($sha | ForEach-Object { $_.ToString('x2') })).Substring(0, 12)
}

function Ensure-HumanIssuerImage {
    param([Parameter(Mandatory)][string]$Connection)
    $tag = "localhost/pulso-local-identity:$(Get-LocalIdentityContentTag)"
    Invoke-Podman -Connection $Connection image inspect $tag 2>$null | Out-Null
    if ($LASTEXITCODE -ne 0) {
        $out = Invoke-Podman -Connection $Connection build -t $tag (Join-Path $script:RepoRoot 'local-identity') 2>&1
        if ($LASTEXITCODE -ne 0) { throw "pulso:core_not_ready: building the local-identity image failed: $((([string]($out | Select-Object -Last 3)) -replace '\s+', ' '))" }
    }
    $tag
}

# CAP-63: no local-sim kid and no auth.simulated=true in any remote (staging/prod) configuration of the repository.
function Get-RemoteSimulationVerdict {
    param([string]$Root = $script:RepoRoot)
    $env:PYTHONPATH = Join-Path $script:RepoRoot 'local-identity\src'
    try {
        $raw = (& uv run --python 3.12 --no-project --quiet python (Join-Path $script:LibDir 'scan_remote.py') $Root 2>&1 | Out-String).Trim()
        $findings = @($raw | ConvertFrom-Json)
    } catch { return [pscustomobject]@{ status = 'fail'; detail = 'CAP-63 scan could not run' } }
    if ($findings.Count -eq 0) { return [pscustomobject]@{ status = 'pass'; detail = 'no local-sim kid or auth.simulated in remote configuration' } }
    [pscustomobject]@{ status = 'fail'; detail = 'simulated identity in remote config: ' + (($findings | ForEach-Object { "$($_.path) ($($_.rule))" }) -join ', ') }
}

# The human issuer is internal only: `podman port <container>` must print nothing.
function Get-HumanIssuerExposureVerdict {
    param([string]$PublishedPorts = '')
    if ("$PublishedPorts".Trim()) { return [pscustomobject]@{ status = 'fail'; detail = "human-issuer publishes a host port: $("$PublishedPorts".Trim())" } }
    [pscustomobject]@{ status = 'pass'; detail = 'human-issuer has no host port (internal 8083 only)' }
}

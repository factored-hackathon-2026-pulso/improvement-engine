<#
.SYNOPSIS
  Regenerates core-bridge/wire/agent_core@<sha7>/ from the pinned agent-core checkout (L1a, plan 17.3.1).
.DESCRIPTION
  Writes ONLY core-bridge/wire/agent_core@<sha7>/. Fails closed:
  (1) HEAD == pin sha (contracts/agent_core/pin.json when present, else the constant below), contracts/VERSION noted (1.3.0 reviewed; not a gate),
      `uv sync --locked --python 3.12` in a scratch venv OUTSIDE the repo, `agentcore contracts --check`;
  (2) byte copy of 194 schemas + 2 events + 31 registry schemas (+ openapi.json); (3) derived schemas (flagged derived_by_pulso);
  (4) golden hash vectors; (5) MANIFEST.json.
  -Check regenerates into a temp dir and compares with the committed wire dir (pulso:wire_drift on any difference).
#>
[CmdletBinding()]
param(
    [string]$Checkout = 'D:\.codex\factored\references\agent-core-894fa65',
    [switch]$Check
)
$ErrorActionPreference = 'Stop'
$PinSha = '894fa65575d83420523f33ec1c6919b8965f7ebe'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$repo = Resolve-Path (Join-Path $here '..\..')
$pinFile = Join-Path $repo 'contracts\agent_core\pin.json'
if (Test-Path $pinFile) { $PinSha = (Get-Content $pinFile -Raw | ConvertFrom-Json).sha }
$sha7 = $PinSha.Substring(0, 7)

$head = (git -C $Checkout rev-parse HEAD).Trim()
if ($head -ne $PinSha) { Write-Error "pulso:wire_gen_failed checkout HEAD $head != pin $PinSha"; exit 2 }

$venv = Join-Path $env:TEMP "pulso-wire-venv-$sha7"
$env:UV_PROJECT_ENVIRONMENT = $venv
Push-Location $Checkout
try { uv sync --locked --python 3.12 | Out-Null; if ($LASTEXITCODE -ne 0) { Write-Error 'pulso:wire_gen_failed uv sync --locked'; exit 2 } } finally { Pop-Location }
$py = Join-Path $venv 'Scripts\python.exe'

$committed = Join-Path $repo "core-bridge\wire\agent_core@$sha7"
$target = if ($Check) { Join-Path $env:TEMP "pulso-wire-check-$sha7\agent_core@$sha7" } else { $committed }
& $py -W ignore (Join-Path $here 'gen_wire.py') --checkout $Checkout --out $target --expected-sha $PinSha
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

if ($Check) {
    $a = Get-ChildItem $committed -Recurse -File | ForEach-Object { $_.FullName.Substring($committed.Length) + ':' + (Get-FileHash $_.FullName).Hash }
    $b = Get-ChildItem $target -Recurse -File | ForEach-Object { $_.FullName.Substring($target.Length) + ':' + (Get-FileHash $_.FullName).Hash }
    $diff = Compare-Object $a $b
    if ($diff) { $diff | Out-String | Write-Host; Write-Error 'pulso:wire_drift'; exit 3 }
    Write-Host 'wire check OK (no drift)'
}

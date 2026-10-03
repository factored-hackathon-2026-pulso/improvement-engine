<#
.SYNOPSIS
  Local CI parity for the repository. Single entry used by local/core/fragments/ci.fragment.yml (jobs call
  `pwsh core-bridge/scripts/ci.ps1 -Job <name>`) and by humans when hosted CI is unavailable.
.DESCRIPTION
  -Job all (default) runs every job below in order and stops at the first failure.
    rust               every step of .github/workflows/ci.yml `verify` (cargo fmt/clippy/test, Python contract tests,
                       fixture validation, pinned Pester) via scripts/verify-local-ci.ps1; with -PostgresTestUrl also
                       the `postgres-artifact-migration` job steps (destructive, local pulso_test only).
    contract-drift     gen-wire.ps1 -Check (regenerates the wire snapshot from the pinned checkout and compares)
    mock-wire          test.ps1 -Target mock
    a2-wire            test.ps1 -Target a2
    real-wire          test.ps1 -Target real_local (needs REGISTRY_BASE_URL of a running `agent-core serve`; with no
                       URL it is reported NOT REPRODUCED and fails the run unless -AllowSkipReal)
    lint               ruff check on core-bridge (src, tests, scripts), platform-sim and agent-core-assets; mypy on the
                       runtime package only when -Mypy is given
    core-bridge        the full core-bridge pytest suite on real PG16 (requires -PostgresAdmin)
    platform-sim       the full platform-sim pytest suites
    agent-core-assets  assetcheck + the full agent-core-assets pytest suite
    modules-scan       composes the runtime in a clean interpreter and fails if any `testing.*` module was imported
  -PostgresAdmin   admin DSN of a THROWAWAY Postgres 16 (e.g. postgresql://postgres:pw@127.0.0.1:55432/postgres);
                   PULSO_REQUIRE_POSTGRES=1 is set so a skipped PG test is a failure, never a pass.
  Hosted-only and therefore NOT reproduced here: the ubuntu-latest `verify` leg (the Windows leg is run), the
  `postgres:17@sha256` service container (use -PostgresTestUrl against your own local Postgres 17), and
  `actions/checkout`.
#>
[CmdletBinding()]
param(
    [ValidateSet('all', 'rust', 'contract-drift', 'mock-wire', 'a2-wire', 'real-wire', 'lint', 'core-bridge',
                 'platform-sim', 'agent-core-assets', 'modules-scan')]
    [string]$Job = 'all',
    [string]$PostgresAdmin = $env:PULSO_TEST_PG_ADMIN,
    [string]$PostgresTestUrl,
    [string]$Checkout = 'D:\.codex\factored\references\agent-core-789d6c8',
    [switch]$AllowSkipReal,
    [switch]$Mypy
)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$bridge = (Resolve-Path (Join-Path $here '..')).Path
$repo = (Resolve-Path (Join-Path $bridge '..')).Path
$pin = '789d6c89b2fca90fc10e2abf157da51dc81c5d51'
$pinFile = Join-Path $repo 'contracts\agent_core\pin.json'
if (Test-Path $pinFile) { $pin = (Get-Content $pinFile -Raw | ConvertFrom-Json).sha }
# Own venv: gen-wire.ps1 runs `uv sync --locked` on the shared pulso-wire-venv, which prunes anything outside the Core lock.
$venv = Join-Path $env:TEMP "pulso-ci-venv-$($pin.Substring(0, 7))"
$py = Join-Path $venv 'Scripts\python.exe'
$results = [System.Collections.Generic.List[string]]::new()

function Invoke-Step([string]$Name, [scriptblock]$Body) {
    Write-Output "=== $Name"
    & $Body
    if ($LASTEXITCODE -ne 0) { throw "ci step '$Name' failed with exit code $LASTEXITCODE" }
    $results.Add("PASS $Name")
}

function Ensure-Venv {
    if (Test-Path $py) { return }
    $env:UV_PROJECT_ENVIRONMENT = $venv
    Push-Location $Checkout
    try { uv sync --locked --python 3.12 | Out-Null } finally { Pop-Location }
    # The runtime-only dependencies the Core lock does not carry (same file the image build uses).
    uv pip install --python $py -r (Join-Path $bridge 'runtime-requirements.txt') | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'ci: runtime-requirements install failed' }
}

function Need-Postgres {
    if (-not $PostgresAdmin) { throw 'this job needs -PostgresAdmin (throwaway Postgres 16 admin DSN) or PULSO_TEST_PG_ADMIN' }
    $env:PULSO_TEST_PG_ADMIN = $PostgresAdmin
    $env:PULSO_REQUIRE_POSTGRES = '1'
}

function Job-Rust {
    $extra = @()
    if ($PostgresTestUrl) { $extra = @('-IncludePostgres', '-AllowDestructiveTestDb', '-PostgresTestUrl', $PostgresTestUrl) }
    Invoke-Step 'rust (verify-local-ci)' { & pwsh -NoProfile -File (Join-Path $repo 'scripts\verify-local-ci.ps1') @extra }
    if (-not $PostgresTestUrl) { $results.Add('NOT REPRODUCED rust postgres-artifact-migration steps (no -PostgresTestUrl)') }
}
function Job-Drift { Invoke-Step 'contract-drift' { & pwsh -NoProfile -File (Join-Path $here 'gen-wire.ps1') -Check -Checkout $Checkout } }
function Job-Mock { Invoke-Step 'mock-wire' { & pwsh -NoProfile -File (Join-Path $here 'test.ps1') -Target mock } }
function Job-A2 { Invoke-Step 'a2-wire' { & pwsh -NoProfile -File (Join-Path $here 'test.ps1') -Target a2 } }
function Job-Real {
    if (-not $env:REGISTRY_BASE_URL) {
        if ($AllowSkipReal -or $Job -eq 'all') { $results.Add('NOT REPRODUCED real-wire (no REGISTRY_BASE_URL)'); return }
        throw 'real-wire needs REGISTRY_BASE_URL of a running agent-core serve (pass -AllowSkipReal to record it as not reproduced)'
    }
    Invoke-Step 'real-wire' { & pwsh -NoProfile -File (Join-Path $here 'test.ps1') -Target real_local }
}
function Job-Lint {
    Ensure-Venv
    Push-Location $repo
    try {
        Invoke-Step 'ruff core-bridge' { & $py -m ruff check --isolated --select E4,E7,E9,F --ignore F811 core-bridge/src core-bridge/tests core-bridge/scripts }
        Invoke-Step 'ruff platform-sim' { & $py -m ruff check --isolated --select E4,E7,E9,F --ignore F811 platform-sim }
        Invoke-Step 'ruff agent-core-assets' { & $py -m ruff check --isolated --select E4,E7,E9,F --ignore F811 agent-core-assets }
        if ($Mypy) { Invoke-Step 'mypy runtime package' { & $py -m mypy --python-version 3.12 core-bridge/src/pulso_core_runtime } }
    } finally { Pop-Location }
}
function Job-CoreBridge {
    Ensure-Venv; Need-Postgres
    Push-Location $bridge
    try { Invoke-Step 'core-bridge pytest' { & $py -m pytest -c pyproject.toml tests -p no:cacheprovider } } finally { Pop-Location }
}
function Job-PlatformSim {
    Ensure-Venv; Need-Postgres
    Push-Location $bridge
    try { Invoke-Step 'platform-sim pytest' { & $py -m pytest -c pyproject.toml ../platform-sim/tests -p no:cacheprovider } } finally { Pop-Location }
}
function Job-Assets {
    Ensure-Venv
    Push-Location (Join-Path $repo 'agent-core-assets')
    try {
        Invoke-Step 'assetcheck check' { & $py tools/assetcheck.py check }
        Invoke-Step 'assetcheck validate' { & $py tools/assetcheck.py validate }
        Invoke-Step 'agent-core-assets pytest' { & $py -m pytest -c pytest.ini tests -p no:cacheprovider }
    } finally { Pop-Location }
}
function Job-Scan {
    Ensure-Venv; Need-Postgres
    Push-Location $bridge
    try { Invoke-Step 'sys.modules testing scan' { & $py -m pytest -c pyproject.toml tests/runtime/test_review_final_pg.py -k never_imports -p no:cacheprovider } } finally { Pop-Location }
}

switch ($Job) {
    'rust' { Job-Rust }
    'contract-drift' { Job-Drift }
    'mock-wire' { Job-Mock }
    'a2-wire' { Job-A2 }
    'real-wire' { Job-Real }
    'lint' { Job-Lint }
    'core-bridge' { Job-CoreBridge }
    'platform-sim' { Job-PlatformSim }
    'agent-core-assets' { Job-Assets }
    'modules-scan' { Job-Scan }
    'all' { Job-Lint; Job-Drift; Job-Mock; Job-A2; Job-Real; Job-CoreBridge; Job-PlatformSim; Job-Assets; Job-Scan; Job-Rust }
}
Write-Output '--- ci summary'
$results | ForEach-Object { Write-Output $_ }

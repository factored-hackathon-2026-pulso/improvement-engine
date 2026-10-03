# Pester (v3-compatible syntax) smoke test of the wrapper: fake gh fixtures, exit-code contract.
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$wrapper = Join-Path (Split-Path -Parent $here) 'pin-watch.ps1'

Describe 'pin-watch.ps1' {
    $root = Join-Path ([IO.Path]::GetTempPath()) ('pinwatch-pester-' + [guid]::NewGuid().ToString('N'))
    $sha = '789d6c89b2fca90fc10e2abf157da51dc81c5d51'
    $wire = Join-Path $root 'core-bridge\wire\agent_core@789d6c8'
    New-Item -ItemType Directory -Force $wire | Out-Null
    ('{"sha":"' + $sha + '","contract_version":"1.3.0","files":[]}') | Set-Content (Join-Path $wire 'MANIFEST.json')
    $fx = Join-Path $root 'fx'
    New-Item -ItemType Directory -Force $fx | Out-Null
    ('{"sha":"' + $sha + '"}') | Set-Content (Join-Path $fx 'repos_pulso-factored_agent-core_commits_main.json')
    '[]' | Set-Content (Join-Path $fx 'repos_pulso-factored_agent-core_pulls_state=open&per_page=100.json')
    ('{"sha":"' + ('9' * 40) + '"}') | Set-Content (Join-Path $fx 'repos_pulso-factored_llm-gateway_commits_main.json')

    It 'exits 0 with verdict no-change when upstream == pin' {
        $out = & $wrapper -- --repo-root $root --json --no-checks --gh-fixtures $fx --out-dir (Join-Path $root 'out') --state-file (Join-Path $root 'st.json') | Out-String
        $LASTEXITCODE | Should Be 0
        ($out | ConvertFrom-Json).verdict | Should Be 'no-change'
    }
    It 'exits 1 when there is no pin' {
        & $wrapper -- --repo-root (Join-Path $root 'nowhere') --json --no-checks 2>$null | Out-Null
        $LASTEXITCODE | Should Be 1
    }
}

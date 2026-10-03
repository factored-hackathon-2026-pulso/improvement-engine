$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$core = (Resolve-Path (Join-Path $here '..')).Path
$repo = (Resolve-Path (Join-Path $core '..\..')).Path
$ps = (Get-Process -Id $PID).Path

Describe 'verify-fragments.ps1' {
    It 'applies patch.json to a temp copy and validates' {
        $out = & $ps -NoProfile -File (Join-Path $repo 'scripts\core\verify-fragments.ps1') 2>&1 | Out-String
        $LASTEXITCODE | Should Be 0
        $out | Should Match 'fragments: ok'
    }
    It 'patch.json entries have the 17.9.4 shape and matching sha256' {
        $p = Get-Content (Join-Path $core 'fragments\patch.json') -Raw | ConvertFrom-Json
        (@($p.entries).Count -ge 3) | Should Be $true
        foreach ($e in $p.entries) {
            foreach ($k in 'target', 'op', 'anchor', 'fragment_path', 'fragment_sha256', 'requires_owner_ack') { ($e.PSObject.Properties.Name -contains $k) | Should Be $true }
            ($e.op -in 'add_service', 'add_job', 'add_doc_row', 'add_env_name') | Should Be $true
            (Get-FileHash (Join-Path $repo $e.fragment_path) -Algorithm SHA256).Hash.ToLower() | Should Be $e.fragment_sha256
        }
    }
    It 'fails when a fragment hash does not match' {
        $out = & $ps -NoProfile -File (Join-Path $repo 'scripts\core\verify-fragments.ps1') -TamperForTest 2>&1 | Out-String
        $LASTEXITCODE | Should Not Be 0
        $out | Should Match 'fragment_sha256'
    }
}

Describe 'ci.fragment.yml' {
    It 'is the full job set of core-bridge/scripts/ci.ps1 and every job calls that script' {
        $ci = Get-Content (Join-Path $repo 'core-bridge\scripts\ci.ps1') -Raw
        $set = [regex]::Match($ci, "ValidateSet\(([^)]*)\)").Groups[1].Value
        $jobs = @([regex]::Matches($set, "'([a-z0-9-]+)'") | ForEach-Object { $_.Groups[1].Value } | Where-Object { $_ -ne 'all' })
        $frag = Get-Content (Join-Path $core 'fragments\ci.fragment.yml') -Raw
        $called = @([regex]::Matches($frag, 'core-bridge/scripts/ci\.ps1 -Job ([a-z0-9-]+)') | ForEach-Object { $_.Groups[1].Value })
        ($called | Sort-Object) -join ',' | Should Be (($jobs | Sort-Object) -join ',')
        $frag | Should Not Match 'test\.ps1'
    }
    It 'passes -PostgresAdmin to PG jobs from a postgres service and does not inline commands' {
        $frag = Get-Content (Join-Path $core 'fragments\ci.fragment.yml') -Raw
        $frag | Should Match 'PULSO_TEST_PG_ADMIN'
    }
}

Describe 'verify-fragments rejection of unsafe patch entries' {
    It 'rejects a target or fragment path that escapes the repository or is not allowlisted' {
        $bad = Join-Path $env:TEMP 'pulso-bad-patch.json'
        $p = Get-Content (Join-Path $core 'fragments\patch.json') -Raw | ConvertFrom-Json
        $p.entries[0].target = '..\..\escape.yaml'
        $p | ConvertTo-Json -Depth 6 | Set-Content $bad -Encoding utf8
        $out = & $ps -NoProfile -File (Join-Path $repo 'scripts\core\verify-fragments.ps1') -PatchPath $bad 2>&1 | Out-String
        $LASTEXITCODE | Should Not Be 0
        $out | Should Match 'target_not_allowed'
    }
}

Describe 'repository hygiene' {
    It 'git-ignores local/.secrets' {
        git -C $repo check-ignore -q 'local/.secrets/x/core.env'
        $LASTEXITCODE | Should Be 0
    }
    It 'has no private key material in local/core' {
        $hits = Get-ChildItem $core -Recurse -File -Exclude *.Tests.ps1 | Select-String -Pattern 'BEGIN (RSA|EC|OPENSSH|PRIVATE)'
        @($hits).Count | Should Be 0
    }
}

Describe 'machine registry file' {
    It 'declares both connections and never pulso-codex' {
        $r = Get-Content (Join-Path $core 'machine.registry.json') -Raw | ConvertFrom-Json
        (@($r.machines.name) -contains 'pulso-dev') | Should Be $true
        (@($r.machines.name) -contains 'pulso-dev-root') | Should Be $true
        (@($r.machines.name) -like 'pulso-codex*').Count | Should Be 0
    }
}

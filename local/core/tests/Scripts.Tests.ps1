$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$core = (Resolve-Path (Join-Path $here '..')).Path
$ps = (Get-Process -Id $PID).Path
$sec = Join-Path $env:TEMP 'pulso-core-test-secrets'
function Invoke-Script($name, [string[]]$a) {
    $out = & $ps -NoProfile -File (Join-Path $core $name) @a 2>&1 | Out-String
    [pscustomobject]@{ code = $LASTEXITCODE; out = $out }
}

Describe 'start.ps1 (dry run, no container engine touched)' {
    It 'clean clone: plans start and generates secrets without printing values' {
        Remove-Item -Recurse -Force $sec -ErrorAction SilentlyContinue
        $r = Invoke-Script 'start.ps1' @('-Namespace', 'claude-testclean', '-Profile', 'real_local', '-DryRun', '-SecretsRoot', $sec, '-PortBase', '18990')
        $r.code | Should Be 0
        $r.out | Should Match '--connection pulso-dev'
        $r.out | Should Match 'pulso-claude-testclean'
        $r.out | Should Match '127.0.0.1:'
        $f = Join-Path $sec 'claude-testclean\core.env'
        Test-Path $f | Should Be $true
        $pw = (Get-Content $f | Where-Object { $_ -like 'POSTGRES_PASSWORD=*' }) -replace '^POSTGRES_PASSWORD=', ''
        ($pw.Length -ge 24) | Should Be $true
        $r.out.Contains($pw) | Should Be $false
    }
    It 'second run keeps the same secrets (idempotent)' {
        $f = Join-Path $sec 'claude-testclean\core.env'
        $h1 = (Get-FileHash $f).Hash
        $null = Invoke-Script 'start.ps1' @('-Namespace', 'claude-testclean', '-Profile', 'real_local', '-DryRun', '-SecretsRoot', $sec, '-PortBase', '18990')
        (Get-FileHash $f).Hash | Should Be $h1
        Remove-Item -Recurse -Force $sec
    }
    It 'refuses a machine that is not registered (machine_not_registered)' {
        $r = Invoke-Script 'start.ps1' @('-Namespace', 'claude-t1', '-Profile', 'fixture', '-Machine', 'pulso-codex', '-DryRun', '-SecretsRoot', $sec)
        $r.code | Should Not Be 0
        $r.out | Should Match 'pulso:machine_not_registered'
    }
    It 'refuses an occupied explicit port base (port_conflict, exit 4)' {
        $l = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Parse('127.0.0.1'), 18971); $l.Start()
        try {
            $r = Invoke-Script 'start.ps1' @('-Namespace', 'claude-t2', '-Profile', 'fixture', '-DryRun', '-PortBase', '18970', '-SecretsRoot', $sec)
            $r.code | Should Be 4
            $r.out | Should Match 'pulso:port_conflict'
        } finally { $l.Stop(); Remove-Item -Recurse -Force $sec -ErrorAction SilentlyContinue }
    }
    It 'refuses a namespace that is not claude-*' {
        $r = Invoke-Script 'start.ps1' @('-Namespace', 'codex-h1', '-Profile', 'fixture', '-DryRun', '-SecretsRoot', $sec)
        $r.code | Should Not Be 0
        $r.out | Should Match 'pulso:namespace_not_owned'
    }
    It 'fixture profile declares doubles and contract_mock' {
        $r = Invoke-Script 'start.ps1' @('-Namespace', 'claude-t3', '-Profile', 'fixture', '-DryRun', '-PortBase', '18960', '-SecretsRoot', $sec)
        $r.code | Should Be 0
        $r.out | Should Match 'runtime_profile=contract_mock'
        $r.out | Should Match 'platform-sim'
        Remove-Item -Recurse -Force $sec -ErrorAction SilentlyContinue
    }
}

Describe 'reset.ps1' {
    It 'is destructive only with -Confirm' {
        $r = Invoke-Script 'reset.ps1' @('-Namespace', 'claude-t1', '-DryRun')
        $r.code | Should Not Be 0
        $r.out | Should Match 'pulso:confirm_required'
    }
    It 'refuses a namespace of another team' {
        $r = Invoke-Script 'reset.ps1' @('-Namespace', 'codex-h1', '-Confirm', '-DryRun')
        $r.code | Should Not Be 0
        $r.out | Should Match 'pulso:namespace_not_owned'
    }
    It 'scopes the removal to the project labels' {
        $r = Invoke-Script 'reset.ps1' @('-Namespace', 'claude-t1', '-Confirm', '-DryRun')
        $r.code | Should Be 0
        $r.out | Should Match 'label=com.pulso.namespace=claude-t1'
        $r.out | Should Match 'label=com.pulso.team=claude'
    }
}

Describe 'doctor executor key check' {
    It 'reports bridge_executor_key (skipped without engine)' {
        $r = Invoke-Script 'doctor.core.ps1' @('-Json', '-SkipEngine')
        $c = @($r.out | ConvertFrom-Json) | Where-Object check -eq 'bridge_executor_key'
        $c.status | Should Be 'skipped'
    }
}

Describe 'doctor.core.ps1 -Json' {
    It 'emits an array of {check,status,code,detail}' {
        $r = Invoke-Script 'doctor.core.ps1' @('-Json', '-SkipEngine')
        $j = @($r.out | ConvertFrom-Json)
        ($j.Count -ge 5) | Should Be $true
        foreach ($k in 'check', 'status', 'code', 'detail') { ($j[0].PSObject.Properties.Name -contains $k) | Should Be $true }
    }
    It 'reports machine_not_registered for an unregistered machine' {
        $r = Invoke-Script 'doctor.core.ps1' @('-Json', '-SkipEngine', '-Machine', 'pulso-codex')
        @((@($r.out | ConvertFrom-Json)) | Where-Object { $_.code -eq 'machine_not_registered' }).Count | Should Be 1
    }
    It 'reports core_checkout_wrong_sha when the checkout is not the pin' {
        $r = Invoke-Script 'doctor.core.ps1' @('-Json', '-SkipEngine', '-Checkout', $env:TEMP)
        @((@($r.out | ConvertFrom-Json)) | Where-Object { $_.code -eq 'core_checkout_wrong_sha' }).Count | Should Be 1
    }
    It 'passes the pin check against the reference checkout' {
        $r = Invoke-Script 'doctor.core.ps1' @('-Json', '-SkipEngine')
        (@($r.out | ConvertFrom-Json) | Where-Object { $_.check -eq 'agent_core_pin' }).status | Should Be 'pass'
    }
}

Describe 'smoke.ps1' {
    It 'fails with core_not_ready when the runtime is unreachable' {
        $r = Invoke-Script 'smoke.ps1' @('-Namespace', 'claude-t1', '-BaseUrl', 'http://127.0.0.1:1', '-TimeoutSec', '2')
        $r.code | Should Not Be 0
        $r.out | Should Match 'pulso:core_not_ready'
    }
    It 'passes against a stub runtime and writes an honest report (no real_local claim without digest)' {
        $port = Get-Random -Minimum 18930 -Maximum 18950
        $job = Start-Job -ArgumentList $port -ScriptBlock {
            param($port)
            $l = [System.Net.HttpListener]::new(); $l.Prefixes.Add("http://127.0.0.1:$port/"); $l.Start()
            $end = (Get-Date).AddSeconds(40)
            $t = $null
            while ((Get-Date) -lt $end) {
                if ($null -eq $t) { $t = $l.GetContextAsync() }
                if (-not $t.Wait(500)) { continue }
                $c = $t.Result; $t = $null; $p = $c.Request.Url.AbsolutePath; $body = '{}'
                if ($p -eq '/internal/v1/version') { $body = '{"agent_core_sha":"789d6c89b2fca90fc10e2abf157da51dc81c5d51","contracts_version":"1.3.0","runtime_profile":"agent_core_real","doubles":["tools: stand-in"],"image_digest":"unknown"}' }
                if ($p -eq '/_sim/info') { $c.Response.StatusCode = 404 }
                $c.Response.ContentType = 'application/json'; $b = [Text.Encoding]::UTF8.GetBytes($body); $c.Response.OutputStream.Write($b, 0, $b.Length); $c.Response.Close()
            }
            $l.Stop()
        }
        Start-Sleep -Milliseconds 1000
        try {
            $rep = Join-Path $env:TEMP 'pulso-smoke-report.json'
            $r = Invoke-Script 'smoke.ps1' @('-Namespace', 'claude-t1', '-BaseUrl', "http://127.0.0.1:$port", '-ReportPath', $rep, '-SkipSeedCheck', '-SkipExporter', '-TimeoutSec', '5')
            $r.out | Should Match 'smoke: pass'
            $j = Get-Content $rep -Raw | ConvertFrom-Json
            $j.runtime_profile | Should Be 'agent_core_real'
            $j.agent_core_sha | Should Be '789d6c89b2fca90fc10e2abf157da51dc81c5d51'
            (@($j.doubles).Count -ge 1) | Should Be $true
            $j.target | Should Not Be 'real_local'
        } finally { Stop-Job $job -ErrorAction SilentlyContinue; Remove-Job $job -Force -ErrorAction SilentlyContinue }
    }
}

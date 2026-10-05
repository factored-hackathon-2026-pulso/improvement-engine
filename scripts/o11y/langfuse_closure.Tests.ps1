# Pester 3.x (host version; also runs under pwsh). Offline: the pure parts of scripts/o11y/langfuse_closure.ps1
# (plan, settings, env-file validation by NAMES, redaction, stack environment) and a canary for credential handling.
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
. (Join-Path $here 'langfuse_closure.lib.ps1')

Describe 'Get-LfcPlan' {
    It 'refuses an empty command line' { { Get-LfcPlan } | Should Throw 'nothing to do' }
    It '-All is Models Up Traffic Verify Down in that order' { ((Get-LfcPlan -All).Steps -join ',') | Should Be 'Models,Up,Traffic,Verify,Down' }
    It '-All -KeepUp leaves the stack up' { ((Get-LfcPlan -All -KeepUp).Steps -join ',') | Should Be 'Models,Up,Traffic,Verify' }
    It 'runs partial steps in the fixed order' { ((Get-LfcPlan -Verify -Traffic).Steps -join ',') | Should Be 'Traffic,Verify' }
    It 'refuses -Up with -Down, -Down with -Traffic, -KeepUp with -Down' {
        { Get-LfcPlan -Up -Down } | Should Throw 'contradict'
        { Get-LfcPlan -Down -Traffic } | Should Throw 'cannot be combined'
        { Get-LfcPlan -KeepUp -Down } | Should Throw 'contradict'
    }
    It 'accepts -Down alone' { ((Get-LfcPlan -Down).Steps -join ',') | Should Be 'Down' }
}

Describe 'Get-LfcSettings' {
    It 'has its own prefix and distinct ports' {
        $s = Get-LfcSettings
        $s.Prefix | Should Be 'pulso-lfc'
        @($s.PgPort, $s.GwPort, $s.CorePort, $s.EnginePort, $s.ForwarderPort, $s.MockPort | Select-Object -Unique).Count | Should Be 6
        $s.ForwarderUrl | Should Be 'http://127.0.0.1:4328'
    }
    It 'refuses the shared prefixes and duplicate ports' {
        { Get-LfcSettings -Prefix 'pulso-l3' } | Should Throw 'belongs to another'
        { Get-LfcSettings -Prefix 'pulso-demo' } | Should Throw 'belongs to another'
        { Get-LfcSettings -GwPort 8211 } | Should Throw 'different'
    }
}

Describe 'Test-LangfuseEnvFile and credential handling' {
    $pk = 'pk-lf-CANARY' + [guid]::NewGuid().ToString('N')
    $sk = 'sk-lf-CANARY' + [guid]::NewGuid().ToString('N')
    $dir = Join-Path ([IO.Path]::GetTempPath()) ('lfc-' + [guid]::NewGuid().ToString('N'))
    $null = New-Item -ItemType Directory -Path $dir
    $good = Join-Path $dir 'good.env'
    Set-Content -LiteralPath $good -Value @("LANGFUSE_SECRET_KEY=$sk", "LANGFUSE_PUBLIC_KEY=$pk", 'LANGFUSE_BASE_URL=https://us.cloud.langfuse.com') -Encoding ASCII
    $partial = Join-Path $dir 'partial.env'
    Set-Content -LiteralPath $partial -Value @("LANGFUSE_SECRET_KEY=$sk", 'LANGFUSE_PUBLIC_KEY=', 'LANGFUSE_BASE_URL=https://us.cloud.langfuse.com') -Encoding ASCII
    $evil = Join-Path $dir 'evil.env'
    Set-Content -LiteralPath $evil -Value @("LANGFUSE_SECRET_KEY=$sk", "LANGFUSE_PUBLIC_KEY=$pk", 'LANGFUSE_BASE_URL=https://collector.example.org') -Encoding ASCII
    $plain = Join-Path $dir 'plain.env'
    Set-Content -LiteralPath $plain -Value @("LANGFUSE_SECRET_KEY=$sk", "LANGFUSE_PUBLIC_KEY=$pk", 'LANGFUSE_BASE_URL=http://127.0.0.1:4399') -Encoding ASCII

    It 'accepts a file with the 3 key names and returns the host only' {
        $r = Test-LangfuseEnvFile -Path $good
        $r.Ok | Should Be $true
        $r.Host | Should Be 'us.cloud.langfuse.com'
        (($r | ConvertTo-Json -Depth 4).Contains($sk)) | Should Be $false
    }
    It 'names a missing or blank key and never a value' {
        $r = Test-LangfuseEnvFile -Path $partial
        $r.Ok | Should Be $false
        ($r.Missing -join ',') | Should Be 'LANGFUSE_PUBLIC_KEY'
        (($r | ConvertTo-Json -Depth 4).Contains($sk)) | Should Be $false
    }
    It 'refuses a missing file, a host that is not Langfuse and plain http outside the mock' {
        (Test-LangfuseEnvFile -Path (Join-Path $dir 'nope.env')).Ok | Should Be $false
        (Test-LangfuseEnvFile -Path $evil).Ok | Should Be $false
        (Test-LangfuseEnvFile -Path $plain).Ok | Should Be $false
        (Test-LangfuseEnvFile -Path $plain -AllowHttpLoopback).Ok | Should Be $true
    }
    It 'CANARY: needles mask the keys and the Basic header; the stack environment carries no credential' {
        $v = Read-EnvFileValues -Path $good
        $needles = Get-LfcNeedles -LangfuseValues $v
        $basic = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes("${pk}:${sk}"))
        $out = Protect-Text -Text "a $sk b $pk c Basic $basic" -Needles $needles
        $out.Contains($sk) | Should Be $false
        $out.Contains($pk) | Should Be $false
        $out.Contains($basic) | Should Be $false
        $envText = ((Get-LfcStackEnvironment -Settings (Get-LfcSettings)).GetEnumerator() | ForEach-Object { "$($_.Key)=$($_.Value)" }) -join "`n"
        $envText.Contains($sk) | Should Be $false
        $envText.Contains($pk) | Should Be $false
        $envText.Contains('LANGFUSE_SECRET') | Should Be $false
    }
    It 'CANARY: a child that dumps its environment never shows the values, and this process never holds them' {
        $v = Read-EnvFileValues -Path $good
        $needles = Get-LfcNeedles -LangfuseValues $v
        $r = Invoke-Scrubbed -File $env:ComSpec -Arguments @('/c', 'set') -Env $v -Needles $needles -Quiet
        $text = $r.Output -join "`n"
        $text.Contains($sk) | Should Be $false
        $text.Contains($pk) | Should Be $false
        ($text -match 'LANGFUSE_SECRET_KEY=\*\*\*') | Should Be $true
        [Environment]::GetEnvironmentVariable('LANGFUSE_SECRET_KEY') | Should BeNullOrEmpty
    }
    It 'CANARY: the real script, run with a bad command line from another directory, exits 2 and prints no value' {
        $script = Join-Path $here 'langfuse_closure.ps1'
        $exe = (Get-Process -Id $PID).Path
        Push-Location ([IO.Path]::GetTempPath())
        try {
            $r = Invoke-Scrubbed -File $exe -Arguments @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $script, '-EnvFile', $good) -Env @{} -Needles @($sk, $pk) -Quiet
        } finally { Pop-Location }
        $r.ExitCode | Should Be 2
        (($r.Output -join "`n") -match 'nothing to do') | Should Be $true
    }
    Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
}

Describe 'Get-StackEnvironment for the closure' {
    It 'points both exporters at the loopback forwarder with content on' {
        $e = Get-LfcStackEnvironment -Settings (Get-LfcSettings)
        $e['OTEL_EXPORTER_OTLP_ENDPOINT'] | Should Be 'http://127.0.0.1:4328'
        $e['OTEL_EXPORTER_OTLP_PROTOCOL'] | Should Be 'http/protobuf'
        $e['LLM_GATEWAY_TRACE_CONTENT'] | Should Be '1'
        $e['AGENTCORE_TRACE_CONTENT'] | Should Be '1'
        $e['AGENTCORE_TRACE_LANGFUSE'] | Should Be '1'
        $e['PULSO_GW_HOST_NETWORK'] | Should Be '1'
    }
}

Describe 'Story ids, findings and hints' {
    It 'reads the trace ids engine_trace.py prints, once each' {
        $lines = @('story finding=0 trace_id=7e1ffba44834058839ef1a914c474128 spans=13 outcome=x scores=[]', 'story finding=1 trace_id=0123456789abcdef0123456789abcdef spans=9 outcome=y scores=[]',
            'story finding=1 trace_id=0123456789abcdef0123456789abcdef spans=9 outcome=y scores=[]', 'target=forwarder capture_content=True stories=2 scores_sent=8')
        (Get-StoryTraceIds -Lines $lines).Count | Should Be 2
    }
    It 'counts the findings that have an evidence ref' {
        $l = '{"findings":[{"evidence_ref":"ev_1"},{"evidence_ref":"ev_2"},{"finding_id":"x"}]}' | ConvertFrom-Json
        Get-LoopFindingCount -Loop $l | Should Be 2
    }
    It 'explains agent-core unreachable as a stack that is not up' {
        (Get-StackDownHint 'agent-core unreachable: URLError') -match 'stack is not up' | Should Be $true
        Get-StackDownHint 'something else' | Should Be ''
    }
    It 'diagnoses the health ping' {
        Get-HealthDiagnosis 'langfuse health: HTTP 200 OK' | Should Be 'ok'
        (Get-HealthDiagnosis 'unreachable (URLError)') -match 'not reachable' | Should Be $true
        (Get-HealthDiagnosis 'HTTP 404') -match 'region' | Should Be $true
    }
}

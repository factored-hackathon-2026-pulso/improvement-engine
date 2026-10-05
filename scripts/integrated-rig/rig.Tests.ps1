# Pester 3.x (host version; also runs under pwsh). Offline: the pure parts of scripts/integrated-rig (settings, environments, memory gate,
# evidence selection, reality table) and the CANARY: no environment value is ever printed. No network, no stack, no model.
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
. (Join-Path $here 'rig.lib.ps1')

Describe 'Get-RigSettings' {
    It 'uses its own prefix and ports, all different' {
        $s = Get-RigSettings
        $s.Prefix | Should Be 'pulso-env1'
        @($s.PgPort, $s.GwPort, $s.CorePort, $s.EnginePort, $s.PlatformPort, $s.BatteryPg, $s.BatteryGw, $s.BatteryCore | Select-Object -Unique).Count | Should Be 8
    }
    It 'refuses the shared and demo prefixes' {
        { Get-RigSettings -Prefix 'pulso-l3' } | Should Throw 'shared'
        { Get-RigSettings -Prefix 'pulso-demo' } | Should Throw 'demo loop'
    }
    It 'refuses a platform port that collides with a stack port' {
        { Get-RigSettings -PlatformPort 8210 } | Should Throw 'different'
    }
}

Describe 'Get-RigSettings overrides (AGT1: a lane runs its own rig beside ENV1)' {
    It 'reads prefix and ports from PULSO_STACK_PREFIX and PULSO_RIG_*_PORT' {
        $keep = @{}
        $names = 'PULSO_STACK_PREFIX', 'PULSO_RIG_PG_PORT', 'PULSO_RIG_GW_PORT', 'PULSO_RIG_CORE_PORT', 'PULSO_RIG_ENGINE_PORT', 'PULSO_RIG_PLATFORM_PORT', 'PULSO_RIG_SPA_PORT'
        foreach ($n in $names) { $keep[$n] = [Environment]::GetEnvironmentVariable($n) }
        try {
            $env:PULSO_STACK_PREFIX = 'agt1'; $env:PULSO_RIG_PG_PORT = '55540'; $env:PULSO_RIG_GW_PORT = '8240'; $env:PULSO_RIG_CORE_PORT = '8241'
            $env:PULSO_RIG_ENGINE_PORT = '4240'; $env:PULSO_RIG_PLATFORM_PORT = '8245'; $env:PULSO_RIG_SPA_PORT = '5184'
            $s = Get-RigSettings
            $s.Prefix | Should Be 'agt1'
            $s.PgPort | Should Be 55540
            $s.CorePort | Should Be 8241
            $s.PlatformPort | Should Be 8245
            $s.SpaPort | Should Be 5184
            (Get-PlatformEnvironment -Settings $s -KeysFile 'k' -ServiceToken 't' -DbPath 'd')['CC_CORS_ORIGINS'] | Should Match 'localhost:5184'
        } finally { foreach ($n in $names) { [Environment]::SetEnvironmentVariable($n, $keep[$n]) } }
    }
    It 'keeps the ENV1 defaults and SPA port 5174 without overrides' {
        $s = Get-RigSettings
        $s.SpaPort | Should Be 5174
    }
}

Describe 'Test-RamBudget' {
    It 'allows above the minimum only' {
        (Test-RamBudget -FreeMb 1501 -MinMb 1500) | Should Be $true
        (Test-RamBudget -FreeMb 1500 -MinMb 1500) | Should Be $false
        (Test-RamBudget -FreeMb 900) | Should Be $false
    }
    It 'never allows an unreadable value' { (Test-RamBudget -FreeMb -1 -MinMb 0) | Should Be $false }
}

Describe 'environments' {
    $s = Get-RigSettings
    It 'platform env wires agent-core, the keys file, the service token and a fresh SQLite file' {
        $e = Get-PlatformEnvironment -Settings $s -KeysFile 'D:\k\private.json' -ServiceToken 'svc-token-value' -DbPath 'D:\x\cc.db'
        $e['CC_AGENT_CORE_URL'] | Should Be 'http://127.0.0.1:8211'
        $e['CC_AGENT_KEYS_FILE'] | Should Be 'D:\k\private.json'
        $e['CC_INTERNAL_SERVICE_TOKEN'] | Should Be 'svc-token-value'
        $e['CC_DATABASE_URL'] | Should Be 'sqlite+aiosqlite:///D:/x/cc.db'
        $e['CC_PORT'] | Should Be '8200'
        $e['CC_CORS_ORIGINS'] | Should Match 'localhost:5174'
    }
    It 'agent-core grants env points at the platform with the same token' {
        $e = Get-AgentCoreGrantsEnvironment -Settings $s -ServiceToken 'svc-token-value'
        $e['AGENTCORE_GRANTS_URL'] | Should Be 'http://127.0.0.1:8200'
        $e['AGENTCORE_GRANTS_TOKEN'] | Should Be 'svc-token-value'
    }
    It 'loop lane env carries the prefix and the four ports the demo loop reads' {
        $e = Get-LoopLaneEnvironment -Settings $s
        $e['PULSO_STACK_PREFIX'] | Should Be 'pulso-env1'
        $e['PULSO_DEMO_CORE_PORT'] | Should Be '8211'
        $e['PULSO_DEMO_ENGINE_PORT'] | Should Be '4210'
    }
    It 'engine env announces to the platform only when URL and token are given' {
        $off = Get-EngineEnvironment -Settings $s -CellsPath 'c' -WorkDir 'w' -StoreDir 's' -Source 'synthetic'
        $off['PULSO_ANNOUNCE_TO_PLATFORM'] | Should Be 'off'
        $on = Get-EngineEnvironment -Settings $s -CellsPath 'c' -WorkDir 'w' -StoreDir 's' -Source 'synthetic' -PlatformUrl 'http://127.0.0.1:8200' -PlatformToken 'svc-token-value'
        $on['PULSO_ANNOUNCE_TO_PLATFORM'] | Should Be 'on'
        $on['PULSO_PLATFORM_SERVICE_TOKEN'] | Should Be 'svc-token-value'
    }
}

Describe 'New-ServiceToken' {
    It 'is random, long and prefixed' {
        $a = New-ServiceToken; $b = New-ServiceToken
        $a | Should Not Be $b
        $a.Length -gt 40 | Should Be $true
        $a | Should Match '^svc-'
    }
}

Describe 'Select-EvidenceLinks' {
    $ids = @('CASE-00000000000000000000000104', 'CASE-00000000000000000000000107', 'CASE-00000000000000000000000112', 'not-a-case', 'CASE-IIIIIIIIIIIIIIIIIIIIIIIIII')
    It 'keeps only CASE- ids and is deterministic per evidence ref' {
        $a = @(Select-EvidenceLinks -CaseIds $ids -EvidenceRef 'ev_1' -Count 2)
        $b = @(Select-EvidenceLinks -CaseIds $ids -EvidenceRef 'ev_1' -Count 2)
        $a.Count | Should Be 2
        ($a -join ',') | Should Be ($b -join ',')
        foreach ($x in $a) { $x | Should Match '^CASE-[0-9A-HJKMNP-TV-Z]{26}$' }
    }
    It 'caps at 8 and at the number available, and returns nothing without valid ids' {
        @(Select-EvidenceLinks -CaseIds $ids -EvidenceRef 'e' -Count 50).Count | Should Be 3
        @(Select-EvidenceLinks -CaseIds @('x', 'y') -EvidenceRef 'e').Count | Should Be 0
    }
}

Describe 'New-AnnouncePayload with explicit evidence links' {
    $rec = [pscustomobject]@{ outcome = 'announced'; evidence_ref = 'ev_9'; delivery = [pscustomobject]@{ proposal_id = 'prop-1' }
        evaluation = [pscustomobject]@{ dossier = [pscustomobject]@{ es = [pscustomobject]@{ title = 'T'; description = 'D'; sections = [pscustomobject]@{ problem = 'p'; evidence = 'e'; expected_effect = 'x' } } } } }
    It 'uses the given CASE- ids' {
        $p = New-AnnouncePayload -Record $rec -EvidenceLinks @('CASE-00000000000000000000000104')
        @($p.evidenceLinks).Count | Should Be 1
        $p.evidenceLinks[0] | Should Be 'CASE-00000000000000000000000104'
    }
    It 'falls back to the opaque derived id' {
        $p = New-AnnouncePayload -Record $rec
        $p.evidenceLinks[0] | Should Match '^CASE-'
    }
}

Describe 'Format-RealityTable' {
    It 'prints every row with a closed class vocabulary' {
        $lines = Format-RealityTable -Rows (Get-RigRealityRows)
        $lines.Count | Should Be ((Get-RigRealityRows).Count + 1)
        (($lines -join "`n") -match 'real') | Should Be $true
        (($lines -join "`n") -match 'stand-in') | Should Be $true
        (($lines -join "`n") -match 'relaxed') | Should Be $true
    }
    It 'refuses an unknown class' { { Format-RealityTable -Rows @(@{ Item = 'x'; Class = 'maybe'; Note = 'n' }) } | Should Throw 'stand-in' }
}

Describe 'CANARY: no environment value is ever printed' {
    $canary = 'CANARY-SECRET-VALUE-0123456789'
    $envFile = Join-Path ([IO.Path]::GetTempPath()) ('rig-canary-' + [guid]::NewGuid().ToString('N') + '.env')
    [IO.File]::WriteAllText($envFile, "GATEWAY_TOKEN_AGENT_CORE=$canary`nAGENTCORE_REGISTRY_DSN=postgresql://user:dsnpassword-9876543210@127.0.0.1:55510/agentcore`n")
    $vals = Read-EnvFileValues -Path $envFile
    $needles = Get-RedactionNeedles -Sources @($vals)
    It 'Protect-Text masks the value, the DSN password and fragments' {
        $t = Protect-Text -Text "token=$canary dsn=postgresql://user:dsnpassword-9876543210@host" -Needles $needles
        (Test-OutputClean -Text $t -Secrets @($canary, 'dsnpassword-9876543210')) | Should Be $true
        $t | Should Match '\*\*\*'
    }
    It 'Test-OutputClean detects a leak' { (Test-OutputClean -Text "x $canary y" -Secrets @($canary)) | Should Be $false }
    It 'a scrubbed child that prints the secret and the whole environment shows none of it' {
        $shell = Get-ChildShell
        $r = Invoke-Scrubbed -File $shell -Arguments @('-NoProfile', '-Command', 'Write-Output $env:RIG_CANARY; Get-ChildItem env: | ForEach-Object { $_.Name + "=" + $_.Value }') `
            -Env @{ RIG_CANARY = $canary } -Needles $needles -Quiet
        $out = $r.Output -join "`n"
        (Test-OutputClean -Text $out -Secrets @($canary)) | Should Be $true
        $out | Should Match 'RIG_CANARY'
    }
    It 'Get-EnvNames returns names only' {
        $names = (Get-EnvNames -Env @{ RIG_CANARY = $canary; B = 'two-value' }) -join ','
        $names | Should Be 'B,RIG_CANARY'
        (Test-OutputClean -Text $names -Secrets @($canary, 'two-value')) | Should Be $true
    }
    It 'the scripts never echo a variable value: no Write-Host/Say of an env member in the rig scripts' {
        $bad = @()
        foreach ($f in 'up.ps1', 'down.ps1', 'health.ps1', 'run_story.ps1') {
            $text = [IO.File]::ReadAllText((Join-Path $here $f))
            foreach ($m in [regex]::Matches($text, '(?m)^\s*(Say|Write-Host)\b[^\r\n]*\$(penv|eenv|envUp|acEnv|gwEnv|secretAll|svc|tok)\b[^\r\n]*$')) {
                if ($m.Value -notmatch 'Get-EnvNames') { $bad += "$f : $($m.Value.Trim())" }
            }
        }
        ($bad -join "`n") | Should Be ''
    }
    Remove-Item -LiteralPath $envFile -Force -ErrorAction SilentlyContinue
}


Describe 'Save-Hops (AGT1: run_story ended with "Argument types do not match" and wrote no hops json)' {
    It 'writes the hops list held in a List[object]' {
        $hops = New-Object System.Collections.Generic.List[object]
        $hops.Add([pscustomobject]@{ Hop = 'a'; Status = 'OK'; Detail = 'd'; Needs = '' })
        $hops.Add([pscustomobject]@{ Hop = 'b'; Status = 'not-run'; Detail = 'd'; Needs = 'x' })
        $p = Join-Path ([IO.Path]::GetTempPath()) ("hops-" + [guid]::NewGuid().ToString('N') + '.json')
        Save-Hops -Path $p -Hops $hops
        $back = @(Get-Content -Raw -LiteralPath $p | ConvertFrom-Json)
        Remove-Item -LiteralPath $p -Force
        $back.Count | Should Be 2
        $back[1].Status | Should Be 'not-run'
    }
}

# Pester 3.x (host version). Offline: the guards and pure decisions of scripts/demo-platform.ps1.
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
. (Join-Path $here 'demo-platform.lib.ps1')

Describe 'Get-MinEventsForHistory' {
    It 'covers the 14-day cold-start gate of the sensor plus a day of margin' {
        $n = Get-MinEventsForHistory -MeanGapSeconds 900
        $n | Should BeGreaterThan 15000
        $n | Should BeLessThan 17000
    }
    It 'needs more events when cases arrive faster' {
        (Get-MinEventsForHistory -MeanGapSeconds 60) | Should BeGreaterThan (Get-MinEventsForHistory -MeanGapSeconds 900)
    }
}

Describe 'Get-DemoPlatformPlan guards' {
    It 'accepts the defaults' {
        $p = Get-DemoPlatformPlan
        $p.Scenario | Should Be 'escalation_rise'
        $p.Horizon | Should BeGreaterThan $p.Backfill
        $p.Backfill | Should Not BeLessThan $p.MinEvents
    }
    It 'refuses a backfill too short for the sensor cold-start gate, naming the minimum' {
        { Get-DemoPlatformPlan -Backfill 5000 -Horizon 9000 } | Should Throw 'cold-start'
    }
    It 'refuses a horizon that is not beyond the backfill' {
        { Get-DemoPlatformPlan -Backfill 18000 -Horizon 18000 } | Should Throw '-Horizon'
    }
    It 'refuses a port outside the unprivileged range and a rate outside 1..5000' {
        { Get-DemoPlatformPlan -Port 80 } | Should Throw '-Port'
        { Get-DemoPlatformPlan -Rate 0 } | Should Throw '-Rate'
        { Get-DemoPlatformPlan -Rate 9000 } | Should Throw '-Rate'
    }
    It 'refuses an unknown scenario' {
        { Get-DemoPlatformPlan -Scenario 'magic' } | Should Throw '-Scenario'
    }
    It 'a long mean gap lowers nothing: the minimum scales with the gap' {
        { Get-DemoPlatformPlan -MeanGapSeconds 60 -Backfill 18000 -Horizon 26000 } | Should Throw 'cold-start'
    }
}

Describe 'Get-SimulatorArgs' {
    It 'runs the product stream in follow mode, stoppable by stdin EOF, over a SQLite file' {
        $a = Get-SimulatorArgs -Plan (Get-DemoPlatformPlan) -Sqlite 'C:\t\p.db'
        ($a -join ' ') | Should Match 'python -m product_stream --sqlite C:\\t\\p.db'
        $a -contains '--follow' | Should Be $true
        $a -contains '--stop-on-stdin-eof' | Should Be $true
        $a[$a.IndexOf('--mean-gap-s') + 1] | Should Be '900'
        $a[$a.IndexOf('--scenario') + 1] | Should Be 'escalation_rise'
    }
}

Describe 'Get-PulsoRunEnvironment' {
    It 'is platform mode on loopback with the product-sqlite adapter, declared simulated, scripted models and the offline core' {
        $e = Get-PulsoRunEnvironment -Plan (Get-DemoPlatformPlan -Port 4555) -Sqlite 'p.db' -WorkDir 'w' -StoreDir 's' -ConsoleDir 'c'
        $e['PULSO_DATA_MODE'] | Should Be 'platform'
        $e['PULSO_SOURCE_ADAPTER'] | Should Be 'product-sqlite'
        $e['PULSO_SOURCE_PROVENANCE'] | Should Be 'simulated'
        $e['PULSO_LISTEN_ADDR'] | Should Be '127.0.0.1:4555'
        $e['PULSO_MODEL_PORT'] | Should Be 'scripted'
        $e['PULSO_CORE_PORT'] | Should Be 'offline'
        $e['PULSO_CONSOLE_DIR'] | Should Be 'c'
    }
    It 'carries no secret: no DSN, no token, no key' {
        $e = Get-PulsoRunEnvironment -Plan (Get-DemoPlatformPlan) -Sqlite 'p.db' -WorkDir 'w' -StoreDir 's'
        ($e.Keys -join ' ') | Should Not Match 'DSN|TOKEN|KEY|SEED'
        $e.Contains('PULSO_CONSOLE_DIR') | Should Be $false
    }
}

Describe 'Get-RunListeningUrl' {
    It 'reads the base URL from the JSON listening line' {
        Get-RunListeningUrl -Line '{"event":"listening","addr":"127.0.0.1:4021"}' | Should Be 'http://127.0.0.1:4021'
    }
    It 'returns nothing for anything else, including a non-loopback address' {
        Get-RunListeningUrl -Line '{"event":"run_config"}' | Should BeNullOrEmpty
        Get-RunListeningUrl -Line 'pulso listening on http://127.0.0.1:4020' | Should BeNullOrEmpty
        Get-RunListeningUrl -Line '{"event":"listening","addr":"0.0.0.0:8080"}' | Should BeNullOrEmpty
        Get-RunListeningUrl -Line '' | Should BeNullOrEmpty
    }
}

Describe 'Format-DemoPlatformReport' {
    $doubles = @([pscustomobject]@{ id = 'data.source:simulated-operator-declared'; what = 'simulated' }, [pscustomobject]@{ id = 'port.core:offline-double'; what = 'offline double' })
    $findings = @([pscustomobject]@{ run_id = 'mon-1'; signal_id = 'reassignment_rate.pt.web_chat'; verdict = 'not_viable'; reason = 'gate_failed'; detail = 'the structural gate did not pass' })
    It 'prints the doubles FIRST, then the findings' {
        $lines = Format-DemoPlatformReport -Doubles $doubles -Findings $findings -Runs 3
        $firstDouble = [array]::IndexOf($lines, ($lines | Where-Object { $_ -like '- data.source*' } | Select-Object -First 1))
        $firstFinding = [array]::IndexOf($lines, ($lines | Where-Object { $_ -like '- mon-1*' } | Select-Object -First 1))
        $header = [array]::IndexOf($lines, ($lines | Where-Object { $_ -like 'NOT REAL*' } | Select-Object -First 1))
        $header | Should Be 0
        $firstDouble | Should BeGreaterThan $header
        $firstFinding | Should BeGreaterThan $firstDouble
        ($lines | Where-Object { $_ -like 'FINDINGS*' }) | Should Not BeNullOrEmpty
    }
    It 'lists every double and every finding verbatim' {
        $text = (Format-DemoPlatformReport -Doubles $doubles -Findings $findings) -join "`n"
        $text | Should Match 'port.core:offline-double: offline double'
        $text | Should Match 'reassignment_rate.pt.web_chat -> not_viable \(gate_failed\)'
    }
    It 'says honestly when nothing was admitted or declared' {
        $text = (Format-DemoPlatformReport -Doubles @() -Findings @()) -join "`n"
        $text | Should Match 'no signal was admitted'
        $text | Should Match 'none declared yet'
    }
    It 'never calls a finding viable on its own and carries no quality claim wording' {
        $text = (Format-DemoPlatformReport -Doubles $doubles -Findings $findings) -join "`n"
        $text | Should Not Match 'improves|quality gain'
        $text | Should Match 'never from a quality claim'
    }
}

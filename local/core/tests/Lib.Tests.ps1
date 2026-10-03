# Pester 3.x (the version shipped on this host). Legacy `Should Be` syntax.
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$lib = Join-Path $here '..\lib'
. (Join-Path $lib 'machine.ps1')
. (Join-Path $lib 'ports.ps1')
. (Join-Path $lib 'namespace.ps1')
. (Join-Path $lib 'evidence.ps1')

Describe 'machine registry' {
    It 'accepts the registered rootless connection' {
        (Assert-RegisteredMachine -Machine 'pulso-dev').connection | Should Be 'pulso-dev'
    }
    It 'accepts the registered rootful connection' {
        (Assert-RegisteredMachine -Machine 'pulso-dev-root').connection | Should Be 'pulso-dev-root'
    }
    It 'refuses the Codex machine with pulso:machine_not_registered' {
        { Assert-RegisteredMachine -Machine 'pulso-codex' } | Should Throw 'pulso:machine_not_registered'
    }
    It 'refuses an unknown machine' {
        { Assert-RegisteredMachine -Machine 'something-else' } | Should Throw 'pulso:machine_not_registered'
    }
    It 'refuses an empty machine name' {
        { Assert-RegisteredMachine -Machine '' } | Should Throw 'pulso:machine_not_registered'
    }
}

Describe 'namespaces' {
    It 'derives claude-<ms> and project pulso-claude-<ms>' {
        $ns = New-PulsoNamespace -Milliseconds 1234567
        $ns | Should Be 'claude-1234567'
        Get-PulsoProject -Namespace $ns | Should Be 'pulso-claude-1234567'
    }
    It 'rejects a namespace that is not claude-*' {
        { Assert-ClaudeNamespace -Namespace 'codex-h1' } | Should Throw 'pulso:namespace_not_owned'
        { Assert-ClaudeNamespace -Namespace 'claude-' } | Should Throw 'pulso:namespace_not_owned'
        { Assert-ClaudeNamespace -Namespace 'claude-h1;rm' } | Should Throw 'pulso:namespace_not_owned'
    }
    It 'refuses a project owned by another namespace owner' {
        $existing = @([pscustomobject]@{ project = 'pulso-claude-h1'; team = 'codex'; namespace = 'claude-h1' })
        { Assert-NamespaceOwnership -Namespace 'claude-h1' -Existing $existing } | Should Throw 'pulso:namespace_not_owned'
    }
    It 'accepts a project owned by claude' {
        $existing = @([pscustomobject]@{ project = 'pulso-claude-h1'; team = 'claude'; namespace = 'claude-h1' })
        { Assert-NamespaceOwnership -Namespace 'claude-h1' -Existing $existing } | Should Not Throw
    }
    It 'builds the mandatory labels' {
        $l = Get-PulsoLabels -Namespace 'claude-9'
        $l['com.pulso.team'] | Should Be 'claude'
        $l['com.pulso.namespace'] | Should Be 'claude-9'
    }
}

Describe 'ports' {
    It 'derives the base from crc32 in 18000..19990 and is deterministic' {
        $a = Get-PortBase -Namespace 'claude-h1'
        $a | Should Be (Get-PortBase -Namespace 'claude-h1')
        ($a -ge 18000 -and $a -le 19990) | Should Be $true
        ($a % 10) | Should Be 0
    }
    It 'matches the plan formula crc32(ns)%200*10+18000 (reference vector)' {
        # crc32('123456789') = 0xCBF43926 = 3421780262 ; %200 = 62 ; -> 18620
        Get-PortBase -Namespace '123456789' | Should Be 18620
    }
    It 'plans distinct ports and never fixed 5432/8000' {
        $plan = Resolve-PortPlan -Namespace 'claude-h1' -Services @('postgres', 'runtime', 'exporter', 'sim') -InUse @()
        $plan.Keys.Count | Should Be 4
        (($plan.Values | Sort-Object -Unique).Count) | Should Be 4
        @($plan.Values | Where-Object { $_ -in 5432, 8000 }).Count | Should Be 0
    }
    It 'probes upward when the derived base is busy (no explicit base)' {
        $base = Get-PortBase -Namespace 'claude-h1'
        $plan = Resolve-PortPlan -Namespace 'claude-h1' -Services @('postgres') -InUse @($base)
        ($plan['postgres'] -ne $base) | Should Be $true
    }
    It 'refuses an explicit PortBase that is occupied with pulso:port_conflict' {
        { Resolve-PortPlan -Namespace 'claude-h1' -Services @('postgres', 'runtime') -PortBase 18500 -InUse @(18501) } |
            Should Throw 'pulso:port_conflict'
    }
    It 'accepts an explicit PortBase that is free' {
        $plan = Resolve-PortPlan -Namespace 'claude-h1' -Services @('postgres') -PortBase 18500 -InUse @(18900)
        $plan['postgres'] | Should Be 18500
    }
}

Describe 'evidence' {
    $pin = '86a767474042a566a0dbd6ed23588959f27ebdb3'
    It 'stamps target, runtime_profile, machine and doubles' {
        $s = New-EvidenceStamp -Target 'mock' -RuntimeProfile 'contract_mock' -Namespace 'claude-1' -Machine 'pulso-dev' `
            -Connection 'pulso-dev' -Doubles @('registry-mock')
        $s.target | Should Be 'mock'
        $s.machine.connection | Should Be 'pulso-dev'
        @($s.doubles).Count | Should Be 1
    }
    It 'refuses real_local when /_sim/info is present' {
        Test-RealLocalEvidence -SimInfoPresent $true -AgentCoreSha $pin -ImageDigest 'sha256:aa' -ExpectedImageDigest 'sha256:aa' -Ready $true | Should Be 'evidence_target_unproven'
    }
    It 'refuses real_local on digest mismatch, wrong sha or not ready' {
        Test-RealLocalEvidence -SimInfoPresent $false -AgentCoreSha $pin -ImageDigest 'sha256:aa' -ExpectedImageDigest 'sha256:bb' -Ready $true | Should Be 'evidence_target_unproven'
        Test-RealLocalEvidence -SimInfoPresent $false -AgentCoreSha 'deadbeef' -ImageDigest 'sha256:aa' -ExpectedImageDigest 'sha256:aa' -Ready $true | Should Be 'evidence_target_unproven'
        Test-RealLocalEvidence -SimInfoPresent $false -AgentCoreSha $pin -ImageDigest 'sha256:aa' -ExpectedImageDigest 'sha256:aa' -Ready $false | Should Be 'evidence_target_unproven'
    }
    It 'proves real_local when everything matches' {
        Test-RealLocalEvidence -SimInfoPresent $false -AgentCoreSha $pin -ImageDigest 'sha256:aa' -ExpectedImageDigest 'sha256:aa' -Ready $true | Should Be 'real_local'
    }
    It 'unions the runtime doubles with labelled double containers' {
        @(Merge-Doubles -Runtime @('x: stand-in') -Containers @('platform-sim')).Count | Should Be 2
    }
}

# Evidence stamping (plan 17.3.8): target is derived from observation, never typed.
$script:PinSha = '86a767474042a566a0dbd6ed23588959f27ebdb3'

function New-EvidenceStamp {
    param([string]$Target, [string]$RuntimeProfile, [string]$Namespace, [string]$Machine, [string]$Connection, [string[]]$Doubles = @())
    [ordered]@{
        schema_version = 1; target = $Target; runtime_profile = $RuntimeProfile; namespace = $Namespace
        machine = [ordered]@{ name = $Machine; connection = $Connection }
        doubles = @($Doubles); stamped_at = [DateTime]::UtcNow.ToString('o')
    }
}

function Test-RealLocalEvidence {
    param([bool]$SimInfoPresent, [string]$AgentCoreSha, [string]$ImageDigest, [string]$ExpectedImageDigest, [bool]$Ready)
    if ($SimInfoPresent -or $AgentCoreSha -ne $script:PinSha -or -not $Ready -or
        -not $ImageDigest -or $ImageDigest -eq 'unknown' -or $ImageDigest -ne $ExpectedImageDigest) { return 'evidence_target_unproven' }
    'real_local'
}

# doubles[] = runtime's own report UNION running containers labelled com.pulso.role=double.
function Merge-Doubles {
    param([string[]]$Runtime = @(), [string[]]$Containers = @())
    @(@($Runtime) + @($Containers | ForEach-Object { "container:$_" }) | Where-Object { $_ } | Select-Object -Unique)
}

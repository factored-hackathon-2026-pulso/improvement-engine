# Typed failures: exceptions whose message starts with `pulso:<code>` map to stable exit codes.
$script:ExitCodes = @{
    machine_not_registered = 2; namespace_not_owned = 2; confirm_required = 2; insufficient_memory = 3
    port_conflict = 4; core_not_ready = 5; core_migrate_failed = 6; core_postgres_unavailable = 7
    runtime_cgroup_unavailable = 8; core_seed_failed = 9; core_demo_doubles_active = 10
}

function Get-PulsoCode {
    param([string]$Message)
    if ($Message -match 'pulso:([a-z_]+)') { return $Matches[1] }
    $null
}

function Exit-Pulso {
    param([Parameter(Mandatory)][System.Management.Automation.ErrorRecord]$ErrorRecord)
    $msg = $ErrorRecord.Exception.Message
    [Console]::Error.WriteLine($msg)
    Write-Output $msg
    $code = Get-PulsoCode -Message $msg
    if ($code -and $script:ExitCodes.ContainsKey($code)) { exit $script:ExitCodes[$code] }
    exit 1
}

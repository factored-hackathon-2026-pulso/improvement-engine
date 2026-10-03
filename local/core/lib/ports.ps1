# Host port derivation and probing (plan 17.3.8): PORT_BASE = crc32(ns) % 200 * 10 + 18000, then probed.
function Get-Crc32 {
    param([Parameter(Mandatory)][string]$Text)
    [uint64]$mask = 4294967295
    $table = New-Object 'uint64[]' 256
    for ($i = 0; $i -lt 256; $i++) {
        [uint64]$c = $i
        for ($k = 0; $k -lt 8; $k++) { if ($c -band 1) { $c = [uint64](3988292384 -bxor ($c -shr 1)) } else { $c = $c -shr 1 } }
        $table[$i] = $c
    }
    [uint64]$crc = $mask
    foreach ($b in [Text.Encoding]::UTF8.GetBytes($Text)) { $crc = [uint64]($table[[int](($crc -bxor $b) -band 255)] -bxor ($crc -shr 8)) }
    [uint64]($crc -bxor $mask)
}
function Get-PortBase {
    param([Parameter(Mandatory)][string]$Namespace)
    [int]((([uint64](Get-Crc32 -Text $Namespace)) % 200) * 10 + 18000)
}

# Ports currently bound on the host plus ports published by any project on the engine (pass -Connection to include it).
function Get-PortsInUse {
    param([string]$Connection)
    $used = New-Object System.Collections.Generic.HashSet[int]
    try { foreach ($c in (Get-NetTCPConnection -State Listen -ErrorAction Stop)) { [void]$used.Add([int]$c.LocalPort) } } catch {}
    if ($Connection) {
        try {
            foreach ($line in (Invoke-Podman -Connection $Connection ps --format '{{.Ports}}' 2>$null)) {
                foreach ($m in [regex]::Matches([string]$line, ':(\d+)->')) { [void]$used.Add([int]$m.Groups[1].Value) }
            }
        } catch {}
    }
    @($used)
}

function Resolve-PortPlan {
    [CmdletBinding()]
    param([Parameter(Mandatory)][string]$Namespace, [Parameter(Mandatory)][string[]]$Services,
          [int]$PortBase = 0, [int[]]$InUse = @())
    $explicit = $PortBase -gt 0
    $base = if ($explicit) { $PortBase } else { Get-PortBase -Namespace $Namespace }
    for ($attempt = 0; $attempt -lt 20; $attempt++) {
        $plan = [ordered]@{}
        $i = 0
        foreach ($s in $Services) { $plan[$s] = $base + $i; $i++ }
        $busy = @($plan.Values | Where-Object { $_ -in $InUse })
        if ($busy.Count -eq 0) { return $plan }
        if ($explicit) { throw "pulso:port_conflict: port(s) $($busy -join ',') already bound; nothing was touched" }
        $base += 10
    }
    throw 'pulso:port_conflict: no free port window found after 20 probes'
}

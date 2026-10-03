# Namespace, project and label rules (plan 17.9.1): ns = claude-<id>, project pulso-<ns>.
function New-PulsoNamespace {
    param([long]$Milliseconds = ([DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()))
    "claude-$Milliseconds"
}

function Get-PulsoProject { param([Parameter(Mandatory)][string]$Namespace) "pulso-$Namespace" }

function Assert-ClaudeNamespace {
    param([string]$Namespace)
    if ($Namespace -notmatch '^claude-[a-z0-9][a-z0-9-]{0,40}$') {
        throw "pulso:namespace_not_owned: '$Namespace' is not a claude-* namespace"
    }
}

function Assert-NamespaceOwnership {
    param([Parameter(Mandatory)][string]$Namespace, [object[]]$Existing = @())
    Assert-ClaudeNamespace -Namespace $Namespace
    $project = Get-PulsoProject -Namespace $Namespace
    foreach ($e in @($Existing)) {
        if ($e.project -eq $project -and $e.team -ne 'claude') {
            throw "pulso:namespace_not_owned: project $project is owned by team '$($e.team)'"
        }
    }
}

function Get-PulsoLabels {
    param([Parameter(Mandatory)][string]$Namespace)
    @{ 'com.pulso.team' = 'claude'; 'com.pulso.namespace' = $Namespace }
}

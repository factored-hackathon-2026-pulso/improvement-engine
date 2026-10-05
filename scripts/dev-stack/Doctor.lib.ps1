# Pure helpers of scripts/dev-stack/doctor.ps1 (dot-sourced; tested by Doctor.Tests.ps1). Windows PowerShell 5.1 AND pwsh.
# Nothing here reads or prints a secret value: env files are only counted.

function New-DoctorRow {
    param([string]$Check, [string]$Status, [string]$Detail = '', [string]$Hint = '')
    if (@('OK', 'WARN', 'BLOCK') -notcontains $Status) { throw "unknown status '$Status' (OK|WARN|BLOCK)" }
    [pscustomobject]@{ Check = $Check; Status = $Status; Detail = $Detail; Hint = $Hint }
}

function Get-DoctorExitCode {
    param([object[]]$Rows)
    if (@($Rows | Where-Object { $_.Status -eq 'BLOCK' }).Count -gt 0) { return 1 }
    0
}

# Table lines; the fix hint is shown only for WARN and BLOCK rows.
function Format-DoctorTable {
    param([object[]]$Rows)
    $w = [Math]::Max(10, (($Rows | ForEach-Object { $_.Check.Length } | Measure-Object -Maximum).Maximum))
    foreach ($r in $Rows) {
        $line = ('{0,-5} {1} {2}' -f $r.Status, $r.Check.PadRight($w), $r.Detail)
        if ($r.Status -ne 'OK' -and $r.Hint) { $line += "`n      -> " + $r.Hint }
        $line
    }
}

function ConvertFrom-PythonVersion {
    param([string]$Text)
    if ($Text -match 'Python (\d+)\.(\d+)\.(\d+)') { return New-Object Version ([int]$Matches[1]), ([int]$Matches[2]), ([int]$Matches[3]) }
    $null
}

function Test-PythonVersionOk {
    param($Version)
    if (-not $Version) { return $false }
    ($Version.Major -gt 3) -or ($Version.Major -eq 3 -and $Version.Minor -ge 11)
}

# Presence and number of KEY=value lines only. Never returns a name or a value.
function Get-EnvFileStatus {
    param([string]$Path)
    if (-not $Path -or -not (Test-Path -LiteralPath $Path)) { return [pscustomobject]@{ Present = $false; Keys = 0 } }
    $n = 0
    foreach ($line in [IO.File]::ReadAllLines($Path)) {
        $t = $line.Trim()
        if ($t -and -not $t.StartsWith('#') -and $t.IndexOf('=') -gt 0) { $n++ }
    }
    [pscustomobject]@{ Present = $true; Keys = $n }
}

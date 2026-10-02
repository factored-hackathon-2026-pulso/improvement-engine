[CmdletBinding()]
param([Parameter()][string]$Path)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if ([string]::IsNullOrWhiteSpace($Path)) { $Path = Join-Path $PSScriptRoot "..\local\.env" }
if (Test-Path -LiteralPath $Path) { throw "Refusing to replace existing local environment file: $Path" }

$parent = Split-Path -Parent $Path
if (-not (Test-Path -LiteralPath $parent)) { New-Item -ItemType Directory -Path $parent -Force | Out-Null }

$bytes = New-Object byte[] 32
$generator = [System.Security.Cryptography.RandomNumberGenerator]::Create()
try { $generator.GetBytes($bytes) } finally { $generator.Dispose() }
$password = [Convert]::ToBase64String($bytes).Replace("+", "-").Replace("/", "_").TrimEnd("=")
$content = @("PULSO_POSTGRES_DB=pulso", "PULSO_POSTGRES_USER=pulso", "PULSO_POSTGRES_PASSWORD=$password", "PULSO_POSTGRES_PORT=54329", "PULSO_LOCALSTACK_PORT=4566") -join "`n"
[System.IO.File]::WriteAllText($Path, "$content`n", (New-Object System.Text.UTF8Encoding($false)))
Write-Output "Created local environment file: $Path"

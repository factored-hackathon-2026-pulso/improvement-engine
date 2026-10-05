# Pure helpers of scripts/demo-magic.ps1 (dot-sourced; unit-tested by demo-magic.Tests.ps1).

# 'present' (a build newer than every source file exists), 'stale-present' (a build exists but sources are newer and node is
# missing, so it is served as is), 'build' (needs npm; node exists) or 'api-only' (no build and no node: never pretend).
function Get-ConsoleBuildPlan {
    param([Parameter(Mandatory)][string]$ConsoleDir, [Parameter(Mandatory)][bool]$NodeAvailable)
    $index = Join-Path $ConsoleDir 'dist\index.html'
    $hasDist = Test-Path -LiteralPath $index
    if (-not $hasDist) { if ($NodeAvailable) { return 'build' } else { return 'api-only' } }
    $builtAt = (Get-Item -LiteralPath $index).LastWriteTime
    $inputs = @(Get-ChildItem -LiteralPath (Join-Path $ConsoleDir 'src') -Recurse -File -ErrorAction SilentlyContinue) +
        @('package.json', 'package-lock.json', 'index.html', 'vite.config.ts', 'tsconfig.json', 'public\config.json' | ForEach-Object { Get-Item -LiteralPath (Join-Path $ConsoleDir $_) -ErrorAction SilentlyContinue })
    $newest = $inputs | Where-Object { $_ } | Sort-Object LastWriteTime -Descending | Select-Object -First 1
    if (-not $newest -or $newest.LastWriteTime -le $builtAt) { return 'present' }
    if ($NodeAvailable) { return 'build' } else { return 'stale-present' }
}

# An ephemeral admin token for this local run only (never written to disk, never printed).
function New-DemoToken {
    $b = New-Object byte[] 16
    $rng = [Security.Cryptography.RandomNumberGenerator]::Create()
    try { $rng.GetBytes($b) } finally { $rng.Dispose() }
    -join ($b | ForEach-Object { $_.ToString('x2') })
}

# 'pulso listening on http://127.0.0.1:4020' -> 'http://127.0.0.1:4020'; anything else -> $null.
function Get-ListeningUrl {
    param([string]$Line)
    if ($Line -match '^pulso listening on (http://\S+)\s*$') { return $Matches[1] }
    return $null
}

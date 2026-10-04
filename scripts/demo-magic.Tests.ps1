# Pester 3.x (host version). Offline: the pure decisions of scripts/demo-magic.ps1.
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
. (Join-Path $here 'demo-magic.lib.ps1')

function New-FakeConsole([switch]$Dist, [datetime]$DistAt = (Get-Date), [datetime]$SrcAt = (Get-Date).AddHours(-1)) {
    $d = Join-Path ([IO.Path]::GetTempPath()) ("dm-" + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path (Join-Path $d 'src') | Out-Null
    Set-Content -LiteralPath (Join-Path $d 'src\main.tsx') -Value 'x'
    (Get-Item (Join-Path $d 'src\main.tsx')).LastWriteTime = $SrcAt
    if ($Dist) {
        New-Item -ItemType Directory -Path (Join-Path $d 'dist') | Out-Null
        Set-Content -LiteralPath (Join-Path $d 'dist\index.html') -Value '<html/>'
        (Get-Item (Join-Path $d 'dist\index.html')).LastWriteTime = $DistAt
    }
    $d
}

Describe 'Get-ConsoleBuildPlan' {
    It 'is present when a build newer than every source file exists' {
        Get-ConsoleBuildPlan -ConsoleDir (New-FakeConsole -Dist) -NodeAvailable $true | Should Be 'present'
        Get-ConsoleBuildPlan -ConsoleDir (New-FakeConsole -Dist) -NodeAvailable $false | Should Be 'present'
    }
    It 'rebuilds a stale build when node exists' {
        $d = New-FakeConsole -Dist -DistAt (Get-Date).AddHours(-2)
        Get-ConsoleBuildPlan -ConsoleDir $d -NodeAvailable $true | Should Be 'build'
    }
    It 'builds when there is no build and node exists' {
        Get-ConsoleBuildPlan -ConsoleDir (New-FakeConsole) -NodeAvailable $true | Should Be 'build'
    }
    It 'serves the API only when there is no build and no node (never pretends the console exists)' {
        Get-ConsoleBuildPlan -ConsoleDir (New-FakeConsole) -NodeAvailable $false | Should Be 'api-only'
    }
    It 'a stale build without node is still served (present), flagged by the caller as stale' {
        $d = New-FakeConsole -Dist -DistAt (Get-Date).AddHours(-2)
        Get-ConsoleBuildPlan -ConsoleDir $d -NodeAvailable $false | Should Be 'stale-present'
    }
}

Describe 'New-DemoToken' {
    It 'is 32 hex chars and differs per call' {
        $a = New-DemoToken; $b = New-DemoToken
        $a | Should Match '^[0-9a-f]{32}$'
        $a | Should Not Be $b
    }
}

Describe 'Get-ListeningUrl' {
    It 'reads the base URL from the listening line' {
        Get-ListeningUrl -Line 'pulso listening on http://127.0.0.1:4020' | Should Be 'http://127.0.0.1:4020'
    }
    It 'returns nothing for any other line' {
        Get-ListeningUrl -Line 'pulso: serving the API only' | Should BeNullOrEmpty
    }
}

# Pester 3.x. Offline: dev.ps1 is exercised with -DryRun and with an observed-states file (no container engine).
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$dev = (Resolve-Path (Join-Path $here '..\..\dev.ps1')).Path
$ps = (Get-Process -Id $PID).Path
$tmp = Join-Path $env:TEMP 'pulso-devscript-test'
New-Item -ItemType Directory -Force -Path $tmp | Out-Null
function Invoke-Dev([string[]]$a) { $o = & $ps -NoProfile -File $dev @a 2>&1 | Out-String; [pscustomobject]@{ code = $LASTEXITCODE; out = $o } }
function Write-States($obj) { $f = Join-Path $tmp ([guid]::NewGuid().ToString('N') + '.json'); $obj | ConvertTo-Json -Depth 4 | Set-Content $f; $f }
$ok = @{ 'core-postgres' = @{ status = 'running'; exit = 0; health = 'healthy' }; 'core-migrate' = @{ status = 'exited'; exit = 0; health = '' } }

Describe 'dev.ps1' {
    It 'up -DryRun plans the stack on pulso-dev without touching the engine' {
        $r = Invoke-Dev @('up', '-DryRun')
        $r.code | Should Be 0
        $r.out | Should Match 'pulso-claude-dev'
        $r.out | Should Match '--connection pulso-dev'
    }
    It 'refuses a machine that is not registered (pulso-codex)' {
        $r = Invoke-Dev @('up', '-DryRun', '-Machine', 'pulso-codex')
        $r.code | Should Not Be 0
        $r.out | Should Match 'machine_not_registered'
    }
    It 'doctor is green for healthy observed states' {
        $r = Invoke-Dev @('doctor', '-StatesFile', (Write-States $ok), '-ExpectedServices', 'core-postgres,core-migrate:oneshot')
        $r.code | Should Be 0
        $r.out | Should Match 'doctor: green'
    }
    It 'doctor fails naming the stopped service' {
        $bad = @{ 'core-postgres' = @{ status = 'exited'; exit = 137; health = '' }; 'core-migrate' = @{ status = 'exited'; exit = 0; health = '' } }
        $r = Invoke-Dev @('doctor', '-StatesFile', (Write-States $bad), '-ExpectedServices', 'core-postgres,core-migrate:oneshot')
        $r.code | Should Be 1
        $r.out | Should Match 'core-postgres'
        $r.out | Should Match 'service_stopped'
    }
}

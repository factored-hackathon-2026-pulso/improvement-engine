# Pester 3.x. Offline: the pure parts of doctor.ps1 (row model, exit code, version parsing, env-file presence without values).
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
. (Join-Path $here 'Doctor.lib.ps1')

Describe 'New-DoctorRow / Get-DoctorExitCode' {
    It 'exit 0 when only OK and WARN rows, 1 when any BLOCK' {
        $ok = @((New-DoctorRow 'a' 'OK' 'fine'), (New-DoctorRow 'b' 'WARN' 'meh' 'hint'))
        Get-DoctorExitCode -Rows $ok | Should Be 0
        $bad = $ok + @(New-DoctorRow 'c' 'BLOCK' 'missing' 'install it')
        Get-DoctorExitCode -Rows $bad | Should Be 1
    }
    It 'refuses an unknown status' {
        { New-DoctorRow 'x' 'MAYBE' 'd' } | Should Throw 'unknown status'
    }
    It 'formats a table that carries the hint of blockers and warnings only' {
        $rows = @((New-DoctorRow 'uv' 'OK' '0.5' 'never shown'), (New-DoctorRow 'cargo' 'BLOCK' 'not found' 'install rustup'))
        $t = (Format-DoctorTable -Rows $rows) -join "`n"
        $t | Should Match 'install rustup'
        $t | Should Not Match 'never shown'
    }
}

Describe 'ConvertFrom-PythonVersion' {
    It 'parses python --version output' {
        (ConvertFrom-PythonVersion 'Python 3.11.9').ToString() | Should Be '3.11.9'
        ConvertFrom-PythonVersion 'nonsense' | Should BeNullOrEmpty
    }
    It 'accepts 3.11 and newer only' {
        Test-PythonVersionOk -Version (ConvertFrom-PythonVersion 'Python 3.11.0') | Should Be $true
        Test-PythonVersionOk -Version (ConvertFrom-PythonVersion 'Python 3.13.1') | Should Be $true
        Test-PythonVersionOk -Version (ConvertFrom-PythonVersion 'Python 3.10.12') | Should Be $false
    }
}

Describe 'Get-EnvFileStatus' {
    It 'reports presence and the NUMBER of keys, never a name or value' {
        $f = Join-Path ([IO.Path]::GetTempPath()) ('doc-' + [guid]::NewGuid().ToString('N') + '.env')
        [IO.File]::WriteAllText($f, "SECRET_ONE=hunter2hunter2`n# c`nSECRET_TWO=abcdefgh`n")
        try {
            $s = Get-EnvFileStatus -Path $f
            $s.Present | Should Be $true
            $s.Keys | Should Be 2
            ($s | Out-String) | Should Not Match 'hunter2'
            ($s | Out-String) | Should Not Match 'SECRET_ONE'
        } finally { Remove-Item -LiteralPath $f -Force }
        (Get-EnvFileStatus -Path '/nope/x.env').Present | Should Be $false
    }
}

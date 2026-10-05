# Pester 3.x (also runs under pwsh). Offline: resolution order of the dev config, OS helpers with mocked OS, RAM parsers. No stack, no network.
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
. (Join-Path $here 'DevConfig.ps1')

function New-TempRoot {
    $d = Join-Path ([IO.Path]::GetTempPath()) ('devcfg-' + [guid]::NewGuid().ToString('N'))
    $null = New-Item -ItemType Directory -Force -Path (Join-Path $d 'repo')
    (Join-Path $d 'repo')
}

Describe 'Read-DevConfigFile' {
    It 'reads allowlisted names, strips quotes and comments, keeps an empty value, ignores unknown names' {
        $r = New-TempRoot; $f = Join-Path $r 'devconfig.env'
        [IO.File]::WriteAllText($f, "# c`nPULSO_AGENT_CORE_DIR=`"/x/ac`"`nPULSO_PODMAN_MACHINE=`nSOME_SECRET_KEY=abc`nPULSO_RIG_PG_PORT = 55999`n")
        $v = Read-DevConfigFile -Path $f
        $v['PULSO_AGENT_CORE_DIR'] | Should Be '/x/ac'
        $v.ContainsKey('PULSO_PODMAN_MACHINE') | Should Be $true
        $v['PULSO_PODMAN_MACHINE'] | Should Be ''
        $v['PULSO_RIG_PG_PORT'] | Should Be '55999'
        $v.ContainsKey('SOME_SECRET_KEY') | Should Be $false
    }
    It 'returns an empty table for a missing file' {
        (Read-DevConfigFile -Path '/nope/devconfig.env').Count | Should Be 0
    }
}

Describe 'Get-DevConfig resolution order' {
    It 'file beats env beats relative default beats legacy' {
        $r = New-TempRoot
        $parent = Split-Path -Parent $r
        $null = New-Item -ItemType Directory -Force -Path (Join-Path $parent 'agent-core'), (Join-Path $parent 'llm-gateway')
        $leg = Join-Path $parent 'legacy-ac'; $null = New-Item -ItemType Directory -Force -Path $leg
        $legacy = @{ PULSO_AGENT_CORE_DIR = $leg; PULSO_LLM_GATEWAY_DIR = (Join-Path $parent 'legacy-gw') }
        $c = Get-DevConfig -Root $r -Legacy $legacy -Environment @{} -OS 'Linux'
        $c.Values['PULSO_AGENT_CORE_DIR'] | Should Be (Join-Path $parent 'agent-core')
        $c.Sources['PULSO_AGENT_CORE_DIR'] | Should Be 'default'
        $c = Get-DevConfig -Root $r -Legacy $legacy -Environment @{ PULSO_AGENT_CORE_DIR = '/from/env' } -OS 'Linux'
        $c.Values['PULSO_AGENT_CORE_DIR'] | Should Be '/from/env'
        $c.Sources['PULSO_AGENT_CORE_DIR'] | Should Be 'env'
        [IO.File]::WriteAllText((Join-Path $r 'devconfig.env'), "PULSO_AGENT_CORE_DIR=/from/file`n")
        $c = Get-DevConfig -Root $r -Legacy $legacy -Environment @{ PULSO_AGENT_CORE_DIR = '/from/env' } -OS 'Linux'
        $c.Values['PULSO_AGENT_CORE_DIR'] | Should Be '/from/file'
        $c.Sources['PULSO_AGENT_CORE_DIR'] | Should Be 'file'
    }
    It 'falls back to the legacy value when the relative default is absent and the legacy exists' {
        $r = New-TempRoot; $leg = Join-Path (Split-Path -Parent $r) 'old-gw'; $null = New-Item -ItemType Directory -Force -Path $leg
        $c = Get-DevConfig -Root $r -Legacy @{ PULSO_LLM_GATEWAY_DIR = $leg } -Environment @{} -OS 'Linux'
        $c.Values['PULSO_LLM_GATEWAY_DIR'] | Should Be $leg
        $c.Sources['PULSO_LLM_GATEWAY_DIR'] | Should Be 'legacy'
    }
    It 'keeps the relative default (for a precise error) when neither exists' {
        $r = New-TempRoot
        $c = Get-DevConfig -Root $r -Legacy @{ PULSO_PLATFORM_DIR = '/does/not/exist' } -Environment @{} -OS 'Linux'
        $c.Values['PULSO_PLATFORM_DIR'] | Should Be (Join-Path (Split-Path -Parent $r) 'support-platform')
        $c.Sources['PULSO_PLATFORM_DIR'] | Should Be 'default'
    }
    It 'honours PULSO_DEVCONFIG as the file path' {
        $r = New-TempRoot; $f = Join-Path $r 'elsewhere.env'
        [IO.File]::WriteAllText($f, "PULSO_PLATFORM_DIR=/p`n")
        $c = Get-DevConfig -Root $r -Environment @{ PULSO_DEVCONFIG = $f } -OS 'Linux'
        $c.Values['PULSO_PLATFORM_DIR'] | Should Be '/p'
    }
    It 'derives target dir and exe names per OS' {
        $r = New-TempRoot
        $l = Get-DevConfig -Root $r -Environment @{} -OS 'Linux'
        $l.Values['CARGO_TARGET_DIR'] | Should Be (Join-Path $r 'target')
        $l.Values['PULSO_EXE'] | Should Be (Join-Path (Join-Path (Join-Path $r 'target') 'debug') 'pulso')
        $w = Get-DevConfig -Root $r -Environment @{ CARGO_TARGET_DIR = 'C:/t' } -OS 'Windows'
        $w.Values['PULSO_EXE'] | Should Be (Join-Path (Join-Path 'C:/t' 'debug') 'pulso.exe')
        $w.Values['PULSO_STEPS_EXE'] | Should Be (Join-Path (Join-Path 'C:/t' 'debug') 'steps_cli.exe')
    }
    It 'env files default to ../.pulso-env/*.env' {
        $r = New-TempRoot
        $c = Get-DevConfig -Root $r -Environment @{} -OS 'Linux'
        $c.Values['PULSO_AGENT_CORE_ENV'] | Should Be (Join-Path (Join-Path (Split-Path -Parent $r) '.pulso-env') 'agent-core.env')
        $c.Values['PULSO_LLM_GATEWAY_ENV'] | Should Be (Join-Path (Join-Path (Split-Path -Parent $r) '.pulso-env') 'llm-gateway.env')
    }
    It 'exports file values to the process env only with -Export' {
        $r = New-TempRoot; [IO.File]::WriteAllText((Join-Path $r 'devconfig.env'), "PULSO_RIG_SPA_PORT=5999`n")
        $keep = $env:PULSO_RIG_SPA_PORT
        try {
            $null = Get-DevConfig -Root $r -Environment @{} -OS 'Linux'
            $env:PULSO_RIG_SPA_PORT | Should Be $keep
            $null = Get-DevConfig -Root $r -Environment @{} -OS 'Linux' -Export
            $env:PULSO_RIG_SPA_PORT | Should Be '5999'
        } finally { $env:PULSO_RIG_SPA_PORT = $keep }
    }
}

Describe 'Empty values in devconfig.env' {
    It 'an empty path or port is ignored (a copied example never blanks a path); an empty PULSO_PODMAN_MACHINE is kept' {
        $r = New-TempRoot; [IO.File]::WriteAllText((Join-Path $r 'devconfig.env'), "PULSO_AGENT_CORE_DIR=`nPULSO_PODMAN_MACHINE=`n")
        $c = Get-DevConfig -Root $r -Environment @{ PULSO_AGENT_CORE_DIR = '/from/env' } -OS 'Linux'
        $c.Values['PULSO_AGENT_CORE_DIR'] | Should Be '/from/env'
        $c.Sources['PULSO_PODMAN_MACHINE'] | Should Be 'file'
    }
}

Describe 'Podman connection' {
    It 'explicit connection wins' {
        (Get-DevPodmanArgs -Environment @{ PULSO_PODMAN_CONNECTION = 'c1'; PULSO_PODMAN_MACHINE = 'm' } -OS 'Linux') -join ' ' | Should Be '--connection c1'
    }
    It 'a machine name maps to its root connection' {
        (Get-DevPodmanArgs -Environment @{ PULSO_PODMAN_MACHINE = 'dev2' } -OS 'macOS') -join ' ' | Should Be '--connection dev2-root'
    }
    It 'an explicitly empty machine means the default connection (no args)' {
        @(Get-DevPodmanArgs -Environment @{ PULSO_PODMAN_MACHINE = '' } -OS 'Windows').Count | Should Be 0
    }
    It 'unset: Windows keeps the old pulso-dev-root, Linux uses native podman' {
        (Get-DevPodmanArgs -Environment @{} -OS 'Windows') -join ' ' | Should Be '--connection pulso-dev-root'
        @(Get-DevPodmanArgs -Environment @{} -OS 'Linux').Count | Should Be 0
    }
}

Describe 'OS helpers' {
    It 'exe suffix only on Windows' {
        Get-DevExeSuffix -OS 'Windows' | Should Be '.exe'
        Get-DevExeSuffix -OS 'Linux' | Should Be ''
        Get-DevExeSuffix -OS 'macOS' | Should Be ''
    }
    It 'null device per OS' {
        Get-DevNullDevice -OS 'Windows' | Should Be 'nul'
        Get-DevNullDevice -OS 'Linux' | Should Be '/dev/null'
    }
    It 'Get-DevOS honours the override' {
        Get-DevOS -Override 'Linux' | Should Be 'Linux'
    }
    It 'parses /proc/meminfo (MemAvailable) to MB' {
        ConvertFrom-ProcMeminfo -Text "MemTotal: 16000000 kB`nMemFree: 100 kB`nMemAvailable: 2048000 kB`n" | Should Be 2000
        ConvertFrom-ProcMeminfo -Text "MemTotal: 1 kB" | Should Be -1
    }
    It 'parses vm_stat to MB (free + inactive + speculative)' {
        $t = "Mach Virtual Memory Statistics: (page size of 16384 bytes)`nPages free: 1000.`nPages active: 5.`nPages inactive: 2000.`nPages speculative: 640.`n"
        ConvertFrom-VmStat -Text $t | Should Be 57
        ConvertFrom-VmStat -Text 'garbage' | Should Be -1
    }
    It 'Get-DevFreeRamMb reads meminfo on Linux (mocked) and vm_stat on macOS (mocked)' {
        Mock Read-DevMeminfo { "MemAvailable: 1024000 kB" }
        Get-DevFreeRamMb -OS 'Linux' | Should Be 1000
        Mock Read-DevVmStat { "Mach Virtual Memory Statistics: (page size of 4096 bytes)`nPages free: 256000.`n" }
        Get-DevFreeRamMb -OS 'macOS' | Should Be 1000
    }
}

Describe 'Get-DevShellCommand' {
    It 'uses cmd /c on Windows and sh -c elsewhere, with the null device substituted' {
        $w = Get-DevShellCommand -Line '"e" run 2>{NULL}' -OS 'Windows' -ComSpec 'cmd.exe'
        $w.FileName | Should Be 'cmd.exe'
        $w.ArgumentString | Should Be '/c ""e" run 2>nul"'
        $l = Get-DevShellCommand -Line '"e" run 2>{NULL}' -OS 'Linux'
        $l.FileName | Should Be '/bin/sh'
        $l.ArgumentList[0] | Should Be '-c'
        $l.ArgumentList[1] | Should Be '"e" run 2>/dev/null'
    }
}

Describe 'Test-DevPortFree' {
    It 'is false for a port we hold and true after release' {
        $l = New-Object Net.Sockets.TcpListener([Net.IPAddress]::Loopback, 0); $l.Start()
        $p = $l.LocalEndpoint.Port
        try { Test-DevPortFree -Port $p | Should Be $false } finally { $l.Stop() }
        Test-DevPortFree -Port $p | Should Be $true
    }
}

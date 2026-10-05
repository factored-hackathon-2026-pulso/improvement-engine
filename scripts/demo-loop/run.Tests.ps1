# Pester 3.x (host version; also runs under pwsh). Offline: the pure parts of scripts/demo-loop/run.ps1
# (argument handling, output formatting, redaction). No network, no stack, no model.
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
. (Join-Path $here 'run.lib.ps1')

$loopJson = @'
{
  "models": "xiaomi/mimo-v2.6-flash",
  "baseline": {"label": "live-registry", "live": 29, "fixture": 0},
  "evaluate_before_announce": "on",
  "summary": {"corroborated": 3, "reasoned": 3, "proposed": 2, "delivered": 1, "announced": 1, "not_announced": 1, "unlinked": 1, "blocked": 0, "cost_usd": 0.0031},
  "findings": [
    {"finding_id": "finding_1", "metric": "M4", "status": "proposed", "reason": "compiled", "outcome": "announced", "evidence_ref": "ev_0123456789abcdef",
     "target_ref": "template:t/estado_pqr", "proposal_kind": "patch", "metering": {"cost_usd": 0.0012},
     "delivery": {"status": "delivered", "proposal_id": "prop-abc123"},
     "evaluation": {"verdict": "regression_suite_proven", "reason": "8/8 fail on base", "story_text": {"es": "La base falla 8 de 8 casos de la suite; intento 1 paso. [regression_suite_proven]"},
       "dossier": {"es": {"title": "template:t/estado_pqr - M4: propuesta de cambio",
         "description": "DECISION: proponer a supervision.\n\nProblema observado: M4 mas alto.",
         "sections": {"problem": "M4 es mas alto en category=Cobro indebido.", "evidence": "Celda: 62,0 % frente a 33,0 %.", "expected_effect": "Hipotesis direccional."}}}}},
    {"finding_id": "finding_2", "metric": "M1", "status": "proposed", "reason": "compiled", "outcome": "not_announced:not_fixed", "target_ref": "new_agent:soporte-tecnico",
     "proposal_kind": "new_agent", "metering": {"cost_usd": 0.0019}, "evaluation": {"verdict": "not_fixed", "reason": "candidate still fails"}},
    {"finding_id": "finding_3", "metric": "M6", "status": "unlinked", "reason": "dependency_metric", "metering": {"cost_usd": 0.0}}
  ]
}
'@

Describe 'Get-DemoLoopPlan' {
    It 'refuses an empty command line and names the switches' {
        { Get-DemoLoopPlan } | Should Throw 'nothing to do'
    }
    It 'runs the steps in the fixed order whatever the order of the switches' {
        $p = Get-DemoLoopPlan -Show -Loop -Up -Cells
        ($p.Steps -join ',') | Should Be 'Up,Cells,Loop,Show'
    }
    It '-All is Up Cells Loop Show and never Down' {
        ((Get-DemoLoopPlan -All).Steps -join ',') | Should Be 'Up,Cells,Loop,Show'
    }
    It 'puts Probes after Loop and Announce after Show' {
        ((Get-DemoLoopPlan -Announce -Probes -Show -Loop).Steps -join ',') | Should Be 'Loop,Probes,Show,Announce'
    }
    It 'refuses -Up with -Down and -Down with -Loop' {
        { Get-DemoLoopPlan -Up -Down } | Should Throw 'contradict'
        { Get-DemoLoopPlan -Down -Loop } | Should Throw 'cannot be combined'
    }
    It 'refuses -Synthetic without a cell step and a cells file together with -Synthetic' {
        { Get-DemoLoopPlan -Synthetic -Show } | Should Throw '-Synthetic only applies'
        { Get-DemoLoopPlan -Synthetic -Cells -CellsFile 'x.ndjson' } | Should Throw 'different cell sources'
    }
    It 'bounds MaxFindings, TimeoutMin and ProbeReps' {
        { Get-DemoLoopPlan -Loop -MaxFindings 0 } | Should Throw '-MaxFindings'
        { Get-DemoLoopPlan -Loop -MaxFindings 51 } | Should Throw '-MaxFindings'
        { Get-DemoLoopPlan -Loop -TimeoutMin 0 } | Should Throw '-TimeoutMin'
        { Get-DemoLoopPlan -Probes -ProbeReps 6 } | Should Throw '-ProbeReps'
    }
    It 'labels the mode' {
        (Get-DemoLoopPlan -Cells).Mode | Should Be 'bank'
        (Get-DemoLoopPlan -Cells -Synthetic).Mode | Should Be 'synthetic-planted'
        (Get-DemoLoopPlan -Cells -CellsFile 'a.ndjson').Mode | Should Be 'bank-file'
    }
}

Describe 'Get-DemoStackSettings' {
    It 'uses its own prefix and ports, never the shared stack' {
        $s = Get-DemoStackSettings
        $s.Prefix | Should Be 'pulso-demo'
        $s.PgPort | Should Not Be 55432
        $s.CorePort | Should Not Be 8001
        $s.GwPort | Should Not Be 8080
    }
    It 'refuses the shared prefix, duplicate ports and a bad prefix' {
        { Get-DemoStackSettings -Prefix 'pulso-l3' } | Should Throw 'shared dev stack'
        { Get-DemoStackSettings -PgPort 9000 -GwPort 9000 } | Should Throw 'different'
        { Get-DemoStackSettings -Prefix 'Bad Prefix' } | Should Throw 'lowercase'
    }
}

Describe 'Get-EngineEnvironment' {
    $s = Get-DemoStackSettings
    It 'bank mode opts in to derived aggregates, synthetic does not' {
        $b = Get-EngineEnvironment -Settings $s -CellsPath 'c.ndjson' -WorkDir 'w' -StoreDir 's' -Source bank
        $b['PULSO_ALLOW_DERIVED_AGGREGATES'] | Should Be '1'
        $b['PULSO_CELLS_SOURCE'] | Should Be 'bank'
        $y = Get-EngineEnvironment -Settings $s -CellsPath 'c.ndjson' -WorkDir 'w' -StoreDir 's' -Source synthetic
        $y.Contains('PULSO_ALLOW_DERIVED_AGGREGATES') | Should Be $false
    }
    It 'binds the engine to loopback and evaluates before announcing' {
        $e = Get-EngineEnvironment -Settings $s -CellsPath 'c' -WorkDir 'w' -StoreDir 's' -Source bank
        $e['PULSO_LISTEN_ADDR'] | Should Be '127.0.0.1:4190'
        $e['PULSO_EVAL_BEFORE_ANNOUNCE'] | Should Be 'on'
        $e['PULSO_REGISTRY_VIA'] | Should Be 'api'
    }
    It 'sets the Builder tier only when asked' {
        (Get-EngineEnvironment -Settings $s -CellsPath 'c' -WorkDir 'w' -StoreDir 's' -Source bank).Contains('PULSO_LLM_GATEWAY_BUILDER_MODEL') | Should Be $false
        (Get-EngineEnvironment -Settings $s -CellsPath 'c' -WorkDir 'w' -StoreDir 's' -Source bank -BuilderModel 'xiaomi/mimo-v2.6-pro')['PULSO_LLM_GATEWAY_BUILDER_MODEL'] | Should Be 'xiaomi/mimo-v2.6-pro'
    }
    It 'announces to the platform only when both URL and token are given' {
        (Get-EngineEnvironment -Settings $s -CellsPath 'c' -WorkDir 'w' -StoreDir 's' -Source bank -PlatformUrl 'http://127.0.0.1:9' )['PULSO_ANNOUNCE_TO_PLATFORM'] | Should Be 'off'
        $on = Get-EngineEnvironment -Settings $s -CellsPath 'c' -WorkDir 'w' -StoreDir 's' -Source bank -PlatformUrl 'http://127.0.0.1:9' -PlatformToken 'tok-tok-tok'
        $on['PULSO_ANNOUNCE_TO_PLATFORM'] | Should Be 'on'
    }
}

Describe 'Read-EnvFileValues and redaction' {
    $canary = 'CANARY-' + [guid]::NewGuid().ToString('N')
    $dir = Join-Path ([IO.Path]::GetTempPath()) ('dl-' + [guid]::NewGuid().ToString('N'))
    $null = New-Item -ItemType Directory -Path $dir
    $file = Join-Path $dir 'x.env'
    $dsnPass = 'p4ssw0rd-' + [guid]::NewGuid().ToString('N').Substring(0, 12)
    Set-Content -LiteralPath $file -Value @("# comment", "PULSO_CANARY=$canary", "QUOTED='quoted-value-123456'", "BLANK=", "DSN=postgresql://user:$dsnPass@host:5432/db", "SHORT=1") -Encoding ASCII

    It 'parses KEY=VALUE, strips quotes, drops blanks and comments' {
        $h = Read-EnvFileValues -Path $file
        $h['PULSO_CANARY'] | Should Be $canary
        $h['QUOTED'] | Should Be 'quoted-value-123456'
        $h.ContainsKey('BLANK') | Should Be $false
        $h.Count | Should Be 4
    }
    It 'returns an empty table for a missing file' {
        (Read-EnvFileValues -Path (Join-Path $dir 'nope.env')).Count | Should Be 0
    }
    It 'masks the value, and the password fragment of a DSN, wherever they appear' {
        $needles = Get-RedactionNeedles -Sources @(Read-EnvFileValues -Path $file)
        (Protect-Text -Text "token=$canary end" -Needles $needles) | Should Be 'token=*** end'
        (Protect-Text -Text "pw is $dsnPass ok" -Needles $needles) | Should Be 'pw is *** ok'
    }
    It 'never masks a short value (it would mangle ordinary words)' {
        $needles = Get-RedactionNeedles -Sources @(Read-EnvFileValues -Path $file)
        (Protect-Text -Text 'value 1 stays' -Needles $needles) | Should Be 'value 1 stays'
    }
    It 'leaves null and empty text alone' {
        Protect-Text -Text '' -Needles @('abcdefg') | Should Be ''
    }
    It 'CANARY: a child that dumps its whole environment never prints an env value, and the parent never receives it' {
        $h = Read-EnvFileValues -Path $file
        $needles = Get-RedactionNeedles -Sources @($h)
        $r = Invoke-Scrubbed -File $env:ComSpec -Arguments @('/c', 'set') -Env $h -Needles $needles -Quiet
        $r.ExitCode | Should Be 0
        $text = $r.Output -join "`n"
        $text.Contains($canary) | Should Be $false
        $text.Contains($dsnPass) | Should Be $false
        $text.Contains('quoted-value-123456') | Should Be $false
        ($text -match 'PULSO_CANARY=\*\*\*') | Should Be $true
        [Environment]::GetEnvironmentVariable('PULSO_CANARY') | Should BeNullOrEmpty
    }
    It 'CANARY: a failing child has its stderr masked too' {
        $h = Read-EnvFileValues -Path $file
        $needles = Get-RedactionNeedles -Sources @($h)
        $r = Invoke-Scrubbed -File $env:ComSpec -Arguments @('/c', "echo boom $canary 1>&2 & exit 3") -Env $h -Needles $needles -Quiet
        $r.ExitCode | Should Be 3
        (($r.Output -join "`n").Contains($canary)) | Should Be $false
    }
    Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
}

Describe 'Formatting the loop report' {
    $loop = $loopJson | ConvertFrom-Json
    $sig = '{"dims":{"category":"Cobro indebido"},"discovery":{"rate":0.62,"baseline_rate":0.33,"diff":0.29},"holdout":{"rate":0.6,"baseline_rate":0.34}}' | ConvertFrom-Json

    It 'formats a cell and an effect' {
        Format-Cell $sig.dims | Should Be 'category=Cobro indebido'
        Format-Effect $sig | Should Be '62.0% vs 33.0% rest (+29.0 pp); holdout 60.0% vs 34.0%'
        Format-Effect $null | Should Be 'n/a'
    }
    It 'names the outcome of every kind of record' {
        Get-FindingOutcome $loop.findings[0] | Should Be 'announced'
        Get-FindingOutcome $loop.findings[1] | Should Be 'not_announced:not_fixed'
        Get-FindingOutcome $loop.findings[2] | Should Be 'unlinked (dependency_metric)'
        Get-FindingOutcome ('{"status":"blocked","reason":"model_invalid"}' | ConvertFrom-Json) | Should Be 'blocked (model_invalid)'
    }
    It 'reads the slug from the target and the proof story in one line' {
        Get-ProposalSlug $loop.findings[0] | Should Be 'estado_pqr'
        Get-ProposalSlug $loop.findings[1] | Should Be 'soporte-tecnico'
        Get-ProposalSlug $loop.findings[2] | Should Be '-'
        (Get-ProofLine $loop.findings[0]) | Should Be 'regression_suite_proven: La base falla 8 de 8 casos de la suite; intento 1 paso. [regression_suite_proven]'
        (Get-ProofLine $loop.findings[1]) | Should Be 'not_fixed (candidate still fails)'
        (Get-ProofLine $loop.findings[2]) | Should Be '-'
    }
    It 'shows the slug of a NEW agent apart from its donor' {
        $na = '{"status":"proposed","target_ref":"new_agent:consultas","proposal_kind":"new_agent","agent_id":"soporte-tecnico","donor":"consultas"}' | ConvertFrom-Json
        Get-ProposalSlug $na | Should Be 'soporte-tecnico'
        $l3 = '{"models":"m","baseline":{"label":"x","live":1},"evaluate_before_announce":"on","summary":{"cost_usd":0.0},"findings":[]}' | ConvertFrom-Json
        $na | Add-Member -NotePropertyName finding_id -NotePropertyValue 'finding_1' -Force
        $l3.findings = @($na)
        $t3 = (Format-LoopReport -Loop $l3) -join "`n"
        ($t3 -match 'slug    : soporte-tecnico   \(new_agent of new_agent:consultas, donor consultas\)') | Should Be $true
    }
    It 'explains an infrastructure failure by step and code instead of telling a story' {
        $r = '{"evaluation":{"verdict":"infra_failed","reason":"evaluate returned no verdict","story_text":{"es":"intento 1 paso"},"attempts":[{"problem":{"step":"put_draft","code":"forbidden_role","http":403}}]}}' | ConvertFrom-Json
        Get-ProofLine $r | Should Be 'infra_failed: evaluate returned no verdict (put_draft forbidden_role HTTP 403)'
    }
    It 'prints cost with a dot decimal separator' {
        Format-Cost $loop.findings[0] | Should Be '$0.0012'
        Format-Cost ('{"metering":{"cost_usd":-0.0}}' | ConvertFrom-Json) | Should Be '$0.0000'
    }
    It 'renders one block per finding with all the asked fields' {
        $lines = Format-LoopReport -Loop $loop -Signals @($sig, $sig, $sig) -Mode 'synthetic-planted' -CellsLabel 'planted.ndjson'
        $text = $lines -join "`n"
        ($text -match 'mode: synthetic-planted') | Should Be $true
        ($text -match 'finding_1  M4 pqr_open_rate') | Should Be $true
        ($text -match 'cell    : category=Cobro indebido') | Should Be $true
        ($text -match 'outcome : announced  proposal prop-abc123') | Should Be $true
        ($text -match 'outcome : not_announced:not_fixed') | Should Be $true
        ($text -match 'outcome : unlinked \(dependency_metric\)') | Should Be $true
        ($text -match 'slug    : estado_pqr') | Should Be $true
        ($text -match 'proof   : regression_suite_proven') | Should Be $true
        ($text -match 'cost    : \$0\.0012') | Should Be $true
    }
    It 'shows the ranked candidates, the attempts and a human-owned note when the record has them (MAP1)' {
        $rec = '{"finding_id":"finding_1","metric":"M1","status":"proposed","reason":"compiled","metering":{"cost_usd":0.002},"candidates":[{"rank":1,"target_ref":"template:t/estado_pqr","tried":true},{"rank":2,"target_ref":"prompt:p/resumen_radicado","tried":true},{"rank":3,"target_ref":"template:t/aclarar_cargo","tried":false}],"attempts":[{"rank":1,"target_ref":"template:t/estado_pqr","status":"proposed","reason":"compiled","outcome":"not_announced:not_fixed","proof":"not_fixed"},{"rank":2,"target_ref":"prompt:p/resumen_radicado","status":"blocked","reason":"compile_denied:edit_budget_exceeded","outcome":null,"proof":null}]}' | ConvertFrom-Json
        $h = '{"finding_id":"finding_2","metric":"E2","status":"human_owned","reason":"policy_dispute_amount","metering":{"cost_usd":0.0},"human_owned":{"owner":"riesgo","note":{"es":"Para una persona."}}}' | ConvertFrom-Json
        $l2 = '{"models":"m","baseline":{"label":"x","live":1},"evaluate_before_announce":"on","summary":{"human_owned":1,"cost_usd":0.002},"findings":[]}' | ConvertFrom-Json
        $l2.findings = @($rec, $h)
        $t = (Format-LoopReport -Loop $l2) -join "`n"
        ($t -match 'candidates: 1=template:t/estado_pqr\*  2=prompt:p/resumen_radicado\*  3=template:t/aclarar_cargo ') | Should Be $true
        ($t -match 'hypothesis of where to intervene, not a cause') | Should Be $true
        ($t -match 'tried   : #2 prompt:p/resumen_radicado -> ; proof ; status blocked/compile_denied:edit_budget_exceeded') | Should Be $true
        ($t -match 'human   : for a person \(riesgo\): Para una persona\.') | Should Be $true
        ($t -match 'human-owned 1') | Should Be $true
    }
    It 'says so when the sensor binary was not found' {
        ((Format-LoopReport -Loop $loop) -join "`n") | Should Match 'sensor binary not found'
    }
    It 'prints the dossier ES and the registry read-back, and a line when there is none' {
        $v = Format-DossierView -Record $loop.findings[0] -Registry ([pscustomobject]@{ state = 'draft'; origin = 'auto_detect'; agent = 'consultas'; rev = 1; changes = 2; created_by = 'pulso-engine' })
        ($v -join "`n") | Should Match 'TITLE: template:t/estado_pqr'
        ($v -join "`n") | Should Match 'state=draft origin=auto_detect agent=consultas rev=1 changes=2'
        ((Format-DossierView -Record $loop.findings[2]) -join "`n") | Should Match 'no dossier'
    }
    It 'lists only announced records' {
        @(Get-AnnouncedRecords -Loop $loop).Count | Should Be 1
    }
}

Describe 'New-AnnouncePayload' {
    $loop = $loopJson | ConvertFrom-Json
    It 'derives the opaque evidence link exactly like the engine (CASE- + 26 Crockford characters)' {
        Get-OpaqueLink -EvidenceRef 'ev_0123456789abcdef' | Should Be 'CASE-B6RSPFBMTBH7D97263Z4YZVJ4Q'
    }
    It 'builds the platform body from the dossier' {
        $p = New-AnnouncePayload -Record $loop.findings[0]
        $p.proposalId | Should Be 'prop-abc123'
        $p.title | Should Be 'template:t/estado_pqr - M4: propuesta de cambio'
        @($p.evidenceLinks).Count | Should Be 1
        ($p.evidenceLinks[0] -match '^CASE-[0-9A-HJKMNP-TV-Z]{26}$') | Should Be $true
    }
    It 'cuts an over-long title to 120 characters at a word' {
        $r = $loopJson | ConvertFrom-Json
        $r.findings[0].evaluation.dossier.es.title = ('palabra ' * 30)
        $p = New-AnnouncePayload -Record $r.findings[0]
        ($p.title.Length -le 120) | Should Be $true
        $p.title.EndsWith([string][char]0x2026) | Should Be $true
    }
    It 'refuses personal-data shapes and non-announced records' {
        $r = $loopJson | ConvertFrom-Json
        $r.findings[0].evaluation.dossier.es.sections.problem = 'write to someone@example.com'
        { New-AnnouncePayload -Record $r.findings[0] } | Should Throw 'problem'
        $r.findings[0].evaluation.dossier.es.sections.problem = 'id 123 456 789 012'
        { New-AnnouncePayload -Record $r.findings[0] } | Should Throw 'personal-data'
        { New-AnnouncePayload -Record $loop.findings[1] } | Should Throw 'dossier'
    }
}

Describe 'Read-JsonFile' {
    It 'reads UTF-8 without a BOM keeping the accents (Windows PowerShell would otherwise read it as ANSI)' {
        $f = Join-Path ([IO.Path]::GetTempPath()) ('dl-' + [guid]::NewGuid().ToString('N') + '.json')
        $text = '{"t":"estado ' + [char]0x00F3 + ' ' + [char]0x00F1 + '"}'
        [IO.File]::WriteAllText($f, $text, (New-Object Text.UTF8Encoding($false)))
        (Read-JsonFile -Path $f).t | Should Be ('estado ' + [char]0x00F3 + ' ' + [char]0x00F1)
        Remove-Item -LiteralPath $f -Force
    }
}

Describe 'Format-ProbeReport' {
    It 'lists probe findings with their synthetic evidence class' {
        $rep = '{"action":"none","run_index":1,"confirmed_scenarios":[],"flaky":[],"signals":[{"metric":"P1","dims":{"agent":"consultas","scenario_family":"attacker:vague_customer"},"status":"candidate","reason":"no_previous_run"}]}' | ConvertFrom-Json
        $t = (Format-ProbeReport -Report $rep) -join "`n"
        $t | Should Match 'P1  agent=consultas scenario_family=attacker:vague_customer  status=candidate'
        $t | Should Match 'probe_synthetic'
    }
    It 'says when nothing was found' {
        $rep = '{"action":"none","run_index":1,"confirmed_scenarios":[],"flaky":[],"signals":[]}' | ConvertFrom-Json
        ((Format-ProbeReport -Report $rep) -join "`n") | Should Match 'no probe finding'
    }
}

$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$core = (Resolve-Path (Join-Path $here '..')).Path
$compose = Get-Content -Raw (Join-Path $core 'compose.core.yaml')

Describe 'core_app engine grants (12-app-grants.sql)' {
    $path = Join-Path $core 'init\12-app-grants.sql'
    It 'exists' { Test-Path $path | Should Be $true }
    $sql = if (Test-Path $path) { Get-Content -Raw $path } else { '' }
    foreach ($t in 'runs', 'run_idempotency', 'turn_leases', 'turn_results', 'usage', 'handoffs', 'outbox') {
        It "grants DML on $t to core_app" { $sql | Should Match "(?s)GRANT SELECT, INSERT, UPDATE, DELETE ON[^;]*\b$t\b[^;]*TO core_app" }
    }
    It 'grants sequence usage to core_app' { $sql | Should Match 'GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA public TO core_app' }
    It 'never grants to PUBLIC or exporter_ro and is not superuser-wide' { ($sql -replace '(?m)^--.*$', '') | Should Not Match 'TO PUBLIC|exporter_ro|ALL PRIVILEGES' }
    It 'is applied by core-grants after the exporter grants (before the seed)' {
        $compose | Should Match '10-exporter-grants\.sql -f /init/12-app-grants\.sql'
        $compose | Should Match '"src":"init/12-app-grants\.sql","dest":"/init/12-app-grants\.sql"'
    }
    It 'leaves outbox SELECT-only for exporter_ro (exporter grants run first, app grants never touch exporter_ro)' {
        (Get-Content -Raw (Join-Path $core 'init\10-exporter-grants.sql')) | Should Match 'GRANT SELECT ON audit_events, reg_events, outbox TO exporter_ro'
    }
}

Describe 'core_eval_app sequences (15-eval-grants.sql)' {
    It 'grants sequence usage/update' {
        (Get-Content -Raw (Join-Path $core 'init\15-eval-grants.sql')) | Should Match 'GRANT USAGE, SELECT, UPDATE ON ALL SEQUENCES IN SCHEMA public TO core_eval_app'
    }
}

Describe 'LLM gateway pass-through (agent-core 894fa65)' {
    It 'forwards the external llm-gateway URL/token and the eval budgets file, not the removed LLM_ENDPOINTS' {
        $compose | Should Match 'AGENTCORE_LLM_GATEWAY_URL: \$\{AGENTCORE_LLM_GATEWAY_URL:-\}'
        $compose | Should Match 'AGENTCORE_LLM_GATEWAY_TOKEN: \$\{AGENTCORE_LLM_GATEWAY_TOKEN:-\}'
        $compose | Should Match 'PULSO_EVAL_BUDGETS: \$\{PULSO_EVAL_BUDGETS:-\}'
        $compose | Should Not Match 'LLM_ENDPOINTS: '
        $compose | Should Not Match 'PULSO_LLM_API_KEY: '
    }
}

Describe 'Core rate-limit knobs (agent-core 894fa65 rate_limits_from_env)' {
    It 'forwards the four AGENTCORE_RATE_*/DAILY_BUDGET knobs with Core default values' {
        $compose | Should Match 'AGENTCORE_RATE_MAX_HITS: \$\{AGENTCORE_RATE_MAX_HITS:-30\}'
        $compose | Should Match 'AGENTCORE_RATE_WINDOW_SECONDS: \$\{AGENTCORE_RATE_WINDOW_SECONDS:-60\}'
        $compose | Should Match 'AGENTCORE_DAILY_BUDGET_USD: \$\{AGENTCORE_DAILY_BUDGET_USD:-5\.00\}'
        $compose | Should Match 'AGENTCORE_RATE_SERVICE_MULTIPLIER: \$\{AGENTCORE_RATE_SERVICE_MULTIPLIER:-10\}'
    }
}

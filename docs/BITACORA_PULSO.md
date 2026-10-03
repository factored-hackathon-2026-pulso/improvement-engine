
## 2026-10-03T14:51Z UTC - CLAUDE: run-input slots re-exposed as bind_context facts
- Core 1.3.0 keeps run-input slots claimed; resolve_path/rule data read only validated slots. `pulso/bind_context` now returns the stage-declared inputs as tool-origin facts; Flows read `facts.binding.value.*`. Writer Flow draft plan path fixed to `facts.draft_plan.value.content.*` (artifact_get nests content). InvocationContext.inputs added (invoke/service.py passes inv.input, one line). Scout and writer E2E on real Core + PG16 pass.

## %Y-%m-%dT%H:%MZ CLAUDE: LLM gateway consumer P0 (core-bridge)

- Metering v2 (llm/metering.py, store/receipts.py, migration l3_002): failed calls with usage metered; crossing call counted then refused (ledger outcome over_cap); usage_known honoured (reservation kept); atomic reservation (meter_reserve); model_call_ledger (metadata only). Optional stage pin PULSO_LLM_STAGE_POLICY[_JSON] (alias/model/price from our config).
- Fail-closed: missing/invalid AGENTCORE_LLM_GATEWAY_URL/TOKEN, token "unset", PULSO_CORE_SHA != pin => exit 2 pulso:runtime_config_invalid naming the piece; PULSO_LLM_MODE=disabled explicit escape. Readiness llm_gateway = GET /healthz (<500); key_files also fails on verifier last_reload_error.
- No new receipt outcome (CX-0073/0075): dependency failure = audit agent_step kind=failed + error_kind; verified with real Core + exporter; fixtures in core-bridge/tests/fixtures/dependency_evidence.
- Suites on PG16 (pulso-dev): llm 31, dependency_evidence 8, runtime 79(+4 skip), l3a 81, l5 105, l3b 112, l6 49, integration 37: all pass.

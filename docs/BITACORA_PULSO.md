
## 2026-10-03T14:51Z UTC - CLAUDE: run-input slots re-exposed as bind_context facts
- Core 1.3.0 keeps run-input slots claimed; resolve_path/rule data read only validated slots. `pulso/bind_context` now returns the stage-declared inputs as tool-origin facts; Flows read `facts.binding.value.*`. Writer Flow draft plan path fixed to `facts.draft_plan.value.content.*` (artifact_get nests content). InvocationContext.inputs added (invoke/service.py passes inv.input, one line). Scout and writer E2E on real Core + PG16 pass.

## %Y-%m-%dT%H:%MZ CLAUDE: LLM gateway consumer P0 (core-bridge)

- Metering v2 (llm/metering.py, store/receipts.py, migration l3_002): failed calls with usage metered; crossing call counted then refused (ledger outcome over_cap); usage_known honoured (reservation kept); atomic reservation (meter_reserve); model_call_ledger (metadata only). Optional stage pin PULSO_LLM_STAGE_POLICY[_JSON] (alias/model/price from our config).
- Fail-closed: missing/invalid AGENTCORE_LLM_GATEWAY_URL/TOKEN, token "unset", PULSO_CORE_SHA != pin => exit 2 pulso:runtime_config_invalid naming the piece; PULSO_LLM_MODE=disabled explicit escape. Readiness llm_gateway = GET /healthz (<500); key_files also fails on verifier last_reload_error.
- No new receipt outcome (CX-0073/0075): dependency failure = audit agent_step kind=failed + error_kind; verified with real Core + exporter; fixtures in core-bridge/tests/fixtures/dependency_evidence.
- Suites on PG16 (pulso-dev): llm 31, dependency_evidence 8, runtime 79(+4 skip), l3a 81, l5 105, l3b 112, l6 49, integration 37: all pass.
# 2026-10-03 — CODEX — Consolidated E0 proposal-plan validation and PR preparation

- Rebased and consolidated verified P1/P2/E0 work on live GitHub `main`
  (`589e8dd135da9253a1afeec921bd2768311698d1`); current local head is
  `11615ae2718f1949619229df452bf8c3618ad524`, 14 commits ahead, clean before
  this documentation update.
- Ran `scripts/verify-local-ci.ps1` on Windows at the consolidated head: all
  eight local gates passed, including format, workspace Clippy, Rust tests,
  Python contracts/fixtures, and Pester. The destructive PostgreSQL test was
  intentionally opt-in and ignored.
- Ran the final-head E0 smoke against the augmented sample. It produced one
  descriptive candidate (154/200 recurring-query cases), a replicated but
  descriptive-only holdout (1,433/1,539), and an investigation plan recommending
  mapping review. No exact route was linked; formal route is `do_nothing`,
  review is pending, execution is forbidden, and the plan is not an Agent Core
  artifact. The JSON and NDJSON each contain 16 events and end with the same
  plan status; timeline-parity contracts passed in local CI.
- The repeated OriginalBank scan was stopped before completion because of
  unusually slow I/O (3,696/7,671 files). The previous complete OriginalBank
  smoke remains evidence for the unchanged path; final-head regression tests
  passed, including no-plan behavior. This limitation is explicit in
  `docs/IMPLEMENTATION_STATUS.md` and journal 0065.
- Windows `gh` 2.98 is installed, but its `aleuse` token is invalid and
  `git push` cannot reach `github.com:443`. Published the content-verified
  consolidated snapshot through the authenticated GitHub connector and opened
  PR #78. GitHub compare confirms one commit ahead of main, zero behind, 42
  changed files, and all remote blob SHAs match the local committed files.
- PR #78's `rust-ci` run #127 failed in the PostgreSQL migration, Ubuntu, and
  Windows jobs. The connector exposes no job steps and log retrieval returns
  `BlobNotFound`. The project owner confirmed the GitHub Actions quota is
  exhausted; treat this remote red status as quota-blocked, not as a code
  failure. Local CI passed first; the PR remains reviewable, with merge gated
  by the team's policy for the exhausted-quota checks.

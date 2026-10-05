# B3 live pass (2026-10-05, local stack, aggregates only)

Path: `pulso run` (stub adapter) + `POST /internal/v1/automation/triggers` (explicit) -> worker runs the `trigger:*` job -> value loop
(cells sensor -> Scout, independent Verifier, Builder, recompute, compile -> registry writer as the `builder` principal) against OUR local
agent-core (main `daf4604`, `scripts/dev-stack`, not the shared Core) and the local llm-gateway. No E0 data. Credentials went into the process
environment only. Models: Scout and Builder `xiaomi/mimo-v2.6-flash`, Verifier `xiaomi/mimo-v2.6-pro` (independence label `same_family_other_tier`).
Rubric judge: `z-ai/glm-5.3-flash` hook (`scripts/scoring/gateway_judge.py`), other family.

## Results

| Item | Value |
|---|---|
| Data | bank-derived treated cells (`scripts/aggregate/bank_cells.py`, k>=10), explicit opt-in flag on |
| Sensor (claude-standin) | 17 corroborated findings, 7 skipped (not corroborated) |
| Findings score vs OPBENCH-lite (`score_findings.py`) | recall 1.0 (3/3), precision 0.176 (3/17; the catalog holds only 3 positives), Spearman 1.0, non-findings reported 0 |
| Reasoned (cap 17) | 17: 7 unlinked (M6, no mapping row), 10 blocked, 0 proposed |
| Blocked reasons | `compile_denied:direction_mismatch` 5, `model_invalid` 3, `compile_denied:target_mismatch` 1, `model_refused` 1 (stable across two runs) |
| Proposals created in the local registry by the loop | 0 |
| Rubric score (`score_proposal.py`) | not scored: no proposal compiled |
| Baseline | `fixture-baseline` (the local registry holds only `pulso-builder`, 29 fixture artifacts, 0 live) |
| Synthetic pass (5 cells grid, 1 finding) | blocked `model_invalid` at the Builder |

Opt-in live tests that DID run on this stack: `registry-writer --test live` 2/2 (draft delivered as the engine `builder` principal, retry idempotent;
`pulso-builder` run delivers a validated draft). `reasoning --test live` 0/3 with flash/pro (Builder answers not JSON or wrong direction; timeouts).

## Reading

The loop is wired and honest end to end (trigger -> job -> findings -> typed outcome per finding), but with `mimo-v2.6-flash` as Builder it did not yet
yield a compiled proposal: the gateway reports `el contenido no es JSON` for Builder calls, and when JSON arrives the Builder states a direction the
compiler rejects. Scout and Verifier answers were accepted. Open: Builder prompt/response-format work (or a stronger Builder tier), and live artifacts
in the local registry so patch proposals are not `base_missing`.

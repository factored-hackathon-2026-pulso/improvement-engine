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

## BLD1 (2026-10-05): Builder robustness, tiers, live drafts (aggregates only)

Stack: own prefix `pulso-bld1` (agent-core main `5e3fef9`, registry-e2e imported, baseline label now `live-registry`, 29 live artifacts, 0 fixture).

Diagnosis (raw answers captured outside git, flash and pro, 17 findings, 10 linked): packaging was never the problem in `text` mode (all answers bare JSON; no fences, prose or reasoning text). Real failure classes, flash first run: Builder `expected_direction: increase` 8 of 10 (the model means "resolution rises"), `target_ref` written as the agent slug 2 of 10 (both then reported as compile denials), a TPS refusal for the dimension value `Web Chat` (the one `model_refused`), Scout key typo `mechan_class` 3 of 30, one Builder key that lost its opening quote 1 of 30. In `prompted` mode the gateway discarded 8 of 60 answers (`el contenido no es JSON`: stray key or malformed). Pro: direction wording identical, plus 504 timeouts 3 of about 40. No refusals, no truncation at 4000 tokens.

Changes: engine derives direction (decrease for an up finding), target and kind, the model's wording cannot fail a proposal; `text` mode (no gateway schema, tolerant extraction of one object, strict validation after; `extract.rs`); dimension values become opaque slugs; closed-vocabulary prompts with exact output skeletons; retries (max 2) with the parse or compile problem fed back; Builder escalation flash to pro only after the primary fails (`PULSO_LLM_GATEWAY_BUILDER_ESCALATION_MODEL`), tier, tokens, cost and latency recorded per finding (`metering`); M6 unlinked with reason `dependency_metric` (also `dependent_on_m1`, `level_risk_human_owned`) and not counted as failure.

| Measure (10 linked findings x 3 reps, Verifier pro) | flash Builder | pro Builder |
|---|---|---|
| compiled proposals | 30/30 (100%) | 30/30 (100%) |
| findings needing a retry | Builder 2, Scout 3 | Builder 3, Scout 2 |
| cost per compiled proposal (all roles) | 0.0012 USD | 0.0024 USD |
| latency per finding (all roles) | 37 s avg, 76 s max | 51 s avg, 120 s max |

Flash passes the 70% bar, so it stays the default tier; pro is the escalation. `reasoning --test live` 3/3 (was 0/3; new agent, template patch, prompt patch, all first attempt).

Value loop (`pulso run` + trigger, cap 7): 6 proposals compiled (all flash, 0.0075 USD total), 1 unlinked (`dependency_metric`), 6 draft proposals created in the local registry as `builder` (state draft, never approved), but each is `draft_invalid`: Core `REG-PIN`, the new-agent closure lacks the donor templates (t/abstencion, t/aclarar, t/traspaso, t/mensaje_largo, t/acuse, t/oferta, t/idioma_no_soportado) and understand-turno. Open item for the compiler.

Rubric (`score_proposal.py`, judge `z-ai/glm-5.3-flash`, other family; suite recorded as not_evaluable pending REG1): totals 15 to 18 of 24 (mean 16.7), verdicts revise 2, reject 4. The rejects are R11 = 0: the engine's own docs text carries a digit run of 6+ (hash or ids); fix pending. Judge fixes: gateway rejects `minimum`/`maximum` (now enum), glm needs 6000 max tokens.

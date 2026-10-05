# Agent-behaviour signals from run exports (TEL1)

Question: can the engine detect "a tool is ALWAYS used" or "handoffs concentrate in one agent/locale"? Before TEL1 the
sensor read bank tables and agent PROBES only. TEL1 adds a deterministic, k = 10 path over real agent-core RUN
exports. Findings are `evidence_class: agent_runs`, kept in their own lists, never pooled with bank M1..M10 nor with
T3's `AG_*`; they are associations, never causes.

## Pieces

| Piece | What |
|---|---|
| `scripts/aggregate/agent_runs/` (X-DOC, untouched) | strict reader: cursor chains ending in an empty page, `(run_id, seq)` dedup, terminal event, handoff union, SHA-256 half split. REUSED through its private helpers |
| `scripts/telemetry/agent_signals.py` | thin adapter: A1..A9 cell tables, level findings, descriptive latency/steps |
| `scripts/telemetry/agent_findings.py` | pipeline: export -> `agent_cells.ndjson` -> Rust `steps_cli cells_agent_runs` -> mapped findings (outside the repo, immutable) |
| `scripts/telemetry/export_runs.py` | local exporter (exporter role of the agent-core TEST issuer) -> the T3 envelope |
| `scripts/telemetry/tel1_stack.py` | the EV2 demo stack under TEL1 names/ports (`pulso-tel1-*`, :55561/:8261/:8061) |
| `seams/crates/steps/src/cells.rs` | dims `agent`, `locale`, `tool` added; `(agent, tool)` cells compare with the OTHER agents on the SAME tool; `run_agent_runs` / `Config::agent_runs()` (support floor 20, same k and multiplicity) |
| `scripts/telemetry/agent_signal_mapping.json` | PROPOSED mapping rows (not merged into `seams/crates/reasoning/fixtures/mapping_table.json`, which lives on the unmerged `claude/map1-real-mapping`) |

## Metrics (binary run-level proportions; period = UTC month of `created_at`; half = T3 split)

A "valid,pos" row ships only if denominator >= 10 and numerator and complement are each 0 or >= 10.

| Id | Numerator / denominator | Dims |
|---|---|---|
| A1 handoff_rate | terminal runs with `run_transferred` or status `escalated` or `closed_by` escalation/transfer / terminal runs | agent, locale |
| A2 fallback_rate | runs with a `decision_made.fallback_depth > 0` / runs with a decision | agent, locale |
| A3 closed_early_rate | outcome abstained, clarify_exhausted, abandoned, cancelled / terminal runs | agent, locale |
| A4 tool_error_run_rate | a `tool_called` with status error/timeout/denied / runs with >= 1 tool_called | agent, locale |
| A5 tool_share | runs calling tool X / tool-calling runs of the agent | agent, tool |
| A6 repeated_tool_rate | one tool >= 3 distinct calls in a run / tool-calling runs | agent, locale |
| A7 retry_run_rate | a `tool_called.attempt > 1` / tool-calling runs | agent, locale |
| A8 slow_tool_run_rate | slowest tool call >= 2000 ms / tool-calling runs | agent, locale |
| A9 long_run_rate | >= 5 steps (distinct tool calls + decisions) / all runs | agent, locale |

Descriptive only (not cells; bucketed, per agent, withheld below 10 runs): tool latency p50/p90 buckets
(`<=50, 100, 250, 500, 1000, 2000, 5000, >5000` ms) and steps per run p50/p90. Dimension labels come from versioned
allow-lists (agents, tools; unknown -> `other`; locale es|pt|other). No ids, no free text, no args or results.

Level finding `always_same_tool` (from the already masked A5 cells): >= 95% of an agent's tool-calling runs call the
tool, in BOTH halves, with >= 30 tool-calling runs per half; `peers_vary` is true when another agent (>= 10 runs per
half) calls it in <= 80%. A flow node that always runs a tool is 100% BY DESIGN: the finding says where to look.

## Proposed mapping rows (hypotheses of where to look)

| Finding | Hypothesis | Guardrail |
|---|---|---|
| level `always_same_tool` | prompt/flow review: unconditional flow node (by design) or reflex tool choice in an agent node | tool_error and handoff not worse |
| A5 vs peers | tool-choice instruction review | resolution not worse |
| A1 by locale | prompt language policy check (reply/clarify language, language_detection thresholds) before the escalation rule | wrong-language reply rate not worse |
| A3 | clarify loop review | handoff not worse |
| A4, A7 | tool contract (timeout, retry, arguments) review | completion not worse |
| A6, A9 | loop guard / stop condition review | resolution not worse |
| A2, A8 | model profile / gateway / tool latency: not a patch kind | none |

## Real run (local stack, SYNTHETIC battery traffic, not real customers)

Stack: own `pulso-tel1-*` containers, agent-core main 630a4a7, real gateway and models, battery tools double.
Battery (`scripts/battery`, 37 scenarios) 3 reps then 15 reps; 104 starts hit HTTP 429 `rate_limited` in the 15-rep pass
(partial volume, recorded). Exported 596 runs (360 disputas, 126 consultas, 110 recepcion).

k = 10 with 3 reps (120 runs: 78 / 21 / 21): 33 cells published, 61 suppressed, ZERO findings (the 100% tool share of
disputas had 23 tool-calling runs in the holdout, below the level floor of 30). With 596 runs: 72 cells, 22 suppressed.
Rule of thumb from this volume: an agent x locale cell needs >= 20 per half just to have a chance of 10 positives and 10
negatives, so rare events (error rate 0-5%) never publish at this scale; only large effects or 0%/100% cells do.

Findings (aggregates, 18 reps): comparative corroborated: A5 consultas calls `obtener_pqr` in 100% of tool-calling runs
vs 40% at the other agents; A7 disputas/pt retries in 51% vs 20%; A9 disputas/es long runs 75% vs 0%. Candidates (no
holdout cell): A1 recepcion/es handoff 77% vs 34%, A3 recepcion/es closed early 23% vs 5%. Level: consultas
`obtener_pqr` (56/56 and 33/33; peers vary), disputas `buscar_transacciones`, `convertir_moneda`, `seleccionar`
(142/142 and 143/143). Reading: these are flow-node tools and the router's handoff, i.e. by design; the sensor finds
the pattern, a human decides if it is a defect. No error-rate finding (A4 = 0 everywhere).

# 0674 EVT3 copilot finding through the real loop (UTC 2026-10-05, CLAUDE) [PARTIAL]

Lane EVT3, branch `claude/evt3-copilot-loop`.

Done and validated locally:
- Drift test `scripts/aggregate/tests/test_payload_drift.py` (payload.rs vs PAYLOAD_KEYS, closed enums, id prefixes; mutation guard). 5 tests green.
- Postgres feed: keyed `payload_sql` (sequence = ANY, chunked) replaces a min..max range read; unit tests (4) green; live Postgres test `pg_live` 7/7 green (new: 10,050 cases past the 10,000 cap, payload keys, salted customer key, degraded role).
- w11 test `second_candidate_is_tried_only...` (7 vs 5): NOT a retry-accounting regression; FLOW1 (127) added two flow_edit candidates for Queja, assertion updated to 7; value_loop 27/27 green.
- demo-loop `-Platform` (cells from the synthetic platform history, PULSO_CELLS_FAMILY=platform, registry-e2e + copiloto-sugerencias, served agents); Pester 42/42.

Live run (stack evt3, agent-core main da491b1 incl. PR 85/86, real models): pulso run, 13 corroborated, 9 reasoned, 4 human_owned (P_TYPE_REASSIGN, no model), 2 unlinked, 3 proposed (P_TOOL_USE suite_refused no_mechanism; P_DRAFT_HEAVY_EDIT and P_DRAFT_REJECT guard_regressed), 0 announced, cost USD 0.0042, 11 model calls, 278 s.
Root cause of the first failure: every p/sugerir evaluation run ended `failed` with zero model calls because the serve calibration (testing.e2e_demo) lacks `cal-sugerencias-provisional`. Fix: `scripts/dev-stack/serve_ports_platform.py` plus the stack.py PULSO_SERVE_CALIBRATION hook (reads the artifact from agent-core's own fixture).

Second live run (same stack, calibration fix active, real models: mimo-v2.6-flash agents, mimo-v2.6-pro verifier): VERIFIED that p/sugerir evaluations now run real model calls (about 220 gateway calls to google/gemini-3.1-flash-lite, the suggester generation profile, in one proof; single-scenario `evaluate` returned verdict pass with per-scenario results). The loop result is unchanged at the top level (13 corroborated, 9 reasoned, 4 human_owned, 2 unlinked, 3 proposed, 0 announced) but the p/sugerir proofs moved from infra_failed to a measured outcome.

Second blocker found and fixed upstream: agent-core `_ProbingGateway` marked ANY GatewayError as failed_infra, so one model `invalid_output` (gemini-3.1-flash-lite omitting `type` in a suggestion, schema violation) turned the whole suite evaluation into `failed_infra` ("HarnessUnavailable: el gateway fallo durante el escenario"). The suggester already absorbs invalid_output by design (regenerate, then degrade). Fix: github.com/pulso-factored/agent-core PR 87 (invalid_output no longer infra; unavailable/timeout/rate_limited/refused still are; test added). After it, the full suite returns a verdict.

Third blocker, NOT resolved: with real models the proofs end `guard_regressed` ("guards failing on the BASE (unstable guard)": guard-es-modo-tools-sin-borrador and sometimes guard-es-pii-fuera-del-borrador). Cause measured: the same fixture generation model (gemini-3.1-flash-lite, prompted structured output) fails the schema (missing `type`, about 12-17 gateway 502 invalid_output per run) and the agent then ends the run `failed`; a guard that expects outcome `completed` flips between runs (about 50% on repeated single-guard evaluations). The 6 finding cases do fail on the base as intended (reply not produced), so the suite discriminates, but a guard that is unstable on the base cannot prove a candidate. Tried: stronger `type` wording in p/sugerir (es and pt) after a registry reset; no effect on the error rate, reverted (not shipped). The p/sugerir prompt is PROVISIONAL in agent-core (never calibrated against a real model).

Result summary: fail-on-base proof for the finding cases: obtained (6 of 6 fail on base). Pass-on-candidate: NOT obtained (candidate never evaluated because the base guards are unstable). Announced draft: NONE (0 announced; no proposal created in the registry by this run). Cost of the runs: about USD 0.005 per loop (engine side).

NOT done: a stable suggester guard on the base (needs either a more reliable generation model/profile or a reliable structured-output setting for p/sugerir in agent-core, or repetition-based guard stability in the proof); a Rust scripted e2e test for the platform family; steps_cli preview in run.ps1 uses the old binary unless PULSO_STEPS_CLI is set.

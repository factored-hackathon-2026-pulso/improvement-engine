# 0674 EVT3 copilot finding through the real loop (UTC 2026-10-05, CLAUDE) [PARTIAL]

Lane EVT3, branch `claude/evt3-copilot-loop`.

Done and validated locally:
- Drift test `scripts/aggregate/tests/test_payload_drift.py` (payload.rs vs PAYLOAD_KEYS, closed enums, id prefixes; mutation guard). 5 tests green.
- Postgres feed: keyed `payload_sql` (sequence = ANY, chunked) replaces a min..max range read; unit tests (4) green; live Postgres test `pg_live` 7/7 green (new: 10,050 cases past the 10,000 cap, payload keys, salted customer key, degraded role).
- w11 test `second_candidate_is_tried_only...` (7 vs 5): NOT a retry-accounting regression; FLOW1 (127) added two flow_edit candidates for Queja, assertion updated to 7; value_loop 27/27 green.
- demo-loop `-Platform` (cells from the synthetic platform history, PULSO_CELLS_FAMILY=platform, registry-e2e + copiloto-sugerencias, served agents); Pester 42/42.

Live run (stack evt3, agent-core main da491b1 incl. PR 85/86, real models): pulso run, 13 corroborated, 9 reasoned, 4 human_owned (P_TYPE_REASSIGN, no model), 2 unlinked, 3 proposed (P_TOOL_USE suite_refused no_mechanism; P_DRAFT_HEAVY_EDIT and P_DRAFT_REJECT guard_regressed), 0 announced, cost USD 0.0042, 11 model calls, 278 s.
Root cause of no announce: every p/sugerir evaluation run ends `failed` with zero model calls because the serve calibration (testing.e2e_demo) lacks `cal-sugerencias-provisional`. Fix written (`scripts/dev-stack/serve_ports_platform.py`, stack.py PULSO_SERVE_CALIBRATION hook) but NOT re-run live.

NOT done: re-run with the fix; suite fails-on-base/passes-on-candidate with real models; an announced draft id; Rust scripted e2e test for the platform family; steps_cli preview in run.ps1 uses the old binary unless PULSO_STEPS_CLI is set.

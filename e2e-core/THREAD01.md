# E2E-THREAD-01 (Q1r): ten-step ratchet, replay on the Python host

Test: `e2e-core/tests/unit/test_e2e_thread_01.py`. Runner and hooks: `e2e-core/src/claude_standin/thread01.py`.
Run (no cargo, no containers): `PYTHONPATH='src;tests'` plus agent-core, `uv run --python 3.12 --with pytest ...`
from `e2e-core`; the sensor step uses the existing runner exe (`ED0_RUNNER_EXE`, default
`D:/cargo-targets/claude-ed0/debug/improvement-engine.exe`; step 2 reports `blocked(sensor-exe)` without it).

| # | Step | Replay label | Real when |
|---|---|---|---|
| 1 | data wakes the engine | stand-in (manual command) | scheduled ingest (DEMO-2) |
| 2 | signals, discards | real-narrow (Rust sensor, synthetic package) | |
| 3 | scout + separate verifier | agent_roleplay (recorded, shim replay-only, TPS scanner, ED0L recompute) | live responder lane (INT0) |
| 4 | opportunity, do_nothing | agent_roleplay; target from the SMAP catalogue | |
| 5 | concrete change | real-narrow with `CoreHooks.dry_run` (live INT0); stand-in (CMPpy) in replay | |
| 6 | base vs candidate, 2 gates | stand-in (GSIpy, structural) | `CoreHooks.run_arms` (verdict stays stand-in) |
| 7 | revision | not_exercised unless the gate fails | V3r |
| 8 | human authority | simulated issuer, JWS bound to the draft digest | Core verifies the JWS (INT0) |
| 9 | staging + alias read | stand-in (registry double) | `CoreHooks.publish` + `alias_read` |
| 10 | observation | simulated (platform-sim), observation only | |

Replay fixtures: `tests/fixtures/thread01_queue/responses` (synthetic, role-played). Re-record with
`python -m claude_standin.thread01 --record <dir>`; a drift test fails if the scripted responder and the fixtures differ.
The final report is built by `build_report` and must pass `contracts/engine-run` `check()`; `doubles[]` comes from
`generate_doubles`. Raw E0 is never read: packages are synthetic and live in a temp workdir.

## INT0 status (real-Core hooks)

`claude_standin/core_hooks.py`: `make_dry_run`, `make_alias_read` and `RealCore` (frozen proposal, native evaluation, Core
arms, human-JWS approval, publish, alias read after publish, `gate_probe`, seconds per live call in `timings`).
Live evidence on PG16 + the real Core image (pin c814c2b), `tests/live/test_08_*` and `tests/live/test_09_*`:

| Step | Now | Evidence |
|---|---|---|
| 5 concrete change | real-narrow | Core dry-run digest; the real writer stage then freezes the thread's OWN draft and its `candidate_hash` equals that digest (test_09 windows 1-3) |
| 6 arms | stand-in, `blocked(jev)` | `RealCore.run_arms` is unit-tested, but every Core arm on the atencion world ends `failed_infra: DecisionConfigError` in all three profiles (test_09 `test_step_6_*`): its decision models use provider `jev`, which the Core does not compose (agent-core PR 28, `jev_base_url_not_configurable`) |
| 8 human authority | simulated, `blocked(jev)` | Core approve needs `evaluated`; the native evaluation of the atencion suite ends `failed_infra`, so the human-issuer JWS gets `409 illegal_transition` (the JWS passes Core auth, state is refused). `RealCore.approve` is unit-tested and flips the step when the evaluation can pass |
| 9 publish + alias read | stand-in, `blocked(jev)` | publish is refused the same way and staging is unmoved (`staging_unchanged`); `RealCore.publish/alias_read` ready (alias read refuses staging before publish; step 9 requires the alias read to show the published release) |

Doubles named in `doubles[]` when the hooks run: control-api broker/lab/bank (`e2e-fixtures`), local human issuer (internal
container), roleplay shim (3-4), GSIpy verdict judge (6), platform-sim (10). The writer stage uses no model.
Seconds per live call (3 windows, whole thread plus freeze/evaluate/approve/publish probes 3.7-4.4 s): alias read 0.02-0.03,
dry-run 0.05, writer stage 0.73-0.77, admission 0.03-0.05, evaluate-only 0.58-0.64, proposal read 0.20-0.22, approve/publish
attempt 0.22-0.25, Core arm attempt 0.11-0.13 (`effects.thread01_windows` in the e2e report).
Stale note removed: `core_state_aliases_not_implemented` in `codex_standin/report.py` (alias reads answer 200).

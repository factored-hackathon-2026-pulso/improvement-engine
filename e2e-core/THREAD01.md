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
| 5 | concrete change | stand-in (CMPpy) | `CoreHooks.dry_run` |
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

`claude_standin/core_hooks.py` provides `make_dry_run` (step 5, real Core authoring dry-run through the bridge, base
release = `atencion` prod alias) and `make_alias_read`. Verified live once on PG16 + the real Core image (pinned
agent-core c814c2b) with `e2e-core/tests/live/test_08_thread01_core_hooks.py`: step 5 flips to `real-narrow`,
no step is red. NOT done, still stand-in: `run_arms` (step 6) and `publish` (step 9) need a frozen proposal of the
thread's draft written by a writer stage (scripted model, control-api/gateway doubles) and a human-issuer JWS bound to
it; `alias_read` is only supplied together with `publish`, so step 9 stays `stand-in`. Step 8 JWS stays the local simulated issuer.

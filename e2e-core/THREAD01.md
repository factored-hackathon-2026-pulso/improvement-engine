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
| 8 | human authority | simulated issuer, JWS bound to the draft digest; `blocked(gate)` after a failed gate unless a labelled human override | Core verifies the JWS (INT0) |
| 9 | staging + alias read | stand-in (registry double) | `CoreHooks.publish` + `alias_read` |
| 10 | observation | simulated (platform-sim), observation only | |

Replay fixtures: `tests/fixtures/thread01_queue/responses` (synthetic, role-played). Re-record with
`python -m claude_standin.thread01 --record <dir>`; a drift test fails if the scripted responder and the fixtures differ.
The final report is built by `build_report` and must pass `contracts/engine-run` `check()`; `doubles[]` comes from
`generate_doubles`. Raw E0 is never read: packages are synthetic and live in a temp workdir.

## INT0 status (real-Core hooks)

World: the thread targets the seeded base world `attention-task` (declaration `agent-core-assets/worlds/seeded-base.world.yaml`),
agent `atencion-tarea`: a task agent (tool, rule and respond nodes, no decision model). The conversational `atencion` agent of
`attention-demo` needs provider `jev`, which the Core at pin c814c2b does not compose (`DecisionConfigError`, agent-core PR 28);
`tools/worldcheck.py` (`needs_decision_provider`) keeps the seeded agent free of any decision model, so arms and the native
evaluation complete on the pinned Core with no agent-core or Codex dependency.

`claude_standin/core_hooks.py`: `make_dry_run`, `make_alias_read` and `RealCore` (frozen proposal, native evaluation, Core
arms, human-JWS approval, publish, alias read after publish, `gate_probe`, seconds per live call in `timings`).
Live evidence on PG16 + the real Core image (pin c814c2b), `tests/live/test_08_*` and `tests/live/test_09_*`
(`test_thread01_steps_5_6_8_9_are_real_narrow_against_core[1..3]`, one fresh stack per window):

| Step | Now | Evidence |
|---|---|---|
| 5 concrete change | real-narrow | Core dry-run digest; the real writer stage freezes the thread's OWN draft and its `candidate_hash` equals that digest |
| 6 arms | real-narrow (verdict judge stays the GSIpy stand-in) | 6 Core arm runs per window (3 cases x base/candidate, profile `evolution_task`, scripted tools, scripted llm-gateway double for the closing reply), all `completed` |
| 8 human authority | real-narrow (issuer is the local human-issuer double) | Core native evaluation `pass`, then approve with the human-issuer JWS verified by Core; tampered hash and replay refused by Core |
| 9 publish + alias read | real-narrow | publish to staging, alias read after publish shows the new release (`rel-253b52903d377aae`), prod unchanged |

Honest findings of the live windows:
- The GSIpy structural gate reports `fail: no_structural_improvement` in every window: base and candidate complete the same cases.
  This is not a world-authoring gap that can be fixed: the Core arms report only `status`, `closed_early`, cost and usage, and the
  Core's responder absorbs every prompt-driven failure (missing locale, rejected draft, gateway error) into a template fallback, so a
  Replace-Prompt + Add-EvalSuite change cannot move any observable arm field without the scripted model double encoding the
  "improvement" itself (circular, authored by us). The thread therefore does NOT proceed silently: after a gate that did not pass,
  steps 8 and 9 are `blocked(gate)` (step 10 `not_exercised`) unless `ThreadConfig.human_override` (by=human, actor, reason) is given.
  The live windows pass it, so steps 8-9 run as an explicit labelled human OVERRIDE of a failed gate: `report.gate.verdict`,
  `report.overrides[]` (step, of, verdict, by, label=human_override, reason, actor), a `doubles[]` entry `gate.override`,
  `quality_claims: forbidden`; G1 `check()` rule `G1` rejects approval/publish after a gate that did not pass without it (and an
  override on a passing gate, and an exercised approval/publish without a reported gate verdict). The Core native evaluation
  (`evaluated` before approve) still has to pass independently of the structural gate.
- A window publishes prompt and suite 2.0.0 (immutable in the registry): one window per fresh stack
  (`run.ps1 -PytestArgs '-k','steps_5_6_8_9 and [N]'`); a second window on the same stack is skipped.
- A non-integer JSON number in a draft (suite seed `120.5`) made the writer's put_draft commitment be denied
  (`auth_insufficient`, `manual_reconcile`): Core and the Python draft digest canonicalise it differently. Integers and strings pass;
  `worldcheck` rejects non-integer numbers (`non_integer_number`).
- `RealCore.approve` now runs (once) the Core native evaluation first; the registry approves only an `evaluated` proposal.

Doubles named in `doubles[]` when the hooks run: control-api broker/lab/bank (`e2e-fixtures`), scripted llm-gateway (the agent's
closing reply), local human issuer (internal container), roleplay shim (3-4), GSIpy verdict judge (6), platform-sim (10).
The writer stage uses no model.
Seconds per live call (3 windows, whole thread incl. freeze, evaluation, approve, publish 6.7-6.8 s): alias read 0.01-0.06,
dry-run 0.05, writer stage 0.69-0.77, proposal read 0.20-0.25, Core arm run 0.09-0.31 (6 per window), admission 0.03-0.05,
evaluate-only 0.86-0.91, approve 0.22-0.23 (wrong-hash attempt 0.20-0.27, replay 0.22-0.25), publish 0.27-0.30
(`effects.thread01_windows` in the e2e report).
Stale note removed: `core_state_aliases_not_implemented` in `codex_standin/report.py` (alias reads answer 200).

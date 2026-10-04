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
`generate_doubles`. Default runs never read E0: packages are synthetic and live in a temp workdir. Only the explicit
`--e0` window reads a local E0 package at runtime; there steps 1-2 carry `data_class: E0`, steps 3-4 `original-treated`
(never `generated_sample`, the class `gw-hosted` accepts), and only scanner-passed k-anonymous aggregates reach a responder.
The responder id in a step's `model` label (`agent_roleplay:<id>`) is DECLARED by the answering lane in its response file
(shim-validated for form only); it is not a verified model identity.
In DEMO-0 the "human" of a gate override and of the issuer is SIMULATED: `human_override` is a config value written by the
run's author, so `report.overrides[].simulated` is `true` (G1 rejects a DEMO-0 override without it) and `doubles[]` says
"simulated human".

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

## Live roleplay window 1 (synthetic package, 2026-10-04)

Label `model=agent_roleplay`, `quality_claims: forbidden`, `data_origin=generated_sample`, host python. Shim in LIVE mode
(`python -m claude_standin.thread01 --live <queue> --workdir <dir> --summary <json>`): the thread talks to the shim over real HTTP
on loopback, the queue directory lives outside tracked paths, and each request was answered by a fresh-context subagent that saw
only its one request file and the RUNBOOK responder contract: scout (haiku), verifier (opus), builder (fable); distinct responder
ids per call; none is the implementer of the code. The llm-gateway container was not used (the thread calls the shim directly);
no containers ran. Only treated payloads passed the TPS scanner (0 rejections).

| Measure | Value |
|---|---|
| Responder calls served | 6 (scout 2, verifier 2, builder 2); 7 subagents spawned (1 re-ask) |
| Wall minutes (whole thread, incl. subagent latency) | 4.14 |
| Scanner rejections | 0 |
| Shim response rejections | 1 (scout step 1: the answer omitted `quality_claims`; renamed `.rejected.json`, ledger `response_rejected`), re-asked once, valid |
| `responder_timeout` (55 s hold) | 2, both on the builder's last call; the late answer served the retry |
| Steps | 1 stand-in, 2 real-narrow, 3 scout and verifier agent_roleplay, 4 agent_roleplay, 5 stand-in, 6 stand-in, 7 not_exercised, 8 simulated, 9 stand-in, 10 simulated |
| G1 `check()` | no violations; ED0L recompute accepted the verifier's verdict |

Findings fixed on the way (a responder sees only the request): the `lab_query` tool schema now names `metric_id` and `window_id`,
and each stage request carries a real `output_schema` for its final answer (it was a stub). Scripted fixtures re-recorded for the
new keys (`thread01_queue`, drift test green). Live answers are kept as a second replay fixture set
`tests/fixtures/thread01_live_queue` (synthetic only), replayed with 0 misses by `test_e2e_thread_01_live_replay.py`.
Haiku omitted a required field once; the lane needs the literal field list in the responder prompt.

## Live roleplay window 2 (real local E0, treated aggregates only, 2026-10-04)

Path: `ED0_E0_PATH` read at runtime by the Rust sensor and by the pyarrow feeder `claude_standin/ed0_feed.py` (columns
`case_id`, `query_signature` only). Group = query signature, outcome = recurrence (case with at least two copilot queries;
the same-signature repeat is empty on the real shape). Case keys and group keys are salted HMACs, the salt is ephemeral and never
stored; k = 10. Run: `python -m claude_standin.thread01 --live <queue> --workdir <dir> --summary <json> --e0` with queue, workdir and
summary under the git-ignored `output/`; no fixture from it is committed. Responders: scout (haiku), verifier (opus), builder
(fable), fresh context, distinct ids per call.

| Measure | Value |
|---|---|
| Treated lab | 3 groups, every cell k>=10 (the per-group counts and rates stay in the git-ignored local summary; real-E0-derived numbers are not committed) |
| Sensor (arranque 200, min support 20) | admitted `e0_recurring_copilot_query_cases`, support and holdout figures kept in the local summary, holdout replicated, 2 discards |
| Responder calls / wall minutes | 5 (scout 2, verifier 2, builder 1) / 2.46 |
| Scanner rejections, shim response rejections, timeouts, re-asks | 0 / 0 / 0 / 0 |
| Steps | 1 stand-in, 2 real-narrow, 3 scout and verifier agent_roleplay, 4 agent_roleplay, 5-10 not_exercised |
| SMAP ending | `unlinked`, reason `no_exact_supported_flow_mapping`; no key to the winning category |
| G1 `check()` | no violations; `data_origin generated_sample`, `quality_claims: forbidden` |
| Leak scan | 0 raw case, query, customer or analyst ids or signatures in queue, workdir, summary, log; DC0 scan clean |

Mapping: E0 categories are hashed query-signature groups that no catalogue entry declares, so `smap.e0_mapping` ends `unlinked`
(valid honest ending); steps 5-10 report `not_exercised` with that reason. G1 rule H5 accepts a null `candidate_created_at` when the compile step is not exercised, so no reserved timestamp is reported.

## Q1 slice: the thread on the Rust shell, host=rust, OFFLINE (2026-10-04)

Code: `seams/crates/thread10` (lane L-E2E) and its Python twin `claude_standin/thread01_rust.py`; tests
`seams/crates/thread10/tests/{ten_steps,successor,resume}.rs` and `e2e-core/tests/unit/test_thread01_rust.py`.
Commands run (no containers, no network, `CARGO_TARGET_DIR=D:/cargo-targets/claude-w4f-q1`, `-j 1`):

    cargo test --offline -j 1 -p thread10
    THREAD10_EXE=<target>/debug/thread10.exe uv run ... pytest tests/unit/test_thread01_rust.py

What runs: the engine executor (`engine::executor`, FileStore) drives the nine handlers (sensors, recompute, validation,
compile, arms, gate, native_eval, authority, publish) over `thread10::double::DoublePort`, an OFFLINE Core double behind
`engine::live::CorePort`. The ten report steps are derived from what the job COMMITTED, not from configuration.

| # | Step | Status on host=rust (offline) | Why |
|---|---|---|---|
| 1 | trigger | stand-in | started by hand (ratchet step 1 stays stand-in) |
| 2 | signals | stand-in | the sensor runner is `synth_runner`, a fixed-output binary that reads no data |
| 3 | scout / recompute | stand-in / real-narrow | claim is a scripted value; the recompute is the Rust step over a synthetic lab row |
| 4 | opportunity / validation | stand-in / real-narrow | change spec is a fixed value; validation is the Rust `intent` step |
| 5 | compile | stand-in | Rust compile step, but the dry-run digest comes from the double, not the Core |
| 6 | gate | stand-in | GSIpy-equivalent Rust gate over double arm reports; arms complete identically, verdict `fail` |
| 7 | revision | not_exercised | V3r is a library hook, not wired into the job |
| 8 | approval | simulated, or blocked(gate) | labelled human override of the failed gate, else blocked |
| 9 | publish | stand-in, or blocked(gate) | registry is a double; effectful handler, never re-run after commit |
| 10 | observation | simulated | platform-sim window; the successor correlation below is real control-api code |

No step is labelled `real`. G1 `check()` passes for the three shapes (completed with override, blocked without override,
denied kind) and rejects a tampered copy (H1, G1).

Negatives (Rust tests): denied kind -> `blocked(kind_not_supported)`, steps 6-10 not_exercised, no candidate timestamp;
failed gate without a labelled override -> steps 8-9 `blocked(gate)`, 10 not_exercised; refuted claim -> compile
`blocked(validation)`; unmatched release -> 503 and no successor (retryable once recorded); replayed event (same id) and a
second delivery -> exactly one successor `successor:<release id>`.

Resume: `kill -9` of the `thread10` process after handler 2, 7 (before the effectful publish) and 8 (right after it), then a
second process with a later lease clock: identical committed event sequence, attempt 2, one publish event, one successor. The successor platform is in-process per process, so "one successor" is per run, not across the kill (the correlation runs after the job commits).

A `kill -9` INSIDE the publish effect (after the effect, before its commit) is not resumed: the executor stops with `NeedsReconciliation(8)`, the effect ledger keeps one line, no publish event, no successor (a reconciler is out of scope).

Post-run note: a MEM1 `demo1_thin` note (durable=false, dies with the process) whose evidence refs are the committed events.

NOT achieved: not on the real Core (the Core is a double; INT0 evidence stays the Python host's), no model-driven scout,
verifier or builder in Rust (steps 3-4 scripted), no real sensor (synth_runner), no gate that passes (a scripted
improvement in the double would be circular), V3r revision not wired, memory not durable (DMEMC), no Pg store conformance
(needs a live database; only the FileStore was exercised), no live Podman pass, `host=rust` carries label DEMO-0 only.
`seams/Cargo.lock` gained the `thread10` member (L-CLIENT integrator to acknowledge).

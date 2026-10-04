# demo-magic evidence (DEMOD, 2026-10-04)

Run: `pwsh scripts/demo-magic.ps1` (`-PaceMs 600` default, `-NoBrowser`, `-NoHold`, `-Port 4020`). It builds `pulso`, builds the console
if node exists (otherwise serves the API only and says so), starts `pulso serve`, runs `pulso demo`, prints the doubles[] first and
the steps with labels, then holds the server until Enter/Ctrl+C and stops what it started.

## What was verified

- `demo-magic-script-output.txt`: the script end to end (`-NoHold -NoBrowser`), exit 0, no `pulso` process left afterwards.
- `01..04-*.jpg`: the built-in browser on `http://127.0.0.1:4021/#/run/run-demo0-evidence` while `pulso demo --pace-ms 1500` ran:
  01 two steps complete and ten `[pending]`; 02 `observation [running]`; 03 completed (state `completed`, all labels, `revision
  [not_exercised]`); 04 after a reload: the banner lists every double (ports, steps, `gate.override`) and the native gate shows `fail`.
- `api-*.json`: the same facts from `/internal/v1/debug` (profile with doubles, runs, graph, gates). `demo-stdout.txt`: doubles first, then steps.
- Labels are exactly what thread10 derives from what the job committed: `recompute` and `validation` are `real-narrow`, no step is `real`.

## Honest gaps (console screens still legacy-client / provider-partial)

- The console data provider is `http` (typed `DebugApi`), but the run view keeps the legacy client for part of its data. Observed live before the fix: the mode banner (profile doubles) and the Gates panel were read once and not refreshed by SSE (early doubles and `not_evaluable` until a reload). FIXED in the console (`debug-console/src/state/sideRefresh.ts`, RunView, App): on `doubles_declared` / `gates_set` (live or caught up after a reconnect) the console refetches the profile and the run's gates, debounced (150 ms, one refetch per burst), without blanking the panel or moving focus. The graph (nodes, labels, run
  state) IS live over SSE.
- Traces panel: degraded by design (`trace_id` is null, 12 of 12 nodes); Investigation, Diff and Decision panels are empty/`unknown`:
  this run produces no hypothesis, proposal or decision events.
- Everything in the run is a stand-in: offline Core double (`thread10::DoublePort`), fixed-output sensor runner, scripted scout and
  opportunity, GSIpy judge stand-in, simulated human issuer and override, in-process platform. No real model, no real human, no quality claim.
  `pulso demo --real-core` is refused honestly (no live Core port is wired into this binary, even if a Core answers).
- `pulso serve` keeps runs in memory unless `--store-dir`; the admin token is ephemeral, passed via the environment, never printed.
- Hard kill of the parent PowerShell: the script starts `pulso serve --exit-on-stdin-eof` with a held-open stdin pipe, so the OS closing the pipe makes serve exit by itself (tested: `serve_exits_when_its_stdin_closes...`). A hard-killed `pulso demo` finishes or fails on its own within seconds. Ctrl+C, Enter, errors and normal exit stop both via try/finally.

## v2 (R1V, 2026-10-04): the panels now show what the thread committed

Before: Trazas, Investigacion, Diff, Decision and the gate details were empty (the engine computed the data, nothing streamed it).
Now `debug_api::panels::project` (pure, one projection for both producers) turns the thread's COMMITTED payload (`thread10::Run.payload`,
also `report.committed`) into `investigation_set`, `alternatives_set`, `diff_set`, `gates_set` and `decision_set`. `pulso demo` emits them as each
handler commits (unchanged panels are not re-sent); `debug-api` ingest of a report that carries `committed` emits the same events.
Nothing is generated: every string is derived from a committed field and every evidence item carries the sha256 of the committed value it summarises.

- Investigation: main hypothesis = the scripted scout claim with the verifier verdict; `change` hypothesis = the compile step's operations
  (old -> new refs) with the gate verdict; evidence supports/contradicts (recompute match, validation checks, gate, native evaluation) and
  limits (scripted scout, fixed-output sensor, verifier independent by actor id only, stand-in judge, offline Core). A refuted claim or a
  verifier equal to the scout shows as counter-evidence.
- Diff: structural diff of the committed draft plan (del/add of the refs, preconditions, plan digest). The prompt/suite TEXT is not in the
  engine output, so the last line says no text diff is shown. A denied compile produces no diff (the `change` hypothesis says `blocked(<reason>)`).
- Gates: native = the safety gate with the offline Core evaluation in its reason, improvement = its own gate with the reason
  (`no_structural_improvement`; the evidence says base and candidate completed the same cases: arms identical), one attempt, `report_ref` = digest of the committed gate output.
- Decision: new route `GET /runs/{id}/decision` and a card: SIMULATED label, issuer, actor, gate state it was taken on, the human_override, reasons;
  `available_commands` is empty (nobody can act). A blocked approval is a `blocked` card.
- Traces: still degraded by design and now says exactly what is missing (spans; no collector or tracing in the engine). No trace_id and no span is invented.
- Console changes (small): decision card, statement of every hypothesis, panels refresh on the new events, trace text.

Verification: Rust tests (`debug-api/tests/panels.rs`, `ingest.rs`, `pulso/tests/live.rs`, `thread10/tests/on_commit.rs`); console 212 unit/component tests;
the full real contract suite (`CONTRACT_TARGET=real`, debug-api binary with `--seed-contract` over the demo store) plus
`tests/contract/panels.contract.test.ts` (`CONTRACT_PANEL_RUN=<run>`: zod schemas, evidence refs resolve, SIMULATED card, null trace ids) all pass;
screenshots `v2/00..06-*.png` (headless Chromium via `v2/capture.mjs`), the built-in browser observation in `v2/live-midrun-observation.txt`, API JSON in `v2/api-*.json`.

Honest gaps: the `change` hypothesis and the diff describe a structural change over a double Core; the verdict is the stand-in gate's, not quality.
`gates.native` changed meaning from the earlier demo (it was the stand-in verdict; now safety + the offline native evaluation, with `fail` living in the improvement gate).
The decision route is read-only: the console cannot act on a simulated decision. Evidence timestamps are the time of the commit that produced the snapshot.


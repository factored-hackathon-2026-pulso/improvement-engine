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

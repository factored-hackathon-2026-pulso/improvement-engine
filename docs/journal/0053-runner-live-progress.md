# Local E2E runner live progress

## Decision and scope

Add an opt-in diagnostic progress stream to the existing `local-sim` CLI so a
developer can distinguish a slow/incomplete phase from a completed run while
the process is still active. This addresses visibility during E2E execution;
it is not a service telemetry pipeline or a new run-event persistence model.

`--progress-jsonl` writes one flushed JSON object per phase transition to
stderr. Schema version 1 permits only the event name `run_progress`, phases
`source_preparation`, `detection`, `post_selection_holdout`, and
`persist_outputs`, statuses `started`, `completed`, `skipped`, and `failed`,
and elapsed milliseconds since progress reporting begins. No run/tenant/source identifiers,
input/output paths, source values, proposal content, or raw error text enter
these records. A skipped holdout means its preconditions did not apply; it is
not a zero finding. When the flag is omitted, the CLI's progress behavior is
unchanged.

The progress stream is separate from the persisted `events.ndjson` domain
timeline. It is neither part of the atomic run artifact nor a replacement for
the U07 durable event ledger. It adds no HTTP listener, OpenTelemetry exporter,
health endpoint, or production monitoring claim. The PowerShell E0 wrapper
does not expose the flag yet.
The `run_id` is generated before the first progress record so a clock error
cannot leave an announced phase without a terminal state.

## RED → GREEN

The first binary integration test passed `--progress-jsonl` and failed before
implementation with `missing value for --progress-jsonl`, proving the CLI did
not accept the intended flag. After implementation, success-path coverage
checks the exact ordered start/terminal transition for each phase, valid
allowlisted JSONL fields, no known case ID sentinels, and an explicit `skipped`
post-selection holdout for the retry-primary fixture. Failure-path tests cover
both source preparation and output persistence: each ends in the matching
`failed` status, does not claim later phases or false completion, and does not
print the fixture path in progress diagnostics. Success asserts elapsed time
is non-decreasing; the implementation uses `Instant`, not wall-clock time.

## Verification

Windows checks on branch `feat/runner-live-progress`:

- The initial binary regression failed before implementation with
  `missing value for --progress-jsonl` (expected RED).
- `cargo +1.98.1 fmt --all -- --check` passed.
- `cargo +1.98.1 test --locked -p improvement-engine-runner --test cli_e2e e0_cli_emits_opt_in_sanitized_live_progress_for_each_execution_phase` passed: 1/1 focused test.
- `cargo +1.98.1 test --locked -p improvement-engine-runner` passed: 7 unit tests and 9 binary integration tests.
- `cargo +1.98.1 clippy --locked -p improvement-engine-runner --all-targets -- -D warnings` passed.
- `git diff --check` passed.
- A real local E0 CLI smoke using 200 Arranque cases and
  `--progress-jsonl` exited successfully and emitted 8 ordered progress
  records (four phase starts and terminal states) in 2,497 ms. Only the
  allowlisted phase/status/duration summaries were inspected; no result
  aggregates or source rows were displayed. This validates progress emission
  against the local package, not discovery quality or business conclusions.

An independent adversarial re-review returned GO after the transition,
failure-path, monotonic-time and journal corrections. No Cargo was run by the
reviewer. A Nexus checkpoint receipt was queued in this worktree's
`.nexus/outbox`; it contains no source values, paths, or secrets and was not
reconciled.

The smoke command was:

```powershell
cargo +1.98.1 run --locked --offline -p improvement-engine-runner -- local-sim `
  --mode local-simulation --source e0 `
  --input 'D:\.codex\factored\pulso_muestra_e0' `
  --output 'target-runner-live-progress\e0-smoke-output-review' `
  --tenant-id pulso_local --observed-cutoff 2026-10-02T18:00:00Z `
  --arranque-cases 200 --progress-jsonl
```

No external service or provider was contacted. The generated smoke output was
under the ignored worktree target directory and was removed after validation.

## Limits

The phase stream shows where the current synchronous process is working; it
does not emit per-case work, restart/recovery events, queue age, OTel metrics,
or remote traces. If stderr is unavailable or fails, opting into progress
returns an error rather than silently claiming progress was delivered.

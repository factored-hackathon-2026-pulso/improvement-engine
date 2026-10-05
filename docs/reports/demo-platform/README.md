# demo-platform: `pulso run` processing data end to end (W7, Team CL)

Run: `pwsh scripts/demo-platform.ps1` (`-NoBrowser -NoHold` for unattended, `-Scenario null`, `-WaitSeconds 240`, `-Port 4021`,
`-Backfill 18000 -Horizon 26000 -Rate 200`, `-SkipConsoleBuild`, `-KeepData`). Guards (Pester: `scripts/demo-platform.Tests.ps1`, 17 tests)
refuse before anything starts: a backfill too short for the sensor's 14-day cold-start gate, a horizon not beyond it, a port or rate out
of range, an unknown scenario, no `uv`.

What it does: builds `pulso`; starts the Python simulator (`platform-sim/product_stream`, `uv run --python 3.12`) writing a platform-shaped
SQLite product source (backfill, then follow mode); starts `pulso run` (loopback only) in platform mode over it; prints the console URL;
waits for the first admitted signal; prints doubles[] FIRST and the findings after; holds until Enter/Ctrl+C unless `-NoHold`; stops
both processes (stdin EOF, then kill) and removes its temp data. `demo-platform-script-output.txt` is a real run (exit 0, nothing left running).

## The wiring (what `pulso run` now does)

```
product SQLite (read-only) --monitor::tick_with--> package + run record (<work>/packages, <work>/runs)
   rust-events sensor (real Rust, cells language/channel, discovery/holdout, Bonferroni, replication, k-anonymity)
   hand-over BEFORE the watermark: admit_keyed("monitor:<run_id>") into the JobRepository, then commit the watermark
worker --claim--> EngineRunner: for each admitted signal  thread10::pipeline::run_signals
   scout/verifier/builder through ModelPort, Core through CorePort -> one proposal -> one ledger verdict -> `proposal_verdict` event
   events/panels/profile/doubles into the debug-api Store in process (RunEventSink) -> the console
worker --complete--> status 'complete' (never claimed again)
```

Configuration (environment only; see `seams/crates/pulso/README.md`): `PULSO_DATA_MODE`, `PULSO_SOURCE_ADAPTER` (`product-sqlite` |
`product-postgres` | `dataset-pg` (also `dataset-raw`, `dataset-augmented` = `dataset-pg` over schema `raw`/`augmented`) | `stub`),
`PULSO_SOURCE_SQLITE`, `PULSO_SOURCE_ID`, `PULSO_WORK_DIR`, `PULSO_POLL_INTERVAL_MS` (the existing knob, milliseconds; there is no
`PULSO_POLL_SECS`), `PULSO_READ_BATCH`, `PULSO_SOURCE_PROVENANCE=simulated|real`, `PULSO_MODEL_PORT=scripted|roleplay|gateway`,
`PULSO_CORE_PORT=offline|live`.

## What is real, what is simulated

Real: the Rust `rust-events` sensor over platform-shaped events (identifiers, enums and times only; never payload, text or customer ids);
the Rust monitor tick with its watermark and idempotent hand-over; the Rust pipeline (`run_signals`), the ledger (immutable entries under
`<work>/pipeline/<run>/ledger`), the verdict derivation (from what the job committed), the debug-api store the console reads.

Simulated or double (each one declared in `doubles[]` / the run title / the profile nodes): the data (simulator; the operator declares it
with `PULSO_SOURCE_PROVENANCE=simulated`, otherwise the run says only what the monitor says), scout and verifier answers (scripted;
`scripted-observing-v1` copies the observed rate, so the independent recompute corroborates by construction; labelled `scripted`, never real),
the Core (offline double `thread10::DoublePort`: it never makes a proposal viable; `PULSO_CORE_PORT=live` selects the real port only when the
whole `engine::real_core` configuration is present, otherwise `pulso run` refuses to start), the human decision, the release and the
observation window. No quality claim anywhere.

Dataset mode (`PULSO_DATA_MODE=dataset`, E0/CSV in `raw`/`augmented` via `dataset-pg`): runs are labelled `demo/replay data, not production`,
the sensor is the fixed-output stand-in (the Rust sensor does not read E0), and the proposals it yields are labelled stand-in as well.
Platform mode: `real platform signals; release and observation simulated until EXT-2` (replaced by `simulated platform-shaped signals` when the
operator declares the source simulated).

## What the console shows for a run

Title (mode, label, sensor, models, Core, human); graph nodes `source [mode | adapter | source id | events watermark_from -> watermark_to |
observed until | history]`, `sensor [...]`, `models [...]`, `ports [core ... | release simulated | observation simulated | human simulated]`, one
`proposal-N [verdict: reason] <cell metric id>` per admitted signal; `run_profile_set` event (same facts, machine readable); banner doubles;
Investigation (with the real sensor signal as supporting evidence and an honest limit for the thread's own stand-in sensors step),
Alternatives, Diff, Gates, Decision (blocked by the failed gate on the double) for the last proposal of the run; `proposal_verdict` per proposal.

## Evidence (this branch)

- `seams/crates/pulso/tests/e2e_platform.rs`: simulator -> spawned `pulso run` -> store. On `escalation_rise` (26000 events, 24 days) the
  planted `reassignment_rate.pt.web_chat` is admitted by the last ticks and nothing else is; every admitted signal has exactly one proposal event
  and one ledger entry, every verdict is `not_viable`/`not_evaluable`; investigation, gates and decision panels are populated; the watermark
  reaches `seq:26000`; idle ticks read nothing; a restart over the same work dir adds no run and no event. On `null` nothing is admitted and no
  pipeline directory exists.
- `run_source_tick.rs`: kill between read and watermark commit (a store that fails the first commit) -> the replay has the same run id, one job, one record.
- `run_engine_job.rs` (11 tests), `run_config_source.rs` (8), `repo_complete.rs` + conformance scenarios and 6 new mutants (pg), `tick_with` tests (sources).
- Live demo output: `demo-platform-script-output.txt`: 8 monitor runs, 1 admitted signal, `not_viable (gate_failed)`.

## Honest gaps

- Postgres is compiled but not exercised here: `PgRepo::complete/admit_keyed/job_key` SQL, `product-postgres`, `dataset-pg`, `PgStore` run only with
  `PULSO_TEST_PG_ADMIN` / DSNs (none in this environment). The pipeline ledger is a file store under the work dir even in Postgres mode.
- `PULSO_STORAGE=memory` keeps the job queue in memory. Review fix: the monitor now re-admits (keyed, idempotent) every run record of its source
  found under `<work>/runs` once per process, so a kill after the watermark commit but before the worker finished no longer loses the job; a run the
  console store already holds as completed is a no-op. A Postgres queue is durable on its own.
- Residual at-least-once overlap: a kill (or a failed hand-over) between reading a batch and committing the watermark replays from the same
  watermark; if the source grew meanwhile the replayed batch has a different end, hence a different run id, and the orphaned first job also runs.
  Events in both batches can yield the same signal twice (two proposals, two verdicts). Not fixed: it needs the batch end pinned before the read.
- `PULSO_SOURCE_PROVENANCE` unset in platform mode is now titled `platform-mode signals; ... did not declare whether the source is real or simulated`,
  not `real platform signals` (that label needs `PULSO_SOURCE_PROVENANCE=real`).
- Shutdown while a job runs: the runner is not interrupted; if it outlasts the grace (default 25 s) the worker is cut, the process exits 3, and the
  lease (900 s) simply expires so the job is reclaimed and the idempotent runner resumes. There is no explicit lease release.
- The console projection holds one investigation/gates/decision per run: with several proposals in one tick run the panels show the LAST proposal;
  every proposal is a graph node and a `proposal_verdict` event. No console TypeScript was changed (no dedicated profile panel).
- The thread's own `sensors` step is still the fixed-output stand-in; the real signal enters through the lab row (numerator/count of the cell). The
  recompute therefore checks the claim against the same aggregate the sensor produced.
- A run per monitor tick: with a long backlog the first runs are cold start (0 admitted signals) and still appear as runs.
- `contracts/engine-run` freeze verified (`python contracts/engine-run/freeze.py verify`); the recompute change is additive (a sensed `metric_id` is
  accepted as a signal id next to the sensor's `sig-NNNN`).

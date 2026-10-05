# debug-api: run-event store and /internal/v1/debug for the console (CON1/E4R/H1 slice)

Date: 2026-10-04
Owner: CLAUDE (lane L-CAPI)

## Scope

New crate `seams/crates/debug-api` (std, `serde_json`, `sha2`, `tiny_http`; no Codex crates, no `crates/core`). It serves the
`/internal/v1/debug` read routes and SSE for the debug-console (our internal backoffice, spec V3 section 25), fed by a
run-event store the engine writes through the `RunEventSink` trait.

## What exists

- `Store` (memory, or a directory with one `<run>.jsonl` per run): per-run monotonic sequence from 1, purge floor, projection
  folded from every event (`project.rs`). Reopening a directory keeps sequence, floor and projection.
- Routes: session (+cookie profile, CSRF), profile, runs, graph, events page, investigation, gates, alternatives, proposal diff,
  memory, decision, decision responses (validated, never executed), spec 25 panel routes (empty pages), SSE on both
  `/runs/{id}/events/stream` (legacy client) and `/runs/{id}/stream` (typed provider). `Last-Event-ID` and `after_sequence`
  resume (larger wins), `: hb` heartbeat comments, 410 `cursor_expired` below the floor or beyond the head with both recovery
  shapes (`recovery_after_sequence`+`snapshot_url`, and `recovery{snapshot_ref,after_sequence}`).
- Honesty: the report `doubles[]` become `profile.doubles` (`<part>:<status>`) with `doubles_detail`, `mode=stand_in`; every
  step keeps its label in the node label (`scout [agent_roleplay]`); the improvement gate is `not_evaluable`, never invented.
- Auth: loopback-only bind; optional static bearer `DEBUG_API_TOKEN`; the `/__admin/v1/*` surface (live append, purge, report
  ingest) is a 404 unless `DEBUG_API_ADMIN_TOKEN` is set. No command is executed (`available_commands[]` comes from data;
  the contract seed offers none).
- `debug-api` binary: `--addr --store-dir --ingest REPORT.json --seed-contract`.

## Verification actually run

- `cargo test -j 1 -p debug-api` (CARGO_TARGET_DIR=D:/cargo-targets/claude-w5a-dbg): 36 tests (store 7, routes 13, sse 6, ingest 6, bin 4).
- `CONTRACT_TARGET=real CONTRACT_BASE_URL=http://127.0.0.1:<port> npm run test:contract` against the binary started with
  `--ingest tests/fixtures/engine-run-demo0.json --seed-contract`: api.contract 20/20 pass, demo.contract 9/9 pass.
- Manual: SSE client on `Last-Event-ID: 14` received the retained event, then a live event appended through the admin route.

## Limits and gaps (honest)

- The contract suite reads `run-active`, `dec-1` and `prop-1` unconditionally, so `--seed-contract` adds a run that says it is
  not an engine run (double `contract_seed:fixture`). Without it, those three tests fail by contract design.
- In `CONTRACT_TARGET=real` about half of api.contract tests return early (`fixtureOnly`: resume, 410, heartbeat, idempotency, CAS);
  those behaviors are covered by the Rust tests (`tests/sse.rs`, `tests/routes.rs`), not by the zod suite. demo.contract always
  spawns its own fixture server.
- No Postgres store (memory and file only). A file purge is logical (history stays in the file). Event page cap 1000, `next_cursor` always null.
- The engine does not call `RunEventSink` yet; only report ingest and the admin append exist. No `sources` or `steps` route
  (the console reads exporter data elsewhere). Session auth is `simulated`; no step-up, no commands.

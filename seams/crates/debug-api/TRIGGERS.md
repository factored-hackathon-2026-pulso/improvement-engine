# Trigger intake: `POST /internal/v1/automation/triggers`

Accepts the `pulso.trigger.v1` request built by `scripts/triggers/agentcore_poller.py` and admits one keyed engine job.

## Contract

- Auth: the same bearer as the other automation routes (when `DEBUG_API_TOKEN` is set) and `X-CSRF-Token` (from `GET /api/v1/auth/session`). Without CSRF: 403, nothing admitted.
- `Idempotency-Key` is required and must equal `trigger_key` (`sha256:` + 64 hex).
- Required: `schema = pulso.trigger.v1`, `tenant` (must equal the engine tenant, else 403), `mission`, `source`, `config_digest`, `kind`, `event.type`, `event.ref`. Ids are opaque (<= 200 chars of `A-Za-z0-9-_.:@/`).
- Accepted (`kind`, `event.type`): `explicit/run.now`, `scheduled/schedule.tick`, `outcome/run.closed`, `outcome/release.published|promoted|revoked`. Anything else: 422 `unknown_kind`.
- Admission: `TriggerAdmitter::admit_keyed(tenant, "trigger:<trigger_key>")`, the same contract as `JobRepository::admit_keyed` (same key, same job id, nothing queued twice). `pulso` injects its repository with `Automation::with_admitter`; the standalone `debug-api` binary uses an in-process `MemoryAdmitter`. No admitter: 503 `admission_unavailable`. A failing store: 503 `admission_failed` and no trace, so the retry admits.
- Result: 202 `{state: "admitted", trigger_key, kind, event_type, job_id, job_key}`.
- Replay (same key, same body): the stored response again (202, header `Idempotency-Replayed: true`), no second job and no second event. Same key, different body: 409 `idempotency_conflict`.

## Audit and projection

- One run-event `automation_trigger_received` in the `automation-audit` run (entity = trigger key).
- `GET /internal/v1/automation/triggers` lists them; `GET /internal/v1/automation/case-types` carries `triggers: {count, latest[5]}`.
- Only whitelisted subject fields survive, and only as ids/numbers: `run_id, agent, release, outcome, closed_by, release_id, proposal_id, candidate_hash, origin, interval_secs, slot`. Free text (`reason`), non-opaque values and unknown fields are dropped and counted in `dropped_fields`; they never appear in events or responses.

## Limits

- Trigger records (replay table, projection) are in memory; after a restart the job store key still prevents a second job, but the audit event may be written again. Persisting the replay table is not done: the job store would need a response column the `JobRepository` contract does not have.
- `pulso run` injects its `JobRepository` (`RepoAdmitter`, B3) and its worker executes `trigger:*` jobs through the value loop (`seams/crates/pulso/src/run/value_loop.rs`); without a configured loop a trigger job is recorded as `skipped`. The runner cannot tell the trigger kind from the key (`trigger:<sha256>`): `outcome` triggers run the loop too (idempotent per finding, but model calls repeat unless the job store already holds the finding records).
- The poller's `HttpSink` sends `X-CSRF-Token`, read from `/api/v1/auth/session`.

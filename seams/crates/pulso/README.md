# pulso

One executable. `pulso run` is the container entrypoint (ECS task definition); `pulso healthcheck` is its
HEALTHCHECK; `serve` and `demo` are the local demo pair.

## `pulso run`

Environment only (twelve-factor). Invalid or ambiguous configuration exits 2 with a named reason
(`config_missing|config_invalid|config_conflict: <VAR> ...`); the value of a secret is never printed.

| Variable | Default | Meaning |
|---|---|---|
| `PULSO_DATABASE_URL` | none | `postgres://` DSN (secret, never logged). Required unless `PULSO_STORAGE=memory`. |
| `PULSO_STORAGE` | `postgres` when a URL is set | `postgres` or `memory` (explicit, ephemeral, no migrations). Both set is a conflict. |
| `PULSO_DATA_MODE` | required | `dataset` or `platform`. |
| `PULSO_SOURCE_ADAPTER` | `stub` | dataset: `stub,dataset-raw,dataset-augmented`; platform: `stub,product-sqlite,product-postgres`. A mismatch is refused. |
| `PULSO_POLL_INTERVAL_MS` / `PULSO_BATCH_CAP` | 30000 / 100 | monitor and worker cadence; max jobs per worker cycle. |
| `PULSO_LISTEN_ADDR` | `127.0.0.1:8080` | Container: `0.0.0.0:8080`. Non-loopback needs `PULSO_ALLOW_NON_LOOPBACK=1` **and** `PULSO_DEBUG_TOKEN` (>= 16 chars); an admin token must then differ. |
| `PULSO_DEBUG_TOKEN`, `PULSO_ADMIN_TOKEN` | none | Bearer tokens for the debug API / admin append (from Secrets Manager). |
| `PULSO_BASE_PATH` | empty | Serve console and API under a proxy prefix (`/pulso`). Only that prefix (plus bare `/healthz`, `/readyz`) is reachable; the console `config.json` gets `apiBase` = the prefix. |
| `PULSO_CONSOLE_DIR`, `PULSO_STORE_DIR` | none | Built console directory; file-backed event store (else in memory). |
| `PULSO_STORAGE_PREFIX` | none | Relative object-storage key prefix (validated, carried for the storage layer). |
| `PULSO_SHUTDOWN_GRACE_SECS` | 25 | Grace after SIGTERM; below the ECS default `stopTimeout` of 30 s (raise both together). |
| `PULSO_TENANT`, `PULSO_WORKER_ID` | `tenant-local`, `pulso-<pid>` | Worker identity for lease/fence. |
| `PULSO_EXIT_ON_STDIN_EOF` | off | Also `--exit-on-stdin-eof`. Off in containers (stdin is /dev/null). |

Start-up: bind the listener, then apply the embedded `migrations/*.sql` (the repo files are compiled into the binary,
so there is no directory to ship) under the pg advisory lock; retried every 2 s while the database is unreachable.
`/healthz` is process-up. `/readyz` is 200 only when migrations are applied (or memory mode), the DB answers and every
task is alive; otherwise 503 with one reason: `migrations_pending`, `migrations_failed`, `db_unreachable`,
`task_starting:<t>`, `task_dead:<t>`, `shutting_down`. Logs are one JSON object per line on stdout.

Stop: SIGTERM/SIGINT (unix), console control events (Windows), or stdin EOF (opt-in). Tasks get the grace period to
return; a task that ignores it is cut and the process exits 3. Exit codes: 0 clean, 1 a task died / startup failure,
2 refused configuration, 3 cut at the deadline.

### The wired tasks (W7)

`run::build_tasks` is the only place tasks are wired. With `PULSO_SOURCE_ADAPTER=stub` (default) the monitor does nothing and the worker has no
runner (health checks). With a real adapter:

- monitor = `run::source::SourceTick`: the real `sources::monitor::tick_with` over `product-sqlite | product-postgres | dataset-pg`
  (`dataset-raw`/`dataset-augmented` = `dataset-pg` over schema `raw`/`augmented`), packages and run records under `PULSO_WORK_DIR`, watermark in
  `<work>/watermarks` (or Postgres via `PULSO_PG_WATERMARK_DSN`). The run is handed to the queue as one job keyed `monitor:<run_id>`
  (`JobRepository::admit_keyed`, idempotent) BEFORE the watermark moves; a kill in between replays the same run id and finds the same job.
- worker = `run::engine_job::EngineRunner`: for every signal the sensor admitted, `thread10::pipeline::run_signals` (one proposal, one ledger
  verdict, one `proposal_verdict` event), with the run, profile nodes, doubles and panels written to the debug-api store in process. The models are
  `PULSO_MODEL_PORT=scripted` (default, `scripted-observing-v1`, labelled) | `roleplay` (`PULSO_ROLEPLAY_QUEUE`) | `gateway` (`PULSO_MODEL_GATEWAY=enabled`
  + the gateway env; a refusal is a `not_evaluable` verdict, never a scripted fallback); the Core is the offline double unless `PULSO_CORE_PORT=live`
  (then `engine::real_core` env is required at start, and the job calls `begin_effect` first so a crash is never an automatic retry).
- Extra variables: `PULSO_WORK_DIR` (required for a real adapter), `PULSO_SOURCE_SQLITE` (required for `product-sqlite`), `PULSO_SOURCE_ID`
  (default `<data_mode>:local`), `PULSO_SOURCE_SCHEMA`, `PULSO_READ_BATCH` (events per read, default 1000, max 10000),
  `PULSO_SOURCE_PROVENANCE=simulated|real` (an operator label; `simulated` replaces the monitor's "real platform signals"), `STEPS_RUNNER_EXE`.
  DSNs (secrets, env only): `PULSO_PG_PRODUCT_DSN`, `PULSO_PG_DATASET_DSN`, `PULSO_PG_WATERMARK_DSN`.

Other long-running pieces implement `run::supervisor::Task` (`run(&mut self, &StopToken)`; it must return promptly after the stop) and are added with
`Supervisor::add`. Monitor and worker are wrapped in `Gated` so they never run before the schema is ready. End-to-end evidence:
`docs/reports/demo-platform/README.md`, `tests/e2e_platform.rs`.

### Container

```
ENTRYPOINT ["/pulso", "run"]
HEALTHCHECK CMD ["/pulso", "healthcheck"]     # GET 127.0.0.1:<PULSO_LISTEN_ADDR port>/readyz, exit 0 only on 200
```

## Tests

`cargo test -p pulso` (config matrix, health state machine, supervisor, HTTP/prefix, tasks, process-level spawn).
`PULSO_TEST_PG_ADMIN="host=... user=... "` additionally runs the two-migrators-one-database test against a real server.

## Known gaps (adversarial review, CL)

- (closed, W7) `JobRepository::complete(tenant, job, worker, fence, now)` is the terminal transition (status `complete`, admitted by migration 0050); the worker calls it when the runner returns `Ok`, so a finished job is never claimed again. A runner that fails or is killed still leaves the lease to expire and the job is retried (at-least-once, fenced). Test: `a_job_whose_runner_returned_ok_is_never_run_again`.
- No read/idle timeout on the HTTP front (a slow client holds one of 256 request threads; at the cap `/readyz` also answers 503). Put a proxy with timeouts in front.
- `PULSO_STORAGE_PREFIX` is validated and logged but not consumed by any component yet.
- The listener stops accepting at SIGTERM (same stop token), so `/readyz` cannot answer `shutting_down` during an LB drain; default `PULSO_SHUTDOWN_GRACE_SECS=25` stays below the ECS default stopTimeout of 30 s (keep it below the task's stopTimeout).
- A dead task keeps the process up and not ready (ECS/ALB replace on the failing `pulso healthcheck`; plain `docker compose restart:` policies do not look at health).

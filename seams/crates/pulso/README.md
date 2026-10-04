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
| `PULSO_SOURCE_ADAPTER` | `stub` | dataset: `stub,e0-raw,e0-augmented`; platform: `stub,product-sqlite,product-postgres`. A mismatch is refused. |
| `PULSO_POLL_INTERVAL_MS` / `PULSO_BATCH_CAP` | 30000 / 100 | monitor and worker cadence; max jobs per worker cycle. |
| `PULSO_LISTEN_ADDR` | `127.0.0.1:8080` | Container: `0.0.0.0:8080`. Non-loopback needs `PULSO_ALLOW_NON_LOOPBACK=1` **and** `PULSO_DEBUG_TOKEN` (>= 16 chars); an admin token must then differ. |
| `PULSO_DEBUG_TOKEN`, `PULSO_ADMIN_TOKEN` | none | Bearer tokens for the debug API / admin append (from Secrets Manager). |
| `PULSO_BASE_PATH` | empty | Serve console and API under a proxy prefix (`/pulso`). Only that prefix (plus bare `/healthz`, `/readyz`) is reachable; the console `config.json` gets `apiBase` = the prefix. |
| `PULSO_CONSOLE_DIR`, `PULSO_STORE_DIR` | none | Built console directory; file-backed event store (else in memory). |
| `PULSO_STORAGE_PREFIX` | none | Relative object-storage key prefix (validated, carried for the storage layer). |
| `PULSO_SHUTDOWN_GRACE_SECS` | 50 | Grace after SIGTERM; keep below the ECS `stopTimeout` (>= 60). |
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

### Registering work (R1M, R1E)

`run::build_tasks` is the only place tasks are wired. Implement `run::tasks::Tick` (the monitor, called once per poll
interval with `TickCtx{data_mode, adapter, batch_cap}`) and replace `StubTick` there. Implement `run::tasks::JobRunner`
and set `runner: Some(..)` on the worker; with no runner the worker claims nothing. Any other long-running piece
implements `run::supervisor::Task` (`run(&mut self, &StopToken)`; it must return promptly after the stop) and is added
with `Supervisor::add`. Monitor and worker are wrapped in `Gated` so they never run before the schema is ready.

### Container

```
ENTRYPOINT ["/pulso", "run"]
HEALTHCHECK CMD ["/pulso", "healthcheck"]     # GET 127.0.0.1:<PULSO_LISTEN_ADDR port>/readyz, exit 0 only on 200
```

## Tests

`cargo test -p pulso` (config matrix, health state machine, supervisor, HTTP/prefix, tasks, process-level spawn).
`PULSO_TEST_PG_ADMIN="host=... user=... "` additionally runs the two-migrators-one-database test against a real server.

# Pulso improvement engine

Autonomous detection and improvement service integrating external Agent Core primitives.

## Current slice

Layer 0 provides a minimal Rust workspace, a public tracer for the core crate, and its first read-only source-boundary slice. The source validator loads canonical JSON contracts in deterministic filename order and compares adapter-supplied bytes with a sealed synthetic snapshot; it emits deterministic findings for contract, header, file-digest and policy drift without opening bank data itself. U05 adds an in-memory, typed quota/grant semantic boundary: a quota is global to its tenant/resource/window even when the immutable `RunConfig` changes; grants have explicit expiry/revocation and reservations yield deterministic idempotent receipts. U06 adds the executable reducer contract for durable job admission: it binds a versioned run configuration, trigger and full quota request; grants a fenced, owner-bound lease; records an ambiguous effect as reconciliation-required before dispatch; and never makes a quota-deferred receipt executable. The in-memory reference adapter is exposed behind `DurableJobRepository`; a durable adapter must provide the documented transactional/conditional-update semantics before it can claim restart or multi-process durability. It establishes a reproducible local verification command:

```powershell
cargo test --workspace
python -m unittest discover -s tests -p "test_*_contract.py" -v
python contracts/validate_fixtures.py
```

The GitHub Actions workflow additionally checks formatting, Clippy, Rust unit/integration harnesses, contract unit tests and the complete contract-fixture validator on Windows and Linux. U02 adds tenant-scoped immutable artifact repositories and a PostgreSQL migration/adapter; U03 adds synthetic sealed-source validation. U04 adds an in-memory, discovery-safe projection for adapter-supplied E0 enriched-history rows: it seals namespace/world/cutoff plus file/schema/transform/policy digests and availability per field/group, blocks labels/precomputed signals/final outcomes, and blocks exposure entirely when provenance drifts. U15 mounts an exactly-authorized immutable `memory_wiki` revision into an in-process, ephemeral scratch workspace; it can read and apply typed atomic transformations there, but cannot publish, alter the immutable source, open a host path or use the network. U33 publishes a verified U15 result as the next immutable revision behind an exact world/campaign/protocol/partition head, records permitted use, and applies append-only revocation tombstones before future use. It does not implement detection, an Agent Core runtime, a model gateway, authenticated real-data ingestion, or any external call. Infrastructure lives in sibling `infra`. See the [deployment-boundary contract](docs/architecture/deployment-boundary.md) for local/AWS ownership and the current blocked ingress, database-secret, egress and observability dependencies.

U07 adds a framework-neutral, tenant-scoped run-activity read boundary. Its
projection deduplicates immutable job events and orders them by
`(occurred_at_unix_seconds, event_id)`; the handler exposes bounded pages and
server-side opaque cursors, bound to an authenticated tenant DTO. A changed
snapshot returns explicit expiry, retention physically removes events and
returns explicit purge, and a transport-neutral resumable stream-batch port
is available for a later SSE runtime. Activity reads never mutate the
projection or durable jobs.

`rust-ci` owns its ephemeral PostgreSQL service and runs U02's ignored
destructive migration test against it. The test URL and consent only exist in
that isolated CI job; no GitHub secret, external reusable workflow or deployed
infrastructure is involved. This keeps the engine's durable adapter gate
self-contained while sibling `infra` owns Terraform, AWS deployment and
operational infrastructure.

## Python integration packages and console

Besides the Rust workspace, this repository carries Team Claude's integration packages (Python 3.12 via
`uv --python 3.12`, plus the Node debug console). They sit outside the Cargo workspace (`members` lists only
`crates/*`) and are not yet run by `.github/workflows/ci.yml`, which is Rust-only; they are verified locally.
Everything marked as a double is a double, never the real Agent Core, control-api or lab-broker.

| Directory | What it is | Local check |
|---|---|---|
| [`core-bridge/`](core-bridge/README.md) | `pulso_core_runtime`: the pinned Agent Core composed with Pulso's `/internal/v1` API, tool runtime, native evaluation and read-only exporter; image and wire snapshot | `pwsh core-bridge/scripts/ci.ps1 -Job all -PostgresAdmin <throwaway PG16 DSN>` |
| [`bridge-contract/`](bridge-contract/README.md) | published `/internal/v1` contract for the Rust client: OpenAPI, JSON Schemas, goldens, conformance kit, mock divergence report | `python bridge-contract/gen.py --check` and `python -m pytest bridge-contract` |
| [`platform-contract/`](platform-contract/README.md) | JSON Schemas and event catalog for the allow-listed support-platform tables, goldens, conformance suite | `uv run --python 3.12 --no-project --with pytest --with jsonschema python -m pytest platform-contract/tests` |
| [`platform-sim/`](platform-sim/README.md) | doubles: registry mock, a2 harness, bridge mock, ingest fixture; [`platform_live/`](platform-sim/platform_live/README.md) simulates the 11-table platform model with fault injection | `pwsh core-bridge/scripts/ci.ps1 -Job platform-sim` |
| `platform-exporter/` | read-only exporter of platform `event_log` into `PlatformObservationBatch` (persist before POST, redaction, quarantine, gap findings) | `uv run --python 3.12 pytest` from `platform-exporter/` |
| [`local-identity/`](local-identity/README.md) | sandbox-only human issuer for Core human Principal JWS; never for staging or prod | `uv sync --python 3.12 && uv run pytest` from `local-identity/` |
| `e2e-core/` | Codex stand-in (Python) driving the real local Core stack end to end, with an honest `e2e-report.json` | `pwsh e2e-core/run.ps1 -UnitOnly` (without the stack); `pwsh e2e-core/run.ps1` (full, Podman) |
| [`demo/`](demo/README.md) | demo steps on the real local stack, each marked `real`, `stand-in` or `simulated`; feeds the console | `pwsh demo/run.ps1 -Offline` (no stack) |
| [`local/core/`](local/core/README.md) | standalone Podman stack for the real Core (`real_local`), doctor and smoke scripts | `Invoke-Pester local/core/tests` and `python -m pytest local/core/tests` |
| [`agent-core-assets/`](agent-core-assets/README.md) | generic Scout/Verifier/Builder/Writer stages, worlds and expected state, with a validator | `uv run --python 3.12 --with pyyaml python tools/assetcheck.py check` from `agent-core-assets/` |
| `debug-console/` | internal backoffice of the detection and self-improvement system (spec V3 section 25; not a product UI): see [its docs](debug-console/docs/README.md) | `npm ci && npm run typecheck && npm test && npm run build` from `debug-console/` |

### Agent Core pin

The engine consumes Agent Core at a pinned commit. Do not copy a SHA from this README: the authoritative value is the
`PIN_SHA` constant in [`core-bridge/src/pulso_core_runtime/__init__.py`](core-bridge/src/pulso_core_runtime/__init__.py),
and the reasoning for each bump is in the ADR series under [`core-bridge/docs/adr`](core-bridge/docs/adr/README.md).
Pin history: `86a7674` -> `789d6c8` -> `894fa65` -> `c814c2b` (current; see ADR 0012).

## Local engine dependencies

The optional Windows-first local dependency stack belongs here, not in sibling
`infra`: `local/compose.yaml` defines pinned PostgreSQL and LocalStack (S3
only), both bound to loopback and an internal network. Copy
`local/.env.example` to the ignored `local/.env`, or generate it once with
`powershell -NoProfile -File scripts/init-local-env.ps1`; the initializer
refuses replacement and never prints its generated password. Then run:

```powershell
podman compose --env-file local/.env -f local/compose.yaml up -d
```

`PULSO_RUN_CONTAINER_TESTS=1 python -m unittest tests/test_local_compose_contract.py -v`
only verifies the Compose rendering when a real Podman backend is available.
It does not certify PostgreSQL, S3, Agent Core, AWS or a deployed engine.

For local development telemetry, start the optional Grafana LGTM profile:

```powershell
podman compose --env-file local/.env -f local/compose.yaml --profile observability up -d otel-lgtm
```

Grafana is at `http://127.0.0.1:3000` (`admin` / `admin`, local-only default).
A host-process engine can send OTLP over gRPC to `127.0.0.1:4317` or HTTP to
`127.0.0.1:4318`. An engine container attached to `pulso-internal` should use
the service DNS name `otel-lgtm:4317` or `otel-lgtm:4318` instead. All
host-published ports are loopback-bound; the collector and backends share only
the internal `pulso-internal` network. The profile is not started by the
default stack, has no host data/secret mounts or persistent volume, and is
configured with 24-hour metrics, logs and trace retention arguments by
default. Override the ports and per-signal retention with `PULSO_OTEL_*` values
in `local/.env`; the checked-in duration defaults are validated, but caller
overrides are not runtime-validated and must be a single valid Go duration
token such as `24h` (no spaces or additional flags, because the upstream image
splits extra arguments on whitespace). Malformed overrides may prevent startup.
The image healthcheck covers Grafana, Loki, Tempo,
Prometheus and the collector. This slice configures a backend only: Compose
render/startup and effective retention have not been runtime-verified because
the local Podman VM socket is unavailable, and the engine is not yet wired to
emit telemetry. This development/demo/test stack is not a production
monitoring service and must not receive real customer data.

To run the current snapshot-level local simulation against both the E0
enrichment and original bank CSVs, see [the Windows local snapshot E2E guide](docs/local-e0-e2e-runner.md).
Each source writes to a separate fresh output directory. These runs make no
provider calls; original-bank findings remain descriptive and non-executable.

Before pushing a feature branch, run the same safe Rust, Python and Windows
Pester gates used by GitHub Actions:

```powershell
pwsh -NoProfile -File scripts/verify-local-ci.ps1
```

The PostgreSQL migration suites are deliberately excluded by default because
they reset their target database. To opt in, pass `-IncludePostgres
-AllowDestructiveTestDb -PostgresTestUrl <local-url>`; the script only accepts
loopback hosts and the database name `pulso_test`, keeps the URL out of output,
and sets the repository's destructive-test consent only while those commands
run. Use a disposable database (preferably the same PostgreSQL major version as
CI). `-PlanOnly` prints the selected commands without running them. A local
green run is a pre-PR signal, not a substitute for the required GitHub checks.

Start with [AGENTS.md](AGENTS.md), [CONTEXT.md](CONTEXT.md), the [contract journal](docs/journal/0002-layer-0-contract-envelope.md), the [workspace journal](docs/journal/0003-layer-0-rust-workspace.md), the [source-validation journal](docs/journal/0004-u03-source-contract-validation.md), the [artifact-store journal](docs/journal/0004-u02-immutable-artifact-store.md), the [enriched-history journal](docs/journal/0006-u04-enriched-history.md), the [wiki-scratch journal](docs/journal/0006-u15-wiki-scratch.md) and the [published-memory journal](docs/journal/0010-u33-published-memory.md).

U08/U12 add a local investigation foundation: embedded in-memory SQLite receives
only an approved source representation and runs a bounded `SELECT` AST, never
raw SQL. Sessions bind run, tenant, grant, snapshot and TTL; source writes and
egress are denied. Sealed receipts feed a reproducible descriptive rate with an
explicit denominator and missingness. It neither imports bank data nor opens
filesystem/network; see the [investigation journal](docs/journal/0008-u08-u12-investigation-foundation.md).

U07 adds a framework-neutral, tenant-scoped run-activity API/read-model: bounded
indexed reads, authenticated tenant DTOs, server-side opaque cursors and a
resumable stream-batch port. See the [run-activity journal](docs/journal/0008-u07-run-activity.md).

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

# Pulso improvement engine

Autonomous detection and improvement service integrating external Agent Core primitives.

## Current slice

Layer 0 provides a minimal Rust workspace, a public tracer for the core crate, and its first read-only source-boundary slice. The source validator loads canonical JSON contracts in deterministic filename order and compares adapter-supplied bytes with a sealed synthetic snapshot; it emits deterministic findings for contract, header, file-digest and policy drift without opening bank data itself. U05 adds an in-memory, typed quota/grant semantic boundary: a quota is global to its tenant/resource/window even when the immutable `RunConfig` changes; grants have explicit expiry/revocation and reservations yield deterministic idempotent receipts. It establishes a reproducible local verification command:

```powershell
cargo test --workspace
python -m unittest discover -s tests -p "test_*_contract.py" -v
python contracts/validate_fixtures.py
```

The GitHub Actions workflow additionally checks formatting, Clippy, Rust unit/integration harnesses, contract unit tests and the complete contract-fixture validator on Windows and Linux. U02 adds tenant-scoped immutable artifact repositories and a PostgreSQL migration/adapter; U03 adds synthetic sealed-source validation. U04 adds an in-memory, discovery-safe projection for adapter-supplied E0 enriched-history rows: it seals namespace/world/cutoff plus file/schema/transform/policy digests and availability per field/group, blocks labels/precomputed signals/final outcomes, and blocks exposure entirely when provenance drifts. U15 mounts an exactly-authorized immutable `memory_wiki` revision into an in-process, ephemeral scratch workspace; it can read and apply typed atomic transformations there, but cannot publish, alter the immutable source, open a host path or use the network. It does not implement detection, an Agent Core runtime, a model gateway, authenticated real-data ingestion, or any external call. Infrastructure lives in sibling `infra`.

`rust-ci` owns its ephemeral PostgreSQL service and runs U02's ignored
destructive migration test against it. The test URL and consent only exist in
that isolated CI job; no GitHub secret, external reusable workflow or deployed
infrastructure is involved. This keeps the engine's durable adapter gate
self-contained while sibling `infra` owns Terraform, AWS deployment and
operational infrastructure.

Start with [AGENTS.md](AGENTS.md), [CONTEXT.md](CONTEXT.md), the [contract journal](docs/journal/0002-layer-0-contract-envelope.md), the [workspace journal](docs/journal/0003-layer-0-rust-workspace.md), the [source-validation journal](docs/journal/0004-u03-source-contract-validation.md), the [artifact-store journal](docs/journal/0004-u02-immutable-artifact-store.md), the [enriched-history journal](docs/journal/0006-u04-enriched-history.md) and the [wiki-scratch journal](docs/journal/0006-u15-wiki-scratch.md).

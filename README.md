# Pulso improvement engine

Autonomous detection and improvement service integrating external Agent Core primitives.

## Current slice

Layer 0 provides a minimal Rust workspace, a public tracer for the core crate, and its first read-only source-boundary slice. The source validator loads canonical JSON contracts in deterministic filename order and compares adapter-supplied bytes with a sealed synthetic snapshot; it emits deterministic findings for contract, header, file-digest and policy drift without opening bank data itself. It establishes a reproducible local verification command:

```powershell
cargo test --workspace
python -m unittest discover -s tests -p "test_*_contract.py" -v
python contracts/validate_fixtures.py
```

The GitHub Actions workflow additionally checks formatting, Clippy, Rust unit/integration harnesses, contract unit tests and the complete contract-fixture validator on Windows and Linux. U02 adds tenant-scoped immutable artifact repositories and a PostgreSQL migration/adapter; U03 adds synthetic sealed-source validation. It does not implement detection, an Agent Core runtime, a model gateway, authenticated real-data ingestion, or any external call. Infrastructure lives in sibling `infra`.

`rust-ci` also invokes the reviewed, SHA-pinned reusable PostgreSQL workflow in
`pulso-factored/infra`. That isolated GitHub-hosted database runs U02's ignored
destructive migration test; the engine supplies neither a database URL nor
secrets. Its first green caller run is the evidence that the cross-repository
boundary and real PostgreSQL gate work.

Start with [AGENTS.md](AGENTS.md), [CONTEXT.md](CONTEXT.md), the [contract journal](docs/journal/0002-layer-0-contract-envelope.md), the [workspace journal](docs/journal/0003-layer-0-rust-workspace.md), the [source-validation journal](docs/journal/0004-u03-source-contract-validation.md) and the [artifact-store journal](docs/journal/0004-u02-immutable-artifact-store.md).

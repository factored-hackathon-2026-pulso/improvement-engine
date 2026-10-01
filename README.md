# Pulso improvement engine

Autonomous detection and improvement service integrating external Agent Core primitives.

## Current slice

Layer 0 provides a minimal Rust workspace and a public tracer for the core crate. It establishes a reproducible local verification command:

```powershell
cargo test --workspace
python -m unittest discover -s tests -p "test_*_contract.py" -v
python contracts/validate_fixtures.py
```

The GitHub Actions workflow additionally checks formatting, Clippy, Rust unit/integration harnesses, contract unit tests and the complete contract-fixture validator on Windows and Linux. This does not implement detection, artifacts, an Agent Core runtime, a model gateway, data ingestion, or any external call. Infrastructure lives in sibling `infra`.

Start with [AGENTS.md](AGENTS.md), [CONTEXT.md](CONTEXT.md), the [contract journal](docs/journal/0002-layer-0-contract-envelope.md) and the [workspace journal](docs/journal/0003-layer-0-rust-workspace.md).

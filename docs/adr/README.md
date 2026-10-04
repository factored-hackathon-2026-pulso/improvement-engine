# Engine ADR index

Engine-level decision records (series `0001`-`0005`). The `core-bridge` package keeps its own series, indexed in
[`../../core-bridge/docs/adr/README.md`](../../core-bridge/docs/adr/README.md) (`0001`-`0011`; its next number is `0012`).

| ADR | Purpose | Status |
|---|---|---|
| [0001](0001-service-boundary.md) | Service boundary: detection and autonomous improvement only; Agent Core executes primitives | no status header in the file |
| [0002](0002-source-contract-wire-format.md) | JSON is the executable SourceContract wire format | accepted for Layer 0 |
| [0003](0003-artifact-content-digest.md) | Artifact content digest in U02 (Spanish) | no status header in the file |
| [0004](0004-u07-v2-run-event-identity.md) | Keep durable run activity sequence-native | accepted |
| [0005](0005-u24-v2-sequence-debug-read.md) | U24 reads the durable V2 run sequence | accepted (supersedes part of 0004) |

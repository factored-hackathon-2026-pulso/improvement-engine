# core-bridge ADR index

Decision records of the `core-bridge` package (series `0001`-`0011`). The engine-level ADRs (service boundary, source
contract, artifact digest, run events) are a separate series, indexed in [`../../../docs/adr/README.md`](../../../docs/adr/README.md).
Status is copied from each file's own header; a record that says "proposed" was merged with its PR but its reviewer
approval is not written back into the file.

| ADR | Purpose | Status |
|---|---|---|
| [0001](0001-registry-mock-fastapi.md) | Registry wire mock is a minimal FastAPI process (CAP-52) | accepted |
| [0002](0002-native-evaluate-canonical-digest-and-admission-states.md) | Canonical `native_evaluate` broker digest, in-progress admissions, explicit early close | accepted |
| [0003](0003-writer-mode-and-protected-executor.md) | Writer commitment modes and the protected builder executor | accepted |
| [0004](0004-run-input-slots-via-bind-context-facts.md) | Run inputs reach Flows as `bind_context` facts | accepted |
| [0005](0005-per-route-audiences-purposes-and-jti-replay.md) | Service JWTs with per-route audience and purpose, receiver-owned `jti` replay | accepted |
| [0006](0006-cgroups-disabled-runner-workaround.md) | `--cgroups=disabled` runner for Podman machines without the `pids` controller (local workaround only) | accepted (workaround) |
| [0007](0007-evaluation-context-ref-format.md) | `evaluation_context_ref` format and the derived idempotency key | accepted |
| [0008](0008-agent-core-pin-789d6c8.md) | Agent Core pin bump `86a7674` -> `789d6c8` | proposed (merged) |
| [0009](0009-key-delivery-from-env.md) | Key delivery from env secrets in the image entrypoint | accepted |
| [0010](0010-agent-core-pin-894fa65.md) | Agent Core pin bump `789d6c8` -> `894fa65` (dual-pin rule) | proposed (merged) |
| [0011](0011-annex-d-alignment.md) | Annex D / V3 31.5 alignment of the runtime | accepted |
| [0012](0012-agent-core-pin-c814c2b.md) | Agent Core pin bump `894fa65` -> `c814c2b` (wire identical except MANIFEST sha lines; `synthesise_args` fix) | proposed |

The next number is `0013`. The
current pin is the `PIN_SHA` constant in
[`../../src/pulso_core_runtime/__init__.py`](../../src/pulso_core_runtime/__init__.py); do not duplicate it elsewhere.

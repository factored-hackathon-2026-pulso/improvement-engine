# pulso-bootstrap (CAP-48)

Emits `bootstrap-report/v1` (schema: `schemas/bootstrap-report.v1.schema.json`) listing the seeded assets
(worlds `attention-task`, `attention-demo`, ...) with content digests.

- Offline (no Core): `uv run --python 3.12 --with pyyaml python scripts/core/pulso_bootstrap.py --plan [--out bootstrap-report.json]`.
  Exit 1 on drift between the repo worlds and `agent-core-assets/manifest.yaml` / `expected-state.json`.
- Apply (verifies a seeded Core): `... pulso_bootstrap.py --apply --core-url http://localhost:<port>`; checks every manifest
  release via `GET /v1/registry/releases/{id}`. Seeding itself is the `core-seed` service. Live check (optional):
  bring the stack up with `e2e-core/run.ps1` on the `pulso-dev` machine, then run `--apply`. Unit tests use a fake transport.

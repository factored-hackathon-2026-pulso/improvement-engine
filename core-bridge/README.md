# core-bridge

Pulso runtime wrapper around the pinned agent-core (`pulso_core_runtime`), the wire snapshot and its parity tests.

## Local CI parity

Hosted CI may be unavailable, so `scripts/ci.ps1` reproduces every applicable job locally:

    pwsh core-bridge/scripts/ci.ps1 -Job all -PostgresAdmin postgresql://postgres:<pw>@127.0.0.1:<port>/postgres

`-PostgresAdmin` must point to a throwaway Postgres 16 (for example a Podman container on `pulso-dev`, started with
`--cgroups=disabled`, bound to 127.0.0.1, removed afterwards). Jobs: `rust` (the full `verify` job of
`.github/workflows/ci.yml` through `scripts/verify-local-ci.ps1`), `contract-drift`, `mock-wire`, `a2-wire`,
`real-wire`, `lint` (ruff, optional `-Mypy`), `core-bridge`, `platform-sim`, `agent-core-assets`, `modules-scan`
(no `testing.*` module may be imported when the runtime is composed).

Not reproducible locally and reported as such: the ubuntu leg of `verify`, the `postgres:17@sha256` service
container (pass `-PostgresTestUrl` for a local Postgres 17 `pulso_test` database instead), and `real-wire` without
a running `agent-core serve` (`REGISTRY_BASE_URL`).

## Signer files (`/run/pulso-keys`)

`bridge-identity`, `bridge-staff`, `bridge-callback` (control-api binding callback, A03 class ii) and
`bridge-executor` (executor to lab-broker, A03 class iii; must be a different keypair than the callback key).

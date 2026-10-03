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

## What this package is

`pulso_core_runtime` composes the real pinned agent-core (SHA `86a767474042a566a0dbd6ed23588959f27ebdb3`, contracts
1.3.0) with Pulso's isolated `/internal/v1` API, tool runtime, evaluation runtime and a separate read-only exporter.
Python 3.12 only (`uv --python 3.12`). Contract revision: `pulso-two-teams-1`. It never imports `testing.*` and never
accepts `AGENTCORE_ALLOW_DEMO`.

## Layout

| Path | Content |
|---|---|
| `src/pulso_core_runtime/main.py`, `compat.py`, `factories.py`, `pin.py`, `adapters.py` | composition, closed pin-symbol list, the seven factories, pinned registry port, broker/budget adapters |
| `internal/` | `/internal/v1` sub-app, service-JWT verifier, `pulso_bridge` schema and `jti` table |
| `invoke/`, `credentials/`, `reconcile/`, `store/` | L3a: invoke, receipts CAS, credentials, binding, reconciler |
| `tools/`, `facts/`, `stages/` | L3b: `pulso/*` dispatcher, protected writer, fact whitelist and schemas, stage catalogue |
| `evaluation/`, `harness.py`, `registry_service.py` | L5: admissions, harness, native port, arms |
| `exporter/` | L6: read-only exporter (`python -m pulso_core_runtime.exporter`) |
| `wire/agent_core@86a7674/` | L1a generated wire snapshot (`scripts/gen-wire.ps1 [-Check]`) |
| `scripts/` | `gen-wire.ps1`, `test.ps1`, `ci.ps1`, `build-image.ps1` |
| `tests/` | `wire`, `runtime`, `l3a`, `l3b`, `l5`, `l6`, `integration` (most need real PG16) |
| `Dockerfile`, `docker-entrypoint.sh` | non-root image, entrypoints `runtime|exporter|migrate|agentcore` (and `seed|bootstrap|sweep`, see gaps) |

## Documentation

- ADRs: `docs/adr/0001` FastAPI registry mock, `0002` native-evaluate digest/admission states/early close, `0003` writer
  modes and protected executor, `0004` run inputs as `bind_context` facts, `0005` per-route audiences and `jti` replay,
  `0006` cgroups-disabled runner, `0007` `evaluation_context_ref` format.
- Flows: `docs/flows/core-invoke.md`, `core-receipts-state-machine.md`, `core-reconcile-matrix.md`,
  `core-evaluation-admission-arms.md`, `core-exporter-cursor-cas.md`, `core-binding-context-channel.md`.
- Per-package journals: `docs/journal/claude-0001` (L1) to `claude-0008` (L8).
- Sibling READMEs: `../platform-sim/README.md`, `../agent-core-assets/README.md`, `../local/core/README.md`.

## Running the tests

Tests that need Postgres skip unless `PULSO_TEST_PG_ADMIN` (admin DSN of a throwaway PG16) is set
(`PULSO_REQUIRE_POSTGRES=1` makes them fail instead of skip). Start the throwaway database on Podman machine `pulso-dev`
with `--cgroups=disabled` (ADR 0006), bind it to 127.0.0.1 with a unique name and port, and remove it afterwards:

    python -m pytest -c pyproject.toml tests -p no:cacheprovider        # from core-bridge/, pinned venv

The venv needs `jsonschema` and `referencing` (declared in `pyproject.toml` and `runtime-requirements.txt`); an
environment without them fails to collect `tests/l3b/test_catalog_and_facts.py`, `test_d2_facts_and_ordinals.py` and
`test_review_fixes.py`.

## Runtime configuration (names only, never values)

`AGENTCORE_REGISTRY_DSN`, `AGENTCORE_EVAL_DSN`, `PULSO_SERVICE_KEYS`, `PULSO_IDENTITY_KEYS`, `PULSO_STAFF_KEYS`,
`PULSO_BRIDGE_{IDENTITY,STAFF,CALLBACK,EXECUTOR}_SIGNER`, `PULSO_LAB_BROKER_URL`, `PULSO_CONTROL_API_URL`,
`PULSO_EVAL_BUDGETS`, `PULSO_EVAL_PERMITS`, `PULSO_BRIDGE_MAX_INFLIGHT`, `PULSO_BRIDGE_INSTANCE`, `PULSO_FACTORY_<NAME>`.
Exit code 2 means configuration error (demo double, missing factory, pin drift, unreadable key file, empty URL).

## Known gaps

`docker-entrypoint.sh` names `seed`, `bootstrap` and `sweep` entrypoints that are not modules of this tree;
`mypy --strict` is not clean (agent_core ships no `py.typed`); budgets come from a static file; see the journals for the
per-package list.

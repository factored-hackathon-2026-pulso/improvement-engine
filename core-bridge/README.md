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

`pulso_core_runtime` composes the real pinned agent-core (contracts 1.3.0; the SHA is the `PIN_SHA` constant in
`src/pulso_core_runtime/__init__.py`, the history and rationale are in the ADR series, `docs/adr/README.md`; the wire
snapshot directory name carries the same short SHA) with Pulso's isolated `/internal/v1` API, tool runtime, evaluation runtime and a separate read-only exporter.
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
| `wire/agent_core@<short-sha>/` (currently `agent_core@894fa65/`) | L1a generated wire snapshot (`scripts/gen-wire.ps1 [-Check]`) |
| `scripts/` | `gen-wire.ps1`, `test.ps1`, `ci.ps1`, `build-image.ps1` |
| `tests/` | `wire`, `runtime`, `l3a`, `l3b`, `l5`, `l6`, `integration` (most need real PG16) |
| `Dockerfile`, `docker-entrypoint.sh` | non-root image, entrypoints exactly `runtime|exporter|migrate|agentcore` (asserted by `tests/runtime/test_image.py`) |

## Documentation

- ADRs: `docs/adr/0001` FastAPI registry mock, `0002` native-evaluate digest/admission states/early close, `0003` writer
  modes and protected executor, `0004` run inputs as `bind_context` facts, `0005` per-route audiences and `jti` replay,
  `0006` cgroups-disabled runner, `0007` `evaluation_context_ref` format, `0008` agent-core pin bump to `789d6c8`,
  `0009` key delivery from env, `0010` pin bump to `894fa65` (dual-pin rule), `0011` Annex D alignment. The index with
  one line of purpose and status per ADR is `docs/adr/README.md`; the next ADR number is `0012`.
- Flows: `docs/flows/core-invoke.md`, `core-receipts-state-machine.md`, `core-reconcile-matrix.md`,
  `core-evaluation-admission-arms.md`, `core-exporter-cursor-cas.md`, `core-binding-context-channel.md`.
- Per-package journals: `docs/journal/claude-0001` (L1) to `claude-0011` (CI parity) and, newer, `docs/journals/claude-0012`
  to `claude-0021` (dry-run review, pin bump `894fa65`, platform-sim, assets, stack, CI parity, Annex D alignment).
- The `/internal/v1` contract that the Rust client codes against, including the dry-run and alias read routes, is
  published in `../bridge-contract/README.md`; the route table lives in `internal/app.py::ROUTES`.
- Sibling READMEs: `../bridge-contract/README.md`, `../platform-sim/README.md`, `../agent-core-assets/README.md`, `../local/core/README.md`.

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
Added by the `789d6c8` bump (ADR 0008) and still current: `PULSO_CORE_SHA` (build sha reported by Core's `/version`, set by the image),
`PULSO_KEYS_RELOAD_SECONDS` (default 5, 0 = off), `PULSO_CORE_EXPORT_ENABLED` (default off: Core's `/v1/export/*` is not
mounted; the PG exporter is the ingest path). Pass-through, untouched and off by default: `AGENTCORE_LLM_GATEWAY_URL` +
`AGENTCORE_LLM_GATEWAY_TOKEN` (both or neither; neither = generation falls back to templates), `AGENTCORE_DB_POOL_MAX`,
the S3 blob bucket and the SNS publisher.
LLM gateway config (`pulso_core_runtime/llm/config.py`): `AGENTCORE_LLM_GATEWAY_URL`, `AGENTCORE_LLM_GATEWAY_TOKEN`, `PULSO_LLM_MODE` (`gateway|disabled`),
`PULSO_LLM_STAGE_POLICY`, `PULSO_LLM_STAGE_POLICY_JSON`, `PULSO_LLM_POLICY_REQUIRED`.
Exit code 2 means configuration error (demo double, missing factory, pin drift, unreadable key file, empty URL).

## Key delivery from env (AWS / Fargate; ADR 0009)

`docker-entrypoint.sh` materialises key files from env secrets into tmpfs (`$PULSO_KEYS_DIR`, default
`/run/pulso-keys`, files 0400 owned by uid 10001; the directory must be writable: Fargate ephemeral/tmpfs volume,
Podman `--read-only` gives `/run` as tmpfs), validates shape and length, exports the matching path variable
(`PULSO_IDENTITY_KEYS`, `PULSO_BRIDGE_*_SIGNER`, ...), then `unset`s every secret variable before `exec`. A missing or
malformed required variable exits 2 with `pulso:runtime_config_invalid: <VAR> ...` (variable names only, never values).
The runtime also refuses to start when the executor signer key equals the callback signer key. File mode (local/core
mounts the files and sets the path variables, or uses the default names under `/run/pulso-keys`) keeps working: a
missing env variable is accepted when the corresponding key file is already present and valid. `AGENTCORE_ALLOW_DEMO`
skips this step so the runtime's own demo-double refusal still fires.

| Entrypoint | Required secret env var (content) | Materialised file | Shape |
| --- | --- | --- | --- |
| `runtime` | `PULSO_BRIDGE_IDENTITY_SIGNER_JSON` | `bridge-identity.json` | `{"kid", "key": b64url 32-byte Ed25519 seed}` |
| `runtime` | `PULSO_BRIDGE_STAFF_SIGNER_JSON` | `bridge-staff.json` | same |
| `runtime` | `PULSO_BRIDGE_CALLBACK_SIGNER_JSON` | `bridge-callback.json` | same |
| `runtime` | `PULSO_BRIDGE_EXECUTOR_SIGNER_JSON` | `bridge-executor.json` | same; key must differ from callback |
| `runtime` | `CORE_IDENTITY_KEYS_JSON` | `identity.json` | JSON object (agent-core identity keys) |
| `runtime` | `CORE_STAFF_KEYS_JSON` | `staff.json` | JSON object (agent-core staff keys) |
| `runtime` | `PULSO_SERVICE_KEYS_JSON` | `service.json` | `{"keys": {kid: {"iss", "aud", "key": b64url 32 bytes}}}` |
| `exporter` | `PULSO_EXPORTER_KEY_CONTROL_API_SEED` | `exporter-control-api.key` | b64url 32-byte Ed25519 seed |
| `exporter` | `PULSO_EXPORTER_KEY_LAB_BROKER_SEED` | `exporter-lab-broker.key` | same |
| `migrate`, `agentcore` | none | none | |

Non-secret configuration (DSNs without passwords aside, URLs, tenant, ids) stays as plain env; DSNs carrying
passwords are secrets injected by the platform as ordinary env vars and are not touched by the entrypoint.

## Known gaps

`mypy --strict` is not clean (agent_core ships no `py.typed`); budgets come from a static file; see the journals for the
per-package list.

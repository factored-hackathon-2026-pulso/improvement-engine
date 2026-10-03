# Journal claude-0002: L2 composed runtime, image and internal API

Contract revision: `pulso-two-teams-1`. Pin `86a767474042a566a0dbd6ed23588959f27ebdb3` (contracts 1.3.0).
Package: L2 (plan 17.3.2) plus the integrator composition. Code: `src/pulso_core_runtime/{main,compat,factories,pin,ids,
readiness,adapters,errors}.py`, `internal/{app,auth,store}.py`, `Dockerfile`, `docker-entrypoint.sh`. Commits: L2 slice,
5a09323 (integrator), b3af773 (A03), 4984d92 (final review). Documentation pass at HEAD `4984d92`.

## Purpose
One process that runs the real pinned Core (`resolve_ports`, `build_api_deps`, `create_app`) with Pulso's seven factories
and mounts an isolated `/internal/v1` sub-application, with fail-closed startup.

## Flow (`main._compose`)
`preflight` (reject `AGENTCORE_ALLOW_DEMO`, `assert_compat` on the closed pin symbol list) -> `factory_paths` (seven
factories, `testing.*` rejected) -> observability setup -> key files and URLs checked -> bridge schemas and migrations
(advisory-locked) -> L3 (`build_l3`) -> tool runtime before `resolve_ports` (late-bound builder factory through a `_Lazy`
holder) -> `resolve_ports` -> `PinnedRegistryPort` -> evaluation runtime (UNGUARDED gateway/providers, own budget meter)
-> arm runner -> live gateway/providers wrapped by the binding guards and spend metering -> `build_api_deps` ->
`/internal/v1` mounted via an API extension -> readiness checks `eval_db`, `bridge_schema`, `key_files`, `factories_ok`
-> `uvicorn`. Exit 0 normal, 2 configuration (`pulso:runtime_config_invalid`, demo double, adapter missing, pin drift).

## Input / output
Env: `AGENTCORE_REGISTRY_DSN`, `AGENTCORE_EVAL_DSN` (must be isolated: `assert_isolated`), `PULSO_SERVICE_KEYS`,
`PULSO_IDENTITY_KEYS`, `PULSO_STAFF_KEYS`, `PULSO_BRIDGE_{IDENTITY,STAFF,CALLBACK,EXECUTOR}_SIGNER`, `PULSO_LAB_BROKER_URL`,
`PULSO_CONTROL_API_URL`, `PULSO_EVAL_BUDGETS`, `PULSO_EVAL_PERMITS`, `PULSO_BRIDGE_MAX_INFLIGHT`, `PULSO_BRIDGE_INSTANCE`,
`PULSO_FACTORY_<NAME>`, `PULSO_HOST/PORT`. Out: Core API plus `/internal/v1/*` (invoke, read, aliases, dry-run, version,
credentials, evaluation); `GET /version` reports `agent_core_sha`, `contracts_version`, `pulso_sha`, `image_digest`,
`runtime_profile=agent_core_real` and `doubles[]` computed from the real configuration.

## Transactions and idempotency
Schema creation is idempotent (`CREATE ... IF NOT EXISTS`, advisory lock 7301, migration names recorded in
`pulso_bridge.migrations`, separate from L2's `schema_version`). Service-JWT replay table `pulso_bridge.jti_seen`.

## Errors
Typed config errors with exit 2; internal envelope `{schema_version, code, retryable, trace_id, details}` (Core's
`problem+json` handlers never apply under `/internal/v1`); unhandled exception is `pulso:internal_error` 500 retryable;
unimplemented route is 501 `pulso:not_implemented`.

## Permissions
ADR 0005 (per-route audience/purpose, `jti`, tenant claim). `PulsoAuthz` admits only builder principals carrying
`constructor`, `aprobador` or `stage_task`; no subject/OBO; closed reportable attrs `stage`, `pin_release_id`.
`PinnedRegistryPort` honours the signed `pin_release_id` (active, belongs to the selected agent/version).

## Config
Entry points of `docker-entrypoint.sh`: `runtime | exporter | migrate | agentcore` (seed/bootstrap/sweep were removed: no such modules; seeding is the local/core init job); key JSON from
`CORE_IDENTITY_KEYS_JSON`, `CORE_STAFF_KEYS_JSON`, `PULSO_SERVICE_KEYS_JSON` is written to tmpfs and unset. Image: multi-stage
(wheelhouse from the pinned checkout, contracts/VERSION must equal 1.3.0), non-root uid 10001, only
`src/pulso_core_runtime` copied (no `testing`).

## Observability
`/readyz` reports check names only (503 on any failure); `/version` doubles list; the `testing.*` scan test asserts no
`testing` module is imported after composing (`ci.ps1 -Job modules-scan`).

## Commands (head `4984d92`)
- `pwsh core-bridge/scripts/ci.ps1 -Job core-bridge -PostgresAdmin <admin DSN>` (full suite on PG16)
- `python -m pytest -c pyproject.toml tests/runtime tests/integration -p no:cacheprovider` with `PULSO_TEST_PG_ADMIN` set
- `pwsh core-bridge/scripts/build-image.ps1` (verifies the pin checkout HEAD before building)
Environment: Windows 11, Python 3.12 (uv), pinned venv `%TEMP%\pulso-wire-venv-86a7674`, throwaway PG16 on Podman
`pulso-dev` with `--cgroups=disabled` (ADR 0006).

## RED / GREEN
Reported in BITACORA: first slice 42 tests passed on real PG16; Dockerfile multi-stage build verified on `pulso-dev`
(no `testing`, non-root, `agentcore --help` works); integrator wiring later made `tests/integration` (boot, scout, writer,
arms, kill -9 reconcile) green on PG16; combined L5 suite at review r2 reported 240 passed. First RED of the slice is not
named in the entry. Not re-run here.

## Trade-offs
Late-bound composition (`_Lazy` holder) breaks the cycle between tools, `resolve_ports` and the registry at the cost of a
runtime error if a piece is used before composition. The exporter is a separate entrypoint, not part of the API process.

## Gaps
- `mypy --strict` is not clean (agent_core ships no `py.typed`).
- Transcript is a null stand-in (conversational runs rejected) and `grant-active` is always false; both are listed in
  `/version.doubles[]`. Calibration and classifier asset directories are optional and reported when absent.
- No online key revocation (ADR 0005).

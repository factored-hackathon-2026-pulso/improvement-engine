# ADR 0005: Service JWTs with per-route audience and purpose, receiver-owned `jti` replay

Status: accepted (L2/L3a/L6 and the A02/A03 agreements with Codex). Contract revision: `pulso-two-teams-1`.

## Context
The bridge exposes `/internal/v1/*` to Pulso services and calls the control-api and the lab-broker. A single shared
token would let any holder call any route and replay it. Core's own `principal+jws` credentials are a different
mechanism and must never be accepted on these routes (and vice versa).

## Decision
**Inbound (`internal/auth.py::ServiceJwtVerifier`, routes in `internal/app.py::ROUTES`)**
- `typ=JWT`, verifier-fixed EdDSA, header is exactly `{alg, kid, typ}`; `kid` resolves to a key bound to one
  `(iss, aud)` pair (`key_binding` on mismatch).
- Each route declares an audience (`core-bridge`) and a closed set of `purpose` values: `core_task_invoke`,
  `core_task_read`, `alias_read`, `authoring_dry_run`, `version_probe`, `credential_issue`, `evaluation_admit`,
  `evaluation_arm_run`, `evaluation_arm_read`. A wrong purpose is 403 `pulso:auth_denied`.
- `tenant_id` claim is required on every route except `version_probe`; handlers compare it with the body tenant
  (`pulso:tenant_mismatch`, 403). Finite `iat`/`exp` only, `exp - iat <= 300 s`, `exp - now <= 330 s`.
- Replay: `(iss, jti)` is consumed last (a rejected token never burns its `jti`) in `pulso_bridge.jti_seen`
  (`PgJtiStore`: atomic `INSERT ... ON CONFLICT DO NOTHING`, expired rows swept on write); a replay is
  `pulso:auth_invalid` with reason `jti_replayed` (401). The table is receiver-owned and shared across replicas.
- Bodies are capped at 1 MiB before the handler reads them (413).

**Outbound (one fresh token, with a new `jti`, for every HTTP attempt)**
- control-api binding callback: `aud=control-api`, `scope=binding`, `purpose=core_task_binding`, signed by the
  callback key (A03 class ii).
- lab-broker tools and arm clients: `aud=lab-broker`, a singular `scope` (for example `artifact_read`, `sandbox`),
  `purpose`, `tenant_id`, optional `job_id` and `binding_ref`; signed by the separate **executor** key (class iii).
  Boot fails if the executor key equals the callback key (same seed or same `kid`).
- exporter: `ExporterTokenSigner` mints per route class: observations and cursor to `control-api` (scope
  `observations`), artifacts to `lab-broker` (scope `artifact_write`, purpose `artifact_upload`); one key and `kid`
  per audience, TTL <= 5 min (default 60 s).

## Consequences
Key files are loaded from `/run/pulso-keys` (file names only in errors, never values). A compromised callback key cannot
call the broker, and vice versa. Rotation is by `kid` in the service key file; there is no online revocation list (gap).
The `jti` table grows with traffic until `exp` passes (swept opportunistically on each consume).

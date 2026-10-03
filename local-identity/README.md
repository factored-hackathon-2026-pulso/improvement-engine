# local-identity (sandbox-only human issuer)

Local-only backend service (plan 16.13.2 / 17.3.2, DR-08). It lets the local profile run the demo steps
"human authority only when needed; approval fixes operation/hash and is not publication" without any real SSO.
It is **never** deployed to staging/production, never proxied to the browser, and holds no Pulso state.

## Routes (control-api -> issuer, internal port 8083)

Auth: Pulso service JWT (`typ=JWT`, EdDSA, `kid` bound to `iss=control-api`/`aud=human-issuer`), claims
`iss,aud,sub,tenant_id,purpose,iat,exp,jti`, `exp-iat<=60`, skew 60 s (reject when `now>=exp+skew` or
`iat>now+skew`), fresh `jti` per HTTP attempt, receiver-owned replay table (SQLite file, retained through
`exp+skew`). A Core `principal+jws` is never a valid bearer (`bad_header`).

| Route | purpose | Request (unknown fields rejected) | Result |
|---|---|---|---|
| `POST /internal/v1/human/session-assertions/issue` | `session_assertion` | `{tenant_id, actor_ref, session_intent_ref, nonce}` | `{assertion, kid, exp}`; JWT `iss=human-issuer, aud=control-api, sub, tenant_id, nonce, session_intent_ref, iat, exp(<=60 s), jti, auth_level=session, auth_at` |
| `POST /internal/v1/human/command-authorizations/issue` | `command_authorization` | `{tenant_id, actor_ref, command_ref, operation, target, challenge_ref, nonce}` | `{authorization_jws, kid, exp, metadata:{auth_simulated:true, operation, binding_digest}}` |

`operation` -> target kind -> required role: `approve|reject|publish` -> `proposal{proposal_id,candidate_hash(64 hex),expected_revision}`
-> `aprobador`; `promote` -> `release{release_id,agent_id,alias,expected_revision}` -> `aprobador`; `revoke` -> `release` -> `admin`.
`target.kind` is optional (`"proposal"`/`"release"`); a release never carries `candidate_hash`.

`authorization_jws` is the exact Core human Principal JWS: header `{alg:EdDSA,kid:local-sim-human-*,typ:principal+jws}`,
payload `{type:builder,id:<actor_ref>,roles,scopes:[],attrs,auth:{level:step_up,at,simulated:true},exp:now+60s}`.
Roles are least-authority per operation (`constructor`+`aprobador` the actor holds; `admin` only for `revoke`).
`attrs` (strings only): `actor=human, tenant, issuer=local-identity, command_ref, operation, challenge_ref, target_kind,
binding_digest` plus the target fields. `binding_digest = sha256(JCS-like sorted compact JSON of {tenant_id, actor_ref,
command_ref, operation, challenge_ref, target})` (nonce excluded). Core ignores these attrs (it checks role, `auth.level`
and `exp` only), so the port must call `local_identity.client.assert_bound(...)` against its durable intention before
dispatch; that is what stops replaying an approval for another operation/hash/revision. The signature is verified by Core.

Errors use the D.1 envelope `{schema_version, code, retryable, trace_id, details}`:
`pulso:auth_invalid` 401 (`details.reason`: malformed, bad_header, unknown_kid, bad_signature, key_binding, wrong_audience,
missing_claims, expired, not_yet_valid, ttl_too_long, missing_token), `pulso:auth_denied` 403 (`purpose_denied`,
`tenant_required`), `pulso:service_token_replayed` 401, `pulso:tenant_mismatch` 403 (claim != body, or identity of another
tenant), `pulso:actor_not_allowed` 403, `pulso:role_not_allowed` 403, `pulso:nonce_replayed` 409,
`pulso:invalid_request` 422 (`details.fields`), `pulso:payload_too_large` 413, `pulso:internal_error` 500.
`/healthz`, `/readyz` unauthenticated. No `/docs`/`openapi.json`.

## Local-only guards (exit 2 `local_identity:*`)

`profile_not_local` (`LOCAL_IDENTITY_PROFILE` must be exactly `local`), `remote_environment` (AWS/ECS/Lambda/Kubernetes/Cloud Run/Azure markers, or `PULSO_ENV` not in the allowlist
`''/local/dev/development/test`), `kid_not_local` (human kid must start `local-sim-human-`, session kid `local-sim-session-`),
`key_reuse` (human staff key must differ from session/service keys and from `LOCAL_IDENTITY_BOT_PUBLIC_KEYS`),
`service_keys_invalid` (every key bound to `aud=human-issuer`), `identities_invalid` (bot-shaped ids, unknown roles,
`actor!=human` refused), `config_invalid`. Actor/role comes from the server allowlist (`identities.json`), never from the
request. `local_identity.contract.scan_remote_config(root)` is the CAP-63 contract test: it fails when a `local-sim-*` kid
or `auth.simulated=true` appears in remote configuration (terraform/infra/deploy/*staging*/*prod*/`*.tfvars` ...).

## Keys and environment

`python -m local_identity.keys gen local/.secrets/human-issuer [tenant]` writes (ignored dir, 0600 best effort):
`control-api-signer.json` (control-api side), `human-issuer-service-keys.json`, `session-signer.json`,
`control-api-session-verifier-keys.json`, `human-staff-signer.json`, `core-staff-keys.human-fragment.json`
(`{"principal_keys": {kid: pub}}`: **add the human kid to Core's `staff.json` public set**, public only),
`identities.json`. Env: `LOCAL_IDENTITY_PROFILE=local`, `_SERVICE_KEYS`, `_SESSION_SIGNER`, `_HUMAN_STAFF_SIGNER`,
`_IDENTITIES`, optional `_REPLAY_DB`, `_BOT_PUBLIC_KEYS`, `_CLOCK_SKEW_S` (<=60), `_STEP_UP_TTL_S` (<=120),
`_SESSION_TTL_S` (<=60), `_PORT` (8083), `_HOST` (default 127.0.0.1; the image sets 0.0.0.0, and does NOT bake `LOCAL_IDENTITY_PROFILE`: compose must set it).

## Client (stand-in / adapter)

```python
from local_identity.client import LocalIdentityClient, ServiceSigner, assert_bound

client = LocalIdentityClient("http://human-issuer:8083", ServiceSigner.from_file(path), tenant_id="t1")
auth = client.command_authorization(
    tenant_id="t1",
    actor_ref="local-supervisor",
    command_ref="cmd-1",
    operation="approve",
    target={"proposal_id": "p1", "candidate_hash": "<hex64>", "expected_revision": 3},
    challenge_ref="chal-1",
    nonce=uuid4().hex,
)  # -> CommandAuthorization(authorization_jws, kid, exp, metadata)
assert_bound(
    auth.authorization_jws,
    tenant_id="t1",
    actor_ref="local-supervisor",
    command_ref="cmd-1",
    operation="approve",
    target={...},
    challenge_ref="chal-1",
)
```

Each attempt mints a fresh service token; a new attempt needs a new nonce. Never log/persist `authorization_jws` or
`assertion`, never hand them to the browser.

## Image and compose fragment (do not edit `local/core`; the integrator copies this)

`podman build -t pulso-local-identity:<sha7> local-identity` (python 3.12-slim, uid 10002, `uv.lock` pinned). Fragment
(`compose.fragment.yaml`) adds service/DNS alias `human-issuer`, port 8083 on the internal network only, read-only rootfs,
secrets mounted read-only from the ignored secret dir, replay DB on a named volume, and no host port.

## Tests

`uv sync --python 3.12 && uv run pytest` (unit: 41). The real-Core proof (`tests/proof`) needs the pinned checkout
importable and a Postgres 16 admin DSN: run with the pinned venv, `PULSO_TEST_PG_ADMIN=postgresql://...` (skipped otherwise):
bot cannot approve (`forbidden_role`), human session-level JWS gets `step_up_required`, issued JWS -> approve -> publish
(staging moves, prod does not) -> promote (prod moves), admin-only revoke, nonce/jti/expiry/hash replays rejected.

## HumanAuthorizationPort contract (where binding enforcement MUST live)

Core checks only signature, `auth.level=step_up`, role and `exp`; it ignores the binding attrs and the Principal has no
`jti`. Therefore the Codex port, not Core, must: (1) atomically consume its durable intention (single use; the JWS is
replayable for its lifetime otherwise), (2) call `assert_bound(jws, ..., now=<clock>)` with values read from that
intention (never from the request), (3) send the same JWS bytes to Core. `assert_bound` recomputes the digest, requires
every signed attr/actor/step-up/simulated/required role (`admin` iff `revoke`) and `exp` to agree. Contract tests:
`tests/test_review_hardening.py`. The Core staff public-key fragment must be merged only into the local Core's staff set.
The replay DB defaults to `:memory:` when `_REPLAY_DB` is unset (not durable across restarts): set it outside tests.

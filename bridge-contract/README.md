# bridge-contract: the published `/internal/v1` contract of the Core bridge

A machine-checkable contract of the `pulso-core-runtime` HTTP surface that the Rust engine (Codex) calls, plus a
conformance suite that can be pointed at ANY implementation: our real runtime, the `platform-sim` mock, or a Rust test
double.

| Item | Location | Produced by |
|---|---|---|
| OpenAPI 3.1 | `openapi/bridge-internal-v1.yaml` | `gen.py` (from `core-bridge/src`) |
| JSON Schema 2020-12 (28 files) | `schemas/*.schema.json` | `gen.py`: pydantic models + catalogue + constants |
| Closed lists, auth table, limits, state machine, idempotency rules | `contract.json` | `gen.py` |
| Golden request/response flows | `examples/flows/*.json` | recorded from the REAL composed runtime |
| Conformance suite | `conformance/` | hand-written, schema-driven |
| Mock-vs-contract divergence report | `divergences/mock-vs-contract.json` | `divergence.py` |

`contract_revision = pulso-two-teams-1`. Our pin: agent-core `894fa65575d83420523f33ec1c6919b8965f7ebe`, contracts
`1.3.0` (the constants of `pulso_core_runtime`; `GET /version` must report exactly these). Adding replies or journal
entries does not bump the wire; a DTO, code, limit or auth change does (see "Change policy").

## How Codex uses it

1. **Build the Rust client from `openapi/` + `schemas/`.** Every request DTO is closed (`additionalProperties: false`;
   unknown fields are a 422). Every response is validated by the schema named in the OpenAPI response. Decimal money is a
   string; JSON numbers are integers only (the bridge refuses non-integer numbers in facts).
2. **Treat `contract.json` as the source for constants**: auth claims/TTL, purposes per route, the closed error-code list
   (`error_codes.wire`), limits, the receipt state machine, the idempotency formulas.
3. **Run the conformance suite against your client harness or test double** (below). A double that passes the suite
   with `CONTRACT_TARGET=<label>` is wire-compatible for everything the suite covers; skipped cases are listed with the
   missing capability, never silently green.
4. **Use `examples/flows/*.json` as fixtures.** Each step has the request (JWT claim *profile*, never a token), the
   expected status, the schema and the normalised body. Volatile ids are placeholders (`<core_run_id#1>`, `<exp>`...).

## Running the suite

```text
pip install -r bridge-contract/requirements.txt
```

Target selection is by environment variable NAMES (values are never printed or stored):

| Variable | Meaning |
|---|---|
| `CONTRACT_TARGET` | label: `real` (default), `mock`, or any label for an external target |
| `CONTRACT_BASE_URL` | `http://host:port` of the target; unset + `real` starts the in-process runtime, unset + `mock` spawns the mock |
| `CONTRACT_SERVICE_KEY_FILE` | JSON `{"kid": "...", "seed": "<b64url 32-byte Ed25519 seed>"}`: the control-api key the target trusts |
| `CONTRACT_SERVICE_KID`, `CONTRACT_SERVICE_ISS`, `CONTRACT_SERVICE_AUD` | override kid / `iss` (default `control-api`) / `aud` (default `core-bridge`) |
| `CONTRACT_TENANT`, `CONTRACT_OTHER_TENANT`, `CONTRACT_UNKNOWN_TENANT` | two deployed tenants and one that is NOT in the deployment set (`t1`, `t2`, `t-not-deployed`) |
| `CONTRACT_WORLD_FILE` | optional JSON `{scout_release, writer_release, agent_version, caps: [...]}` seeding refs for flow tests |
| `PULSO_TEST_PG_ADMIN` | only for the in-process real target: admin DSN of a throwaway PG16 |
| `CONTRACT_RECORD=1` | re-record the goldens (real target only) |

```text
# (a) our real runtime: composed in-process, served over HTTP on 127.0.0.1, throwaway PG16
PULSO_TEST_PG_ADMIN=postgresql://postgres:<pw>@127.0.0.1:<port>/postgres  python -m pytest bridge-contract
# (b) the platform-sim mock (spawned)
CONTRACT_TARGET=mock python -m pytest bridge-contract/conformance
# (c) your Rust double / harness
CONTRACT_TARGET=rust-double CONTRACT_BASE_URL=http://127.0.0.1:9000 CONTRACT_SERVICE_KEY_FILE=<file> \
  python -m pytest bridge-contract/conformance
```

The suite imports nothing from `core-bridge` (the kit in `conformance/kit.py` signs JWTs with `cryptography` and
validates with `jsonschema`); only the in-process real world (`worlds/real.py`) and `gen.py` need the pinned agent-core.

Capabilities gate the cases that need target-specific seeding (`needs(...)`): `invoke` (a seeded scout release),
`writer` (a frozen candidate), `evaluation`, `arms` (a sealed scenario manifest), `control` (fault hooks),
`credentials`, `authoring`. An external target declares them in `CONTRACT_WORLD_FILE`; without a capability the case is
skipped with its name. Black-box cases (auth, envelope, validation, limits, version, credentials) need nothing.

### Known-different cases (mock)

`conformance/known_different.py` lists, per target, test name -> divergence id + justification. They run as
`xfail(strict=True)`: they must still fail against that target, and the moment the target converges the suite goes red
so the table shrinks. `real` has no entries. Regenerate the data report with `python bridge-contract/divergence.py`.

## Drift gate (src -> contract)

`python bridge-contract/gen.py --check` (also `tests/test_gen_drift.py`) fails when a checked-in artifact differs from
what the source derives. It also fails when a `pulso:*` literal appears in `core-bridge/src` that is neither classified
as a wire code nor as non-wire in `gen.py`, so a new error code cannot ship without a contract decision. Run
`python bridge-contract/gen.py` after a source change and review the diff. `divergences/` and the goldens have their own
checks (`tests/test_mock_divergence.py`, `conformance/test_golden_flows.py`).

## Versioning and change policy

* `contract_revision` names the agreed wire (`pulso-two-teams-1`). The OpenAPI `info.version` carries it.
* **Compatible (no bump):** new optional response fields inside documented `additionalProperties: true` objects, new
  examples, clarified docs, new routes.
* **Breaking (needs a journal entry accepted by both teams and a new revision):** a new required request field, a
  removed/renamed field, a new status for an existing code, a changed limit/TTL/purpose/claim, a new member of a closed
  list that the client must handle (a new error code, a new receipt state), a changed idempotency formula.
* Alias read and authoring dry-run are implemented, reviewed and frozen like every other route (the former
  `x-status: pending-implementation` markers are gone; the never-emitted `pulso:compile_violation` code was dropped:
  violations travel inside the HTTP 200 body).
* Our pin changes only with the agent-core pin bump; `GET /version` is the runtime check, `contract.json.pin` the
  expected value.

## What Rust must implement, per route

All routes: `Authorization: Bearer <service JWT>`: compact JWS, header exactly `{alg: EdDSA, kid, typ: JWT}`,
payload `iss=control-api, aud=core-bridge, sub=worker:<id>, tenant_id, purpose, job_id, iat, exp, jti`, no `scope`;
`exp - iat <= 300 s`; a FRESH `jti` on every HTTP attempt (retries keep the business key, not the token); optional
`traceparent` (error `trace_id` echoes it). Errors are always `ErrorEnvelope` (never `problem+json`).

| Route | Purpose claim | Rust sends | Rust must handle |
|---|---|---|---|
| `POST /core-tasks/invoke` | `core_task_invoke` | `CoreTaskInvocation`; header `Idempotency-Key = sha256_hex(tenant\|job\|stage\|attempt\|logical_key)`; optional `request_digest = sha256_hex(JCS(body minus request_digest, credentials, trace))`; writer stage carries `registry_mutation_commitment` | `200` terminal (`terminal_ok`/`terminal_failed`), `202` non-terminal (poll `GET core-tasks/{core_run_id}` or re-send the SAME request: the bridge reconciles, never re-executes); `unknown` is never licence to repeat the effect; `429 bridge_busy` and `503 core_unavailable` are retryable with the same key; `409 digest_conflict`; `409 release_*`; `413 input_too_large`; `422 stage_agent_mismatch`; `400 unknown_input_slot`. `binding_ref = sha256_hex(tenant\|key)` can be computed before dispatch |
| `GET /core-tasks/{id}` | `core_task_read` | the `core_run_id` | `200` `CoreTaskReceipt` (`code` `task_unknown`/`task_in_progress` for non-terminal), `404 not_found` (also for another tenant's run); facts only through the whitelist |
| `POST /core-credentials/issue` | `credential_issue` | `{tenant_id, role, purpose}` (NO `schema_version`) | `{jws, kid, exp: int}` with `Cache-Control: no-store`; never persist or log `jws`; only the `credential_policy` rows are issuable; `403 credential_not_issuable`, `503 credential_signing_unavailable` (retryable) |
| `POST /evaluation/admissions` | `evaluation_admit` (+ `job_id` claim required) | `EvaluationAdmissionRequest` incl. a client-chosen `evaluation_context_ref` (the idempotency key) | `201` created, `200` identical replay, `409 idempotency_conflict`, `409 candidate_changed\|suite_mismatch\|admission_expired`, `403 broker_denied\|budget_unknown`, `404 proposal_not_found`, `422 evaluation_context_invalid` |
| `POST /evaluation/arms/run` (also `/{arm_id}/run`) | `evaluation_arm_run` | `ArmRequest` (closed: no oracle/gold fields); the key is `body.idempotency_key` | `200` `ArmReport` (a failure INSIDE the run is `status=failed_infra`/`candidate_failed`/`unknown` with `reason`, not an HTTP error); `409 idempotency_conflict\|mixed_world_rejected\|sandbox_required\|supersedes_invalid`; after a timeout read back, never re-run |
| `GET /evaluation/arms/{execution_id}` and `/by-key/{key}` | `evaluation_arm_read` | `execution_id = arm-sha256_hex(tenant\|key)[:32]` | `200` the stored report (an interrupted run reads back `unknown`), `404` |
| `GET /version` | `version_probe` (no tenant claim needed) | nothing | `CoreVersion`; compare `agent_core_sha`/`contracts_version` with the pin; `doubles` must be empty in a real deployment |
| `GET /core-state/aliases/{agent_id}/{alias}` | `alias_read` | alias in `staging\|prod` | `AliasState`, `404 alias_unknown`, `422` |
| `POST /core-authoring/dry-run` | `authoring_dry_run` | `CoreAuthoringDryRunRequest` (tenant in body == claim) | `200` with non-empty `violations` is NOT success (`candidate_hash` null); `release_id_preview = "rel-" + candidate_hash[:16]` |

Receipt state machine (`contract.json.receipt_state_machine`): `prepared -> sent -> binding_confirmed ->
terminal_ok | terminal_failed`, with `unknown` and `manual_reconcile` reachable after `sent`; terminal states have no
outgoing edge. A timeout after `sent` is `unknown`. A failed writer is `manual_reconcile`, never provably
`terminal_failed`.

Exporter side (Rust is the RECEIVER): `ObservationEvent` / `ObservationBatch` (`pulso-observations-2`, `source_*` fields,
`batch_digest = sha256(JCS(batch without it))`, ACK only after commit) and `AuditAgentStepFailedEvidence`: ONLY a
correlated audit `agent_step` with `kind=failed` and `error_kind` classifies a model-dependency failure (CX-0073/0075);
the task receipt keeps its own outcome.

## Layout

```text
gen.py  divergence.py  contract.json  openapi/  schemas/  examples/flows/  divergences/
conformance/   kit.py (signer, JCS, schemas, client)  conftest.py  known_different.py  flows.py (golden flows)
               test_auth.py test_validation.py test_invoke.py test_evaluation.py test_authoring.py test_golden_flows.py
               worlds/ real.py (in-process runtime + PG16)  mock.py  external.py
tests/         test_gen_drift.py  test_contract_static.py  test_mock_divergence.py
```

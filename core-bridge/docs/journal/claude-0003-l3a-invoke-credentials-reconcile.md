# Journal claude-0003: L3a invoke, credentials, binding service, reconcile, store

Contract revision: `pulso-two-teams-1`. Pin `86a767474042a566a0dbd6ed23588959f27ebdb3`. Package: L3a (plan 17.3.3).
Code: `src/pulso_core_runtime/{invoke,credentials,reconcile,store}/`, `adapters.py`. Commits: 8925163, a7d701d, 8d620da,
1b21a16, dbe7f45 (adopted writes verified against the commitment), 4984d92. Documentation pass at HEAD `4984d92`.

## Purpose
`POST core-tasks/invoke` and `GET core-tasks/{id}`, `core-credentials/issue`, the receipt state machine, the binding
service and the reconciler. Flows: `docs/flows/core-invoke.md`, `core-receipts-state-machine.md`,
`core-reconcile-matrix.md`, `core-binding-context-channel.md`.

## Flow
Eleven steps (see the flow document): authenticate and validate; single-flight; pre-pin; capacity; sign the run principal;
register the frozen context and commit `sent` before the call; Core through the M9 route in-process; binding confirmed by
the first Flow node; drift check; whitelist projection and receipt; any later failure is `unknown`.

## Input / output
In: `CoreTaskInvocation` (annex D.2; stages `scout`, `verifier`, `builder_design`, `writer`; writer commitment, ADR 0003).
Out: `{schema_version, state, core_run_id, reason, outcome, task_binding_ref, receipt?, result?, proven_no_effect?,
adopted_writes?}`; HTTP 200 for terminal states, 202 otherwise. Credentials: `{jws, kid, exp}` returned once with
`Cache-Control: no-store`, never persisted or logged.

## Transactions and idempotency
Key `sha256(tenant|job|stage|attempt|logical_key)` must equal the `Idempotency-Key` header; `request_digest` =
`sha256(JCS(body minus request_digest, credentials, trace))`. Receipt CAS and single-flight: see the state machine
document. `sent` is committed before the Core call; capacity refusal deletes only a `prepared` row. Budget meter
`meter_spend` is atomic and capped.

## Errors
`pulso:invalid_request` (422, with field names), `tenant_mismatch` (403), `stage_unknown`, `input_too_large`,
`unknown_input_slot`, `digest_conflict` (409), `release_pin_unavailable`, `release_revoked`, `release_drift`,
`bridge_busy` (429 retryable), `not_found` (404), `credential_not_issuable` (403), `credential_signing_unavailable`
(503 retryable), `binding_unconfirmed`, `output_missing`, `fact_schema_violation`, `output_too_large`.

## Permissions
Run principal per stage (writer `constructor`, others `stage_task`), TTL 15 min, signed by the identity key. Credential
issuer: two independent signers (staff for `/v1/registry`, identity for `/v1/runs`); policy `(registry_write,
constructor) -> staff`, `(core_task, constructor) -> identity`; humans and approve/publish/promote roles are not issuable;
requires the tenant claim and equal body tenant; TTL <= 15 min. Binding callback token: callback key, `aud=control-api`.

## Config
`PULSO_BRIDGE_MAX_INFLIGHT` (8), `PULSO_BRIDGE_INSTANCE`, `PULSO_CONTROL_API_URL`, signer files under `/run/pulso-keys`;
`InvokeSettings` (`prepared_stale` 30 s, `context_ttl` 20 min, `principal_ttl` 15 min).

## Observability
Receipt rows (`reason` closed codes), `budget_meter` (calls, tokens, cost, `usage_known`, `reconciled`), the receipt's
`budget.known`, trace id echoed from Core, `trace_id` in envelopes from `traceparent` or a fresh id.

## Commands (head `4984d92`)
`python -m pytest -c pyproject.toml tests/l3a tests/integration -p no:cacheprovider` with `PULSO_TEST_PG_ADMIN` set
(throwaway PG16 on `pulso-dev`, `--cgroups=disabled`, removed afterwards). Environment as in journal claude-0002.

## RED / GREEN
First RED (`tests/l3a/test_first_red.py`): same key plus other digest must be 409 with no second `start_run`, and a timeout
after effect must be `unknown` (never `failed`) and never re-executed. GREEN as reported: 58 passed on real PG16 including a
real pinned Core in-process (alias pin, lost response -> `unknown` -> adopted, wrong-agent pin); review rounds added
live-key re-entry that never demotes an in-flight run, capped atomic spend, issuer tenant claim, concurrency and kill -9
tests. About 66 test functions exist in `tests/l3a`. Doubles: control-api callback (`MockTransport`), `bind_context` handler
stand-in, `FakeCore` for the fault matrix. Not re-run here.

## Trade-offs
Never calling `TurnEngine` directly keeps the full M9 route (auth, pin, idempotency) but runs Core in-process behind
ASGI. A failed writer after `sent` is `manual_reconcile`, trading automation for safety.

## Gaps
- `PulsoAuthz` must admit the `stage_task` role (done in `factories.py`; originally a reported gap).
- Reconciliation is request-driven; no background re-drive. `_live` in-flight set is per process.
- `put_draft` content cannot be re-verified on adoption (changes not retained).

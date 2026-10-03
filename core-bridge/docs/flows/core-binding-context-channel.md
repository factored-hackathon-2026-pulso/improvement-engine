# Flow: binding and context channel

Contract revision `pulso-two-teams-1`. Source: `invoke/context.py`, `tools/context.py`, `tools/dispatcher.py`,
`tools/bind.py`, `tools/guard.py`, `tools/authcheck.py`, `invoke/binding.py`, `pin.py`. Plan reference: 17.3.3 and the
CAP-27 context decision in 17.4.

## Problem
The tools and model gateway run on engine worker threads that do not inherit a `ContextVar`. A tool must still know which
tenant, job, stage and commitment it serves, and must never take those from tool arguments or model output.

## Channels
1. **Authoritative: signed principal attribute.** The run principal (signed by the bridge identity key, see
   `core-invoke.md` step 5) carries `task_binding_ref = sha256(tenant|idempotency_key)` plus `tenant`, `job`, `stage`,
   `attempt`, `pin_release_id`. `resolve_context(ctx, registry)` looks the ref up in the shared `InvocationRegistry`
   (immutable `InvocationContext`, TTL 20 min, removed on terminal). Missing attribute: `denied pulso:context_missing`;
   attrs disagree with the registered context: `denied pulso:context_mismatch`.
2. **Secondary: `ContextVar`** `CURRENT_BINDING`, set by `use_binding(ref)` around the in-process Core call. It is used
   only by `BindingGuardGateway` and `BindingGuardProvider`, whose ports carry no principal. When both channels exist and
   disagree the call is `context_mismatch`. A `ContextVar` alone is not trusted for tools (DR-05).
3. **Not a channel:** tool arguments. Tool schemas are closed (`additionalProperties:false`) and contain no tenant, job,
   grant or binding fields.

## Binding states and confirmation
`BindingState`: `pending -> confirmed | denied`. `ConfirmingRegistry` mirrors confirmation into the receipt CAS
(`sent -> binding_confirmed`).

```mermaid
sequenceDiagram
  participant F as Flow node 1: pulso/bind_context
  participant D as PulsoToolDispatcher
  participant A as control-api
  participant S as receipts
  F->>D: execute(bind_context) with principal attrs
  D->>D: resolve context (attr channel, ContextVar cross-check)
  D->>A: POST /internal/v1/core-task-bindings (callback JWT, Idempotency-Key = command_key)
  alt 200
    A-->>D: ok
    D->>S: sent -> binding_confirmed
    D-->>F: facts {binding_state: confirmed, stage flags, declared inputs}
  else 404 / 409
    D->>D: context denied (final)
    D-->>F: denied pulso:binding_failed:<code>
  else timeout / 5xx / network
    D-->>F: denied (effect unproven; receipt manual_reconcile in BindingService path)
  end
```
Request body: `{schema_version, tenant_id, job_id, command_key, request_digest, attempt, core_run_id,
bridge_instance_id, task_binding_ref}`. Token: ADR 0005 (`aud=control-api`, `scope=binding`).

## Gates (every tool call, in order)
1. tool known (`pulso/*` catalogue or protected `registry/*`) else `error unregistered_tool`;
2. context via signed attrs (`pulso:context_missing|context_mismatch`);
3. binding `confirmed` for every tool except `bind_context` (`pulso:binding_unconfirmed`);
4. stage allow-list `stage_allows(stage, tool)` (`tool_not_allowed`);
5. closed arguments, required arguments present (`invalid_args`);
6. broker `POST /broker/authorizations/check` (`pulso:authorization_denied` on false, 5xx, timeout, expired
   `valid_until`; positive answers cached only until `valid_until` and only for the exact operation, resources and
   payload digest);
7. handler (timeouts map to `timeout`, broker errors to `error`, never an engine crash).

Before binding is confirmed, `BindingGuardGateway` refuses model calls (`GatewayError refused`) and
`BindingGuardProvider` refuses Jev predictions: zero tokens, zero cost. Binding denial therefore means zero lab queries,
zero wiki/artifact calls, zero registry writes and zero model calls (L3b first RED, parametrised over `conflict`,
`digest_mismatch`, `not_found`, `unavailable`, `timeout`).

## Declared inputs
`bind_context` also exposes the stage's declared input slots as `facts.binding.value.<slot>` (ADR 0004).

## Concurrency
The channel is exercised by a 32-way concurrent invoke test and a `ThreadPoolExecutor` variant
(`tests/l3b/test_context_channel.py`); the CAP-27 pass criterion is the attribute channel, not the `ContextVar`.

## Gaps
`BindingService` (L3a, `invoke/binding.py`) and the tool-side `bind_context` (L3b, `tools/bind.py`) both implement the
callback; the composed runtime uses the tool-side path through `ControlApiClient`. Which of the two is the single source
of truth for the `manual_reconcile` on an unproven timeout was not re-verified in this pass: `BindingService._unproven`
moves the receipt, while `tools/bind.py` only denies the tool call.

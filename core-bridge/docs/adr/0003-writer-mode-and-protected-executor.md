# ADR 0003: Writer commitment modes and the protected builder executor

Status: accepted (L3b/L3a, plan 17.3.3 "Protected writer"). Contract revision: `pulso-two-teams-1`.

## Context
The `writer` stage is the only stage that can change the Core registry (create a proposal, put a draft, freeze,
reopen, validate, evaluate). Upstream `BuilderToolExecutor(service, actor, ids)` keeps its `_handlers` private, derives
write keys itself, and its `_evaluate` returns only `{"verdict"}`. A model-driven Flow must not be able to choose the
proposal, the content, or the idempotency key, and an evaluation run must not be reachable without an admission.

## Decision
1. **The writer carries a sealed `RegistryMutationCommitment`** inside the digested `CoreTaskInvocation` body
   (`invoke/models.py::RegistryMutationCommitmentDTO`, `extra="forbid"`; only valid for `stage == "writer"`). It is part
   of `request_digest`, so same-key/other-commitment is a `digest_conflict`. Fields: `mode`, `proposal_id`,
   `expected_rev`, `base_release_id`, `evaluate_enabled`, `evaluation_context_ref`, `create_agent_id|origin|title`,
   `put_draft_digest`, `operations` (ordered committed non-evaluate writes, max 32).
2. **Two modes.** `mode="write"` allows `create_proposal, put_draft, freeze, reopen, validate, get_proposal,
   get_write` (`WRITE_MODE_TOOLS`). `mode="evaluate_only"` allows only `validate, get_proposal, get_write`
   (`EVALUATE_ONLY_TOOLS`): every mutator is denied at the executor. In both modes `registry/evaluate` is added only
   when `evaluate_enabled` is true and `evaluation_context_ref` is set.
3. **Wrap, never subclass or call privates.** `tools/builder.py::ProtectedBuilderToolExecutor` wraps the upstream
   executor. Gate order: tool registered, context resolved from the signed principal attribute, binding confirmed, stage
   is `writer` with a commitment, mode allow-list, closed args (subset of the ToolDef `args_schema`), commitment match,
   broker authorization (mutators other than evaluate). Any failure is `denied` with zero effect on the registry.
4. **Commitment match.** `create_proposal`: agent, origin and title equal the sealed values and no proposal id is
   committed (one create per invocation). `put_draft`: `args.proposal_id` equals the committed id (or the one created
   in this invocation), `expected_rev` equals the committed one when set, and `sha256(JCS({proposal_id, expected_rev,
   changes}))` equals `put_draft_digest` (a fresh proposal is bound with null id and null rev). Other mutators match the
   proposal id. A missing digest is a mismatch.
5. **Keys are derived by the bridge, not by the engine.** `pulso-w:` + `sha256("<command_key>|<stage>|<ordinal>")[:48]`
   where `ordinal` is the index of the operation in `commitment.operations`; an operation not in the array is
   `commitment_mismatch`. Evaluate uses `pulso-eval:<evaluation_context_ref>` (ADR 0007). The engine key is remembered
   per binding so `registry/get_write` can be translated; a `get_write` for a key that is neither mapped nor derivable
   from this command is denied (no cross-invocation receipt reads). Results carry `key_digest`, never the key.
6. **Evaluate is handled inside the wrapper** and delegated to an `EvaluationGate` port (L5 `FlowEvaluationGate`), never
   to upstream `_evaluate`. It fails closed with `pulso:evaluation_gate_unavailable` when no gate is wired.
7. **The writer principal is a constructor bot**, not the run principal: `pulso-constructor:<tenant>` with role
   `constructor` (`main._constructor_principal`); the run principal carries role `constructor` only for the writer
   (`InvokeSettings.stage_roles`), `stage_task` for the other stages.
8. **A failed writer is never provably failed.** After `sent`, a writer that fails goes to `manual_reconcile`, the other
   stages to `terminal_failed` (`invoke/service.py::_failed_state`).

## Consequences
The model can only run the committed operations in order, with the committed content. Content of `put_draft` cannot be
re-hashed during reconciliation because the `changes` are not retained (reconciliation can verify operation, proposal
and revision only; see `docs/flows/core-reconcile-matrix.md`). Known gap (BITACORA, L3b): the writer Agent asset must
list `registry/reopen@1` in `tools_allowed` for the reopen path; the current catalogue (`stages/catalog.py`) does list
it, but the asset-side status was not re-verified in this documentation pass.

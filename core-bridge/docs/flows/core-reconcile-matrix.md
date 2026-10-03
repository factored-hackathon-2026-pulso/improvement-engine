# Flow: reconcile matrix

Contract revision `pulso-two-teams-1`. Source: `src/pulso_core_runtime/reconcile/reconciler.py::Reconciler`,
`adapters.py` (`sealed_commitment_check`, `ServiceWriteProbe`), `invoke/wiring.py`.

Reconciliation is **read-only towards Core and the registry**: it never starts a run and never writes to the registry.
It is invoked when a same-key request meets a non-terminal receipt (see `core-invoke.md`). Any exception while reading
evidence moves the receipt to `unknown` (`evidence_unreadable`): unreadable evidence proves nothing.

## Decision order and outcomes

| Order | Evidence | Result state | Reason | Notes |
|---|---|---|---|---|
| 0 | receipt already terminal | unchanged | stored | returned as is |
| 1 | stored Core `RunResult` for `(principal_id, key)` exists | decided by it | | wins over every other evidence |
| 1a | Core `release` differs from the pinned one | `terminal_failed` | `release_drift` | result discarded |
| 1b | outcome not `completed`/`failed` | writer `manual_reconcile`, else `terminal_failed` | `unexpected_outcome` | outcome recorded, no facts |
| 1c | projection `BridgeError` | `terminal_failed` | the error code | |
| 1d | `completed` but binding not proven (state `binding_confirmed` or a binding row) | `manual_reconcile` | `binding_unconfirmed` | |
| 1e | otherwise | `terminal_ok` (completed) or failed-state (failed) | `adopted_core_result` | stores projected `result` and `trace_id` |
| 2 | no Core result, receipt `prepared` | `terminal_failed` | `never_sent` | `proven_no_effect=true` |
| 3 | no Core result, `sent`, no binding row | `manual_reconcile` | `sent_without_binding` | |
| 4 | binding row has a `core_run_id` and Core has that run but no idempotency record | `manual_reconcile` | `run_without_idempotency_record` | |
| 5 | binding row, no Core commit: for every expected write key call `get_write(key)` | | | registry effects may exist |
| 5a | `get_write` raises | `unknown` | `get_write_unavailable` | |
| 5b | a write is present but the commitment check is missing or fails | `manual_reconcile` | `commitment_mismatch` | adopted keys listed |
| 5c | writes present and verified | `manual_reconcile` | `adopted_writes` | adopted keys listed; a human or Codex decides |
| 5d | no write present, meter missing or not `reconciled` | `manual_reconcile` | `budget_unreconciled` | |
| 5e | no write present and meter `reconciled` | `terminal_failed` | `no_core_commit_no_effect` | `proven_no_effect=true` |

Failed-state means `manual_reconcile` for the writer stage and `terminal_failed` for the others.

## Definitions
- "binding row" = `ReceiptBindingLookup`: the receipt is `binding_confirmed` or already carries a `core_run_id`
  (proof that the control-api saw this command).
- "expected write keys" = `expected_write_keys`: writer stage only; one derived `pulso-w:` key per entry of the sealed
  `operations` array in the persisted context, plus `pulso-eval:<ref>` when an evaluation context ref was sealed.
  Non-writer stages expect none, so row 5 for them ends in 5d or 5e.

## Commitment verification of adopted writes (`sealed_commitment_check`)
The invocation context persists the sealed commitment. For each present write the check verifies: the operation sits at
its committed ordinal, the `create_proposal` content hash, the proposal id, the revision, and fresh-proposal
consistency. With **no verifier wired the answer is `commitment_mismatch`** (fail closed). `put_draft` content cannot be
re-hashed because the `changes` are not retained: only operation, proposal and revision are verified.

## Evidence in tests
`tests/l3a/test_reconcile_matrix.py` (fault matrix with a fake Core), `tests/l3a/test_real_core.py` (real pinned Core
in-process: lost response, `unknown`, adopted; wrong-agent pin), `tests/integration/test_reconcile_kill9.py` (a real
subprocess commits a registry write and is SIGKILLed; the composed app reports adopted writes or `commitment_mismatch`),
`tests/l3a/test_commitment_check.py`.

## Gaps
Re-drive is request-driven only. The matrix assumes the registry exposes `get_write(key)` (the composed app uses
`ServiceWriteProbe` over the evaluation service). Evidence for `pulso-eval:` keys on the reconcile path is covered by
the evaluation runtime's own replay (`core-evaluation-admission-arms.md`), not by this matrix.

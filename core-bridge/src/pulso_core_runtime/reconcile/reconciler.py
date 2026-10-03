"""Reconciliation (plan 17.3.3 "Receipt states and reconciliation"). Synchronous and read-only towards Core and
the registry: it never starts a run and never writes to the registry. Same key while `unknown` re-reads here.

Order: (1) stored Core `RunResult` for (principal, key) is terminal and wins; (2) a binding row with no
committed run means no Core commit but registry effects may exist; (3) per expected write `get_write(key)`:
all absent plus a reconciled budget -> `proven_no_effect`, any present -> adopt and verify against the
commitment; (4) no binding and `sent` -> `manual_reconcile`. Anything unreadable stays `unknown`."""

from __future__ import annotations

from collections.abc import Callable, Sequence
from dataclasses import dataclass, field
from typing import Any, Protocol

from pulso_core_runtime.invoke.errors import BridgeError
from pulso_core_runtime.invoke.pin import RELEASE_DRIFT
from pulso_core_runtime.invoke.projection import FactProjector
from pulso_core_runtime.store.receipts import Receipt, ReceiptStore


class RunReader(Protocol):
    def load_run(self, run_id: str) -> Any | None: ...
    def get_run_idempotency(self, principal_id: str, key: str) -> tuple[str, Any] | None: ...


class WriteProbe(Protocol):
    def get_write(self, key: str) -> dict[str, Any] | None: ...


class BindingLookup(Protocol):
    def lookup(self, tenant_id: str, command_key: str) -> dict[str, Any] | None: ...


@dataclass(frozen=True)
class ReconcileResult:
    state: str
    reason: str
    proven_no_effect: bool = False
    adopted_writes: list[str] = field(default_factory=list)
    core_run_id: str | None = None


def _get(obj: Any, name: str) -> Any:
    return obj.get(name) if isinstance(obj, dict) else getattr(obj, name, None)


def _text(value: Any) -> Any:
    return getattr(value, "value", value)


class Reconciler:
    def __init__(self, *, store: ReceiptStore, runs: RunReader, writes: WriteProbe | None = None,
                 bindings: BindingLookup | None = None, projector: FactProjector | None = None,
                 commitment_check: Callable[[Receipt, str, dict[str, Any]], bool] | None = None,
                 expected_writes: Callable[[Receipt], Sequence[str]] = lambda r: ()) -> None:
        self._store, self._runs, self._writes, self._bindings = store, runs, writes, bindings
        self._projector, self._check, self._expected = projector, commitment_check, expected_writes

    def reconcile(self, receipt: Receipt) -> ReconcileResult:
        if receipt.terminal:
            return ReconcileResult(receipt.state, receipt.reason or "terminal", core_run_id=receipt.core_run_id)
        if receipt.state == "manual_reconcile" and receipt.reason == "binding_unproven":
            # The binding callback was a 5xx/timeout: the platform may have applied it, and nothing the bridge can
            # read proves otherwise. Closing it (even as a failure) would invite a redispatch the platform refuses
            # as binding_conflict forever, so it waits for control-api evidence / an operator.
            return ReconcileResult(receipt.state, "binding_unproven", core_run_id=receipt.core_run_id)
        try:
            return self._reconcile(receipt)
        except Exception:  # unreadable evidence is never proof of anything
            return self._move(receipt, "unknown", "evidence_unreadable")

    def _move(self, row: Receipt, state: str, reason: str, **extra: Any) -> ReconcileResult:
        payload: dict[str, Any] = dict(extra.get("receipt") or {})
        if extra.get("proven_no_effect"):
            payload["proven_no_effect"] = True
        if extra.get("adopted"):
            payload["adopted_writes"] = list(extra["adopted"])
        moved = self._store.transition(row.tenant_id, row.idempotency_key, state, reason=reason,
                                       core_run_id=extra.get("core_run_id"), outcome=extra.get("outcome"),
                                       receipt=payload or None)
        final = moved or self._store.get(row.tenant_id, row.idempotency_key) or row
        return ReconcileResult(final.state, reason if moved else (final.reason or reason),
                               proven_no_effect=bool(extra.get("proven_no_effect")),
                               adopted_writes=list(extra.get("adopted") or []), core_run_id=final.core_run_id)

    def _binding_proven(self, receipt: Receipt) -> bool:
        if receipt.state == "binding_confirmed":
            return True
        return bool(self._bindings is not None and self._bindings.lookup(receipt.tenant_id, receipt.idempotency_key))

    def _reconcile(self, receipt: Receipt) -> ReconcileResult:
        # (1) the stored RunResult wins
        stored = self._runs.get_run_idempotency(receipt.principal_id, receipt.idempotency_key)
        if stored is not None:
            result = stored[1]
            run_id, outcome = _get(result, "run_id"), str(_text(_get(result, "outcome")))
            if _get(result, "release") != receipt.release_id:
                return self._move(receipt, "terminal_failed", RELEASE_DRIFT.removeprefix("pulso:"),
                                  core_run_id=run_id)
            if outcome not in ("completed", "failed"):
                return self._move(receipt, "manual_reconcile" if receipt.stage == "writer" else "terminal_failed",
                                  "unexpected_outcome", core_run_id=run_id,
                                  outcome=outcome, receipt={"core_outcome": outcome})
            envelope = None
            if self._projector is not None:
                try:
                    envelope = self._projector.project(receipt.stage, run_id, self._runs.load_run(run_id),
                                                       status=str(_text(_get(result, "status"))), outcome=outcome,
                                                       binding_ref=receipt.task_binding_ref)
                except BridgeError as exc:
                    return self._move(receipt, "terminal_failed", exc.code.removeprefix("pulso:"),
                                      core_run_id=run_id, outcome=outcome)
            if outcome == "completed" and not self._binding_proven(receipt):
                return self._move(receipt, "manual_reconcile", "binding_unconfirmed", core_run_id=run_id,
                                  outcome=outcome)
            failed = "manual_reconcile" if receipt.stage == "writer" else "terminal_failed"
            state = "terminal_ok" if outcome == "completed" else failed
            return self._move(receipt, state, "adopted_core_result", core_run_id=run_id, outcome=outcome,
                              receipt={"result": envelope, "trace_id": _get(result, "trace_id")})
        # (4) no evidence of a send at all
        if receipt.state == "prepared":
            return self._move(receipt, "terminal_failed", "never_sent", proven_no_effect=True)
        binding = self._bindings.lookup(receipt.tenant_id, receipt.idempotency_key) if self._bindings else None
        if binding is None:
            return self._move(receipt, "manual_reconcile", "sent_without_binding")
        run_id = binding.get("core_run_id")
        if run_id and self._runs.load_run(run_id) is not None:
            return self._move(receipt, "manual_reconcile", "run_without_idempotency_record", core_run_id=run_id)
        # (2)+(3) binding without a Core commit: registry effects may still exist
        present: list[str] = []
        for key in self._expected(receipt):
            try:
                found = self._writes.get_write(key) if self._writes else None
            except Exception:
                return self._move(receipt, "unknown", "get_write_unavailable", core_run_id=run_id)
            if found is not None:
                present.append(key)
                if self._check is None or not self._check(receipt, key, found):  # no verifier == unverifiable
                    return self._move(receipt, "manual_reconcile", "commitment_mismatch", core_run_id=run_id,
                                      adopted=present)
        if present:
            return self._move(receipt, "manual_reconcile", "adopted_writes", core_run_id=run_id, adopted=present)
        meter = self._store.meter_get(receipt.tenant_id, receipt.job_id, receipt.stage, receipt.attempt)
        if meter is None or not meter["reconciled"]:
            return self._move(receipt, "manual_reconcile", "budget_unreconciled", core_run_id=run_id)
        return self._move(receipt, "terminal_failed", "no_core_commit_no_effect", core_run_id=run_id,
                          proven_no_effect=True)

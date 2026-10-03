"""Composition adapters between the L3b broker client and the L5 evaluation ports (owned by the integrator).

Everything here fails closed: an unconfigured broker, an unknown budget or a missing artifact port denies or
reports `failed_infra`; nothing is ever assumed allowed."""

from __future__ import annotations

import json
from datetime import datetime
from decimal import Decimal
from pathlib import Path
from typing import Any

from pulso_core_runtime.evaluation.budget import BudgetLimits
from pulso_core_runtime.tools.broker import BrokerClient


class BrokerAuthPort:
    """`BrokerPort` over `POST /broker/authorizations/check` (operation = the evaluation scope name)."""

    def __init__(self, broker: BrokerClient) -> None:
        self._broker = broker

    def check(self, *, tenant_id: str, binding_ref: str, scope: str, payload_digest: str) -> bool:
        try:
            decision = self._broker.authorization_check(binding_ref, scope, [], payload_digest)
        except Exception:  # noqa: BLE001 - broker down/timeout/garbage is a denial, never a pass
            return False
        return decision.allowed

    def __repr__(self) -> str:
        return "BrokerAuthPort"


class StaticBudgetResolver:
    """`budget_ref -> BudgetLimits` from a JSON file `{"<budget_ref>": {"cost_usd_max", "tokens_max", "jobs_max",
    "deadline", "tenant_id"?}}`. No file (or an unknown ref / other tenant) resolves to None (fail closed).
    A stand-in until the control-api budget resolver contract exists (reported in `/version.doubles[]`)."""

    def __init__(self, path: Path | None) -> None:
        self._table: dict[str, Any] = {}
        if path is not None and path.is_file():
            self._table = json.loads(path.read_text(encoding="utf-8"))

    @property
    def configured(self) -> bool:
        return bool(self._table)

    def resolve(self, budget_ref: str, tenant_id: str) -> BudgetLimits | None:
        entry = self._table.get(budget_ref)
        if not isinstance(entry, dict) or entry.get("tenant_id", tenant_id) != tenant_id:
            return None
        deadline = entry.get("deadline")
        cost = entry.get("cost_usd_max")
        return BudgetLimits(budget_ref, None if cost is None else Decimal(str(cost)), entry.get("tokens_max"),
                            entry.get("jobs_max"), None if deadline is None else datetime.fromisoformat(deadline))


class NoArtifactPort:
    """`ArtifactPort` without a broker route that carries a binding: the arm runner reports
    `failed_infra manifest_missing` (documented gap: `artifact_get(ref, tenant_id)` has no binding_ref)."""

    def artifact_get(self, ref: str, tenant_id: str) -> dict[str, Any]:
        raise LookupError("no artifact port")


class ReceiptBindingLookup:
    """`BindingLookup` over the receipt row: a confirmed binding (or one that recorded a Core run id) is proof
    that the control-api saw this command."""

    def __init__(self, store: Any) -> None:
        self._store = store

    def lookup(self, tenant_id: str, command_key: str) -> dict[str, Any] | None:
        row = self._store.get(tenant_id, command_key)
        if row is None or not (row.state == "binding_confirmed" or row.core_run_id):
            return None
        return {"core_run_id": row.core_run_id}


class ServiceWriteProbe:
    """`WriteProbe` over the shared `RegistryService.get_write` (read-only). `service_getter` is late-bound."""

    def __init__(self, service_getter: Any) -> None:
        self._service = service_getter

    def get_write(self, key: str) -> dict[str, Any] | None:
        record = self._service().get_write(key)
        return None if record is None else dict(record.model_dump(mode="json"))


def expected_write_keys(store: Any) -> Any:
    """`Reconciler.expected_writes`: the derived write keys (ordinal = index in the sealed `operations`) plus the
    evaluate key, read from the invocation context persisted before `sent`."""
    from pulso_core_runtime.tools.builder import eval_key, write_key

    def expected(receipt: Any) -> list[str]:
        if receipt.stage != "writer":
            return []
        row = store.context_row(receipt.task_binding_ref)
        ctx = (row or {}).get("context") or {}
        keys = [write_key(receipt.idempotency_key, receipt.stage, n) for n in range(len(ctx.get("operations", [])))]
        ref = ctx.get("evaluation_context_ref")
        if ref:
            keys.append(eval_key(ref))
        return keys

    return expected


class SpendMeteringGateway:
    """Charges every live model call to `ReceiptStore.meter_spend` (atomic, capped). The cap comes from the
    sealed invocation `budget.cost_usd_max`; an exhausted cap refuses the call result (`GatewayError.refused`).
    Without a cap the spend is recorded against an unreachable cap (metering only, Core budgets still apply)."""

    UNCAPPED = "1000000000"

    def __init__(self, inner: Any, contexts: Any, store: Any,
                 current: Any = None) -> None:
        from pulso_core_runtime.tools.context import current_binding

        self._inner, self._contexts, self._store = inner, contexts, store
        self._current = current or current_binding

    def generate(self, prompt: Any, inputs_model_view: Any, locale: Any, schema: Any = None) -> Any:
        from agent_core.domain.errors import GatewayError, GatewayErrorKind

        ref = self._current()
        result = self._inner.generate(prompt, inputs_model_view, locale, schema)
        if ref is None:
            return result
        ic = self._contexts.lookup(ref)
        row = self._store.context_row(ref)
        budget = ((row or {}).get("context") or {}).get("budget") or {}
        cap = str(budget.get("cost_usd_max", self.UNCAPPED))
        cost = str(getattr(result, "cost_usd", 0) or 0)
        tokens = int(getattr(result, "tokens_in", 0)) + int(getattr(result, "tokens_out", 0))
        if not self._store.meter_spend(ic.tenant_id, ic.job_id, ic.stage, ic.attempt, cost_usd=cost, cap_usd=cap,
                                       tokens=tokens):
            raise GatewayError(GatewayErrorKind.refused)
        return result


class EvalTranscript:
    """In-process, bounded transcript store for EVALUATION runs only (synthetic principals; the isolated eval
    DB has no transcript table). The live path keeps `factories.NullTranscript` (task-only, no transcripts)."""

    def __init__(self, max_runs: int = 512) -> None:
        import threading
        from collections import OrderedDict

        self._runs: Any = OrderedDict()
        self._max, self._n, self._lock = max_runs, 0, threading.Lock()

    def append(self, entry: Any) -> str:
        with self._lock:
            self._n += 1
            entries = self._runs.setdefault(entry.run_id, [])
            entries.append(entry)
            while len(self._runs) > self._max:
                self._runs.popitem(last=False)
            return f"eval-entry-{self._n:08d}"

    def read(self, run_id: str) -> list[Any]:
        with self._lock:
            return list(self._runs.get(run_id, []))

    def recent_turns(self, run_id: str, n: int) -> list[Any]:
        return self.read(run_id)[-n:] if n > 0 else []

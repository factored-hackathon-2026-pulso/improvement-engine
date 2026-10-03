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
from pulso_core_runtime.llm.metering import (
    SpendMeteringGateway,  # noqa: F401  (re-export: moved to llm/)
)
from pulso_core_runtime.tools.broker import BrokerClient


class BrokerAuthPort:
    """`BrokerPort` over `POST /broker/authorizations/check` (operation = the evaluation scope name)."""

    def __init__(self, broker: BrokerClient) -> None:
        self._broker = broker

    def check(self, *, tenant_id: str, binding_ref: str, scope: str, payload_digest: str) -> bool:
        try:
            decision = self._broker.authorization_check(binding_ref, scope, [], payload_digest, tenant_id=tenant_id)
        except Exception:  # noqa: BLE001 - broker down/timeout/garbage is a denial, never a pass
            return False
        return decision.allowed

    def __repr__(self) -> str:
        return "BrokerAuthPort"


class StaticBudgetResolver:
    """`budget_ref -> BudgetLimits` from a JSON file `{"<budget_ref>": {"cost_usd_max", "tokens_max", "jobs_max",
    "deadline", "tenant_id"?}}`. No file (or an unknown ref / other tenant) resolves to None (fail closed).
    A stand-in until the control-api budget resolver contract exists (reported in `/version.doubles[]`)."""

    def __init__(self, path: Path | None, inline: str | None = None) -> None:
        """`inline` (env `PULSO_EVAL_BUDGETS_JSON`, same JSON shape; budgets carry no secrets) is for compose/Fargate
        where mounting a file is awkward; a readable file wins over it. Invalid inline JSON raises `ValueError`."""
        self._table: dict[str, Any] = {}
        if path is not None and path.is_file():
            self._table = json.loads(path.read_text(encoding="utf-8"))
        elif inline and inline.strip():
            try:
                table = json.loads(inline)
            except ValueError:
                raise ValueError("PULSO_EVAL_BUDGETS_JSON is not valid JSON") from None
            if not isinstance(table, dict):
                raise ValueError("PULSO_EVAL_BUDGETS_JSON must be a JSON object")
            self._table = table

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


def _request_hash(op: str, payload: Any) -> str:
    """Mirror of the pinned `RegistryService._request_hash` (it is private there; drift is caught by the
    commitment tests, which would reject every legitimate write)."""
    import hashlib

    from agent_core.domain.json import canonical_bytes

    return hashlib.sha256(canonical_bytes({"op": op, "payload": payload})).hexdigest()


def sealed_commitment_check(store: Any, get_write: Any) -> Any:
    """`Reconciler.commitment_check`: an adopted registry write must be exactly what the Codex-sealed commitment
    (persisted in the invocation context before `sent`) allows: the op at its committed ordinal, the committed
    proposal (or, for a fresh proposal, the one the committed `create_proposal` produced), the sealed create
    content, and a revision past `expected_rev`. Anything unreadable, unknown or out of commitment is False
    (the receipt then stays `manual_reconcile`). `put_draft` content cannot be re-hashed here (changes are not
    retained); its op/proposal/revision are checked."""
    from pulso_core_runtime.tools.builder import eval_key, write_key

    def check(receipt: Any, key: str, found: dict[str, Any]) -> bool:
        ctx = ((store.context_row(receipt.task_binding_ref) or {}).get("context")) or {}
        c = ctx.get("commitment")
        if not isinstance(c, dict) or c.get("mode") not in ("write", "evaluate_only"):
            return False
        op, pid, rev = found.get("op"), found.get("proposal_id"), found.get("rev_after")
        if not isinstance(op, str) or not isinstance(pid, str) or not isinstance(rev, int):
            return False
        committed_pid = c.get("proposal_id")
        if committed_pid is not None and pid != committed_pid:
            return False
        ref = c.get("evaluation_context_ref")
        try:
            is_eval = isinstance(ref, str) and key == eval_key(ref)
        except ValueError:
            return False
        if is_eval:
            return op == "evaluate" and c.get("evaluate_enabled") is True and _same_proposal(
                c, get_write, receipt, pid, ctx)
        ops = list(c.get("operations") or [])
        if c["mode"] != "write":
            return False
        ordinal = next((n for n in range(len(ops)) if write_key(receipt.idempotency_key, receipt.stage, n) == key),
                       None)
        if ordinal is None or ops[ordinal] != op:
            return False
        if op == "create_proposal":
            expected = _request_hash(op, {"agent_id": c.get("create_agent_id"), "origin": c.get("create_origin"),
                                          "title": c.get("create_title")})
            return found.get("request_hash") == expected and c.get("create_title") is not None
        if not _same_proposal(c, get_write, receipt, pid, ctx):
            return False
        if op in ("freeze", "reopen"):
            return found.get("request_hash") == _request_hash(op, {"proposal_id": pid})
        if op == "put_draft":
            expected_rev = c.get("expected_rev")
            return expected_rev is None or rev > expected_rev
        return False

    return check


def _same_proposal(c: dict[str, Any], get_write: Any, receipt: Any, pid: str, ctx: dict[str, Any]) -> bool:
    """A fresh proposal (id unknown at commit time) must be the one the committed `create_proposal` made."""
    if c.get("proposal_id") is not None:
        return pid == c["proposal_id"]
    from pulso_core_runtime.tools.builder import write_key

    ops = list(c.get("operations") or [])
    if "create_proposal" not in ops:
        return False
    created = get_write(write_key(receipt.idempotency_key, receipt.stage, ops.index("create_proposal")))
    return isinstance(created, dict) and created.get("proposal_id") == pid


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

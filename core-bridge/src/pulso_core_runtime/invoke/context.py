"""Frozen `InvocationContext` and the registry keyed by the signed `task_binding_ref` attr (plan 17.3.3
"Context channel"). The attr channel is primary; the ContextVar is a redundant check, never the only source."""

from __future__ import annotations

import threading
from contextvars import ContextVar
from dataclasses import asdict, dataclass
from datetime import UTC, datetime, timedelta
from typing import Any


class ContextMissing(LookupError):
    code = "pulso:context_missing"


class ContextMismatch(LookupError):
    code = "pulso:context_mismatch"


@dataclass(frozen=True)
class InvocationContext:
    tenant_id: str
    job_id: str
    stage: str
    attempt: int
    grant_ref: str
    task_binding_ref: str
    pin_release_id: str
    idempotency_key: str
    request_digest: str
    expires_at: datetime
    budget: tuple[tuple[str, Any], ...] = ()
    cutoff: str | None = None
    deadline: str | None = None

    def as_json(self) -> dict[str, Any]:
        d = asdict(self)
        d["expires_at"] = self.expires_at.isoformat()
        d["budget"] = dict(self.budget)
        return d


_CURRENT: ContextVar[InvocationContext | None] = ContextVar("pulso_invocation_context", default=None)


class InvocationRegistry:
    def __init__(self, ttl: timedelta = timedelta(minutes=20),
                 now: Any = lambda: datetime.now(UTC)) -> None:
        self._items: dict[str, InvocationContext] = {}
        self._lock = threading.Lock()
        self._ttl, self._now = ttl, now

    def register(self, ctx: InvocationContext) -> None:
        with self._lock:
            self._items[ctx.task_binding_ref] = ctx

    def remove(self, ref: str) -> None:
        with self._lock:
            self._items.pop(ref, None)

    def __len__(self) -> int:
        return len(self._items)

    def lookup(self, ref: str | None) -> InvocationContext:
        if not ref:
            raise ContextMissing()
        with self._lock:
            ctx = self._items.get(ref)
        if ctx is None or ctx.expires_at <= self._now():
            raise ContextMissing()
        return ctx

    def resolve(self, principal_attrs: dict[str, str]) -> InvocationContext:
        """Context for a tool call: looked up by the signed attr (never from tool args), cross-checked
        against the signed tenant/job and, when set, the ContextVar."""
        ctx = self.lookup(principal_attrs.get("task_binding_ref"))
        if (principal_attrs.get("tenant"), principal_attrs.get("job")) != (ctx.tenant_id, ctx.job_id):
            raise ContextMismatch()
        current = _CURRENT.get()
        if current is not None and current.task_binding_ref != ctx.task_binding_ref:
            raise ContextMismatch()
        return ctx


def bind_current(ctx: InvocationContext | None) -> Any:
    return _CURRENT.set(ctx)


def reset_current(token: Any) -> None:
    _CURRENT.reset(token)


def current_context() -> InvocationContext | None:
    return _CURRENT.get()

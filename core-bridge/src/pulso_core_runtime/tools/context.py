"""Invocation context channel (plan 17.3.3 "Context channel").

The authoritative channel is the signed `Principal.attrs["task_binding_ref"]`: it is looked up in the
`InvocationRegistry` (immutable, TTL, deleted on terminal). A `ContextVar` is a redundant secondary channel;
when both exist and disagree the call is `denied pulso:context_mismatch`. Binding refs are never read from
tool args. A ContextVar alone is NOT enough: worker threads of a pool do not inherit it (DR-05)."""

from __future__ import annotations

import threading
from collections.abc import Callable, Iterator
from contextlib import contextmanager
from contextvars import ContextVar
from dataclasses import dataclass, field
from datetime import UTC, datetime
from enum import StrEnum
from typing import Any

from agent_core.ports import ToolCallContext

CONTEXT_MISSING = "pulso:context_missing"
CONTEXT_MISMATCH = "pulso:context_mismatch"


class ContextError(Exception):
    def __init__(self, code: str) -> None:
        super().__init__(code)
        self.code = code


class BindingState(StrEnum):
    pending = "pending"
    confirmed = "confirmed"
    denied = "denied"


@dataclass(frozen=True)
class RegistryMutationCommitment:
    """What the writer is allowed to do (Codex-sealed). `mode="evaluate_only"` denies every mutator."""

    mode: str  # "write" | "evaluate_only"
    proposal_id: str | None
    expected_rev: int | None
    base_release_id: str | None
    evaluate_enabled: bool
    evaluation_context_ref: str | None = None
    # create_proposal commitment (title carries the `pulso-key:<k>` marker sealed by Codex)
    create_agent_id: str | None = None
    create_origin: str | None = None
    create_title: str | None = None
    # put_draft commitment: sha256 hex of JCS({proposal_id, expected_rev, changes})
    put_draft_digest: str | None = None


@dataclass(frozen=True)
class InvocationContext:
    tenant_id: str
    job_id: str
    stage: str
    attempt: int
    binding_ref: str
    command_key: str
    request_digest: str
    bridge_instance_id: str
    expires_at: datetime
    memory_snapshot_ref: str | None = None
    extract_manifest_ref: str | None = None
    commitment: RegistryMutationCommitment | None = None

    @property
    def evaluation_context_ref(self) -> str | None:
        return self.commitment.evaluation_context_ref if self.commitment else None


@dataclass
class _Entry:
    ctx: InvocationContext
    state: BindingState = BindingState.pending
    session: dict[str, Any] | None = None
    ordinals: dict[str, int] = field(default_factory=dict)
    seen_refs: set[str] = field(default_factory=set)
    scratch: dict[str, Any] = field(default_factory=dict)
    auth_cache: dict[tuple[Any, ...], datetime] = field(default_factory=dict)
    lock: threading.Lock = field(default_factory=threading.Lock)


CURRENT_BINDING: ContextVar[str | None] = ContextVar("pulso_task_binding_ref", default=None)


@contextmanager
def use_binding(binding_ref: str) -> Iterator[None]:
    token = CURRENT_BINDING.set(binding_ref)
    try:
        yield
    finally:
        CURRENT_BINDING.reset(token)


def current_binding() -> str | None:
    return CURRENT_BINDING.get()


class InvocationRegistry:
    """Thread-safe store of frozen contexts keyed by `task_binding_ref`."""

    def __init__(self, clock: Callable[[], datetime] | None = None) -> None:
        self._clock = clock or (lambda: datetime.now(UTC))
        self._entries: dict[str, _Entry] = {}
        self._lock = threading.Lock()

    def register(self, ctx: InvocationContext) -> None:
        with self._lock:
            if ctx.binding_ref in self._entries:
                raise ValueError("binding_ref already registered")
            self._entries[ctx.binding_ref] = _Entry(ctx)

    def remove(self, binding_ref: str) -> None:
        with self._lock:
            self._entries.pop(binding_ref, None)

    def _entry(self, binding_ref: str) -> _Entry:
        with self._lock:
            entry = self._entries.get(binding_ref)
        if entry is None or entry.ctx.expires_at <= self._clock():
            raise ContextError(CONTEXT_MISSING)
        return entry

    def lookup(self, binding_ref: str) -> InvocationContext:
        return self._entry(binding_ref).ctx

    def state(self, binding_ref: str | None) -> BindingState | None:
        """None when unknown/expired (never confirmed)."""
        if not binding_ref:
            return None
        try:
            return self._entry(binding_ref).state
        except ContextError:
            return None

    def is_confirmed(self, binding_ref: str | None) -> bool:
        return self.state(binding_ref) is BindingState.confirmed

    def confirm(self, binding_ref: str) -> None:
        entry = self._entry(binding_ref)
        with entry.lock:
            if entry.state is not BindingState.denied:
                entry.state = BindingState.confirmed

    def deny(self, binding_ref: str) -> None:
        entry = self._entry(binding_ref)
        with entry.lock:
            entry.state = BindingState.denied

    # -- per-invocation scratch (all under the entry lock) -------------------
    def session(self, binding_ref: str) -> dict[str, Any] | None:
        return self._entry(binding_ref).session

    def set_session(self, binding_ref: str, session: dict[str, Any]) -> None:
        entry = self._entry(binding_ref)
        with entry.lock:
            entry.session = session

    def ordinal(self, binding_ref: str, engine_key: str) -> int:
        """Stable position of a write action within the invocation (same engine key -> same ordinal)."""
        entry = self._entry(binding_ref)
        with entry.lock:
            if engine_key not in entry.ordinals:
                entry.ordinals[engine_key] = len(entry.ordinals)
            return entry.ordinals[engine_key]

    def ordinal_of(self, binding_ref: str, engine_key: str) -> int | None:
        entry = self._entry(binding_ref)
        with entry.lock:
            return entry.ordinals.get(engine_key)

    def remember(self, binding_ref: str, key: str, value: Any) -> None:
        entry = self._entry(binding_ref)
        with entry.lock:
            entry.scratch[key] = value

    def recall(self, binding_ref: str, key: str, default: Any = None) -> Any:
        entry = self._entry(binding_ref)
        with entry.lock:
            return entry.scratch.get(key, default)

    def record_refs(self, binding_ref: str, refs: list[str]) -> None:
        entry = self._entry(binding_ref)
        with entry.lock:
            entry.seen_refs.update(r for r in refs if r)

    def seen_refs(self, binding_ref: str) -> frozenset[str]:
        entry = self._entry(binding_ref)
        with entry.lock:
            return frozenset(entry.seen_refs)

    def auth_cached(self, binding_ref: str, key: tuple[Any, ...]) -> bool:
        entry = self._entry(binding_ref)
        with entry.lock:
            until = entry.auth_cache.get(key)
            return until is not None and until > self._clock()

    def auth_store(self, binding_ref: str, key: tuple[Any, ...], valid_until: datetime) -> None:
        entry = self._entry(binding_ref)
        with entry.lock:
            entry.auth_cache[key] = valid_until

    def now(self) -> datetime:
        return self._clock()


def resolve_context(ctx: ToolCallContext, registry: InvocationRegistry) -> InvocationContext:
    """Principal attrs first; the ContextVar may only agree. Attrs must also agree with the frozen context."""
    attrs = ctx.principal.attrs
    binding_ref = attrs.get("task_binding_ref")
    if not binding_ref:
        raise ContextError(CONTEXT_MISSING)
    ic = registry.lookup(binding_ref)
    var = CURRENT_BINDING.get()
    if var is not None and var != binding_ref:
        raise ContextError(CONTEXT_MISMATCH)
    for attr, expected in (("tenant", ic.tenant_id), ("job", ic.job_id), ("stage", ic.stage)):
        if attrs.get(attr) is not None and attrs[attr] != expected:
            raise ContextError(CONTEXT_MISMATCH)
    return ic

"""`EvaluationSandboxPort` (plan 17.3.5): the stateful bank seen from the bridge.

`BrokerSandboxClient` (HTTP to `/internal/v1/broker/sandbox/*`) is a thin adapter owned by the integration
work; this module defines the port, `SandboxPortAdapter` (upstream `SandboxPort`) and
`StatefulSandboxToolExecutor` (`is_sandbox=True`). Native evaluation NEVER receives these (it uses Core's
`LocalSandbox`); see `ArmRunner` for the mixed-world rejection."""

from __future__ import annotations

import hashlib
import threading
from dataclasses import dataclass, field
from typing import Any, Protocol

from agent_core.domain import EntityRef, JsonValue, ToolDef
from agent_core.ports import IdKind, ToolCallContext, ToolResult, ToolStatus
from agent_core.registry import EvalTarget
from agent_core.registry.evaluation.ports import SandboxHandle
from agent_core.registry.suite import SandboxSeed


class SandboxTimeout(Exception):
    """Transport failure AFTER the action may have reached the bank: the effect is uncertain."""


class SandboxUnavailable(Exception):
    """The bank could not be reached BEFORE sending: no effect happened (`failed_infra`)."""


@dataclass(frozen=True)
class SandboxSession:
    session_ref: str
    revision: int
    initial_state_digest: str


@dataclass(frozen=True)
class ActionResult:
    revision: int
    effect_receipt: str
    result: JsonValue
    state_digest: str


class EvaluationSandboxPort(Protocol):
    def open(self, binding_ref: str, seed_manifest_ref: str) -> SandboxSession: ...

    def reset(self, session: SandboxSession) -> SandboxSession: ...

    def act(self, session: SandboxSession, action_key: str, expected_revision: int,
            action: dict[str, JsonValue]) -> ActionResult: ...

    def readback(self, session: SandboxSession, action_key: str) -> ActionResult | None: ...

    def close(self, session: SandboxSession, reason: str) -> str:
        """Keeps evidence; returns `final_state_ref`."""
        ...


# ToolDef id -> bank action type. Anything else is `unsupported_action` (the case is `not_evaluable`, Codex).
# NOTE: the plan names these `sandbox/transaction.lookup` etc., but the pinned `EntityRef.id` pattern is
# `^[a-z0-9][a-z0-9_/-]*$` (no dots), so a dotted id cannot exist as a ToolDef. Underscore forms are the real ids.
TOOL_ACTIONS: dict[str, str] = {
    "sandbox/transaction_lookup": "transaction.lookup",
    "sandbox/transaction_status_explain": "transaction.status_explain",
    "sandbox/case_lookup": "case.lookup", "sandbox/case_open": "case.open", "sandbox/case_note": "case.note",
    "sandbox/case_resolve": "case.resolve", "sandbox/handoff_create": "handoff.create"}
READBACK_TOOL = "sandbox/get_action"


def action_key(execution_id: str, tool_id: str, action_id: str) -> str:
    return hashlib.sha256("|".join((execution_id, tool_id, action_id)).encode()).hexdigest()


class StatefulSandboxToolExecutor:
    is_sandbox = True

    def __init__(self, port: EvaluationSandboxPort, session: SandboxSession, execution_id: str, ids: Any,
                 target: EvalTarget, receipts: list[str], unresolved: set[str] | None = None) -> None:
        self._port, self._session, self._exec, self._ids, self._target = port, session, execution_id, ids, target
        self._receipts = receipts
        self._unresolved = unresolved if unresolved is not None else set()
        self._lock = threading.Lock()
        self._revision = session.revision
        self._n = 0

    def execute(self, tool: EntityRef, args: dict[str, JsonValue], bound_params: dict[str, str],
                ctx: ToolCallContext, idempotency_key: str | None = None) -> ToolResult:
        call_id = self._ids.new_id(IdKind.call)
        with self._lock:
            if tool.id == READBACK_TOOL:
                key = str(args.get("action_key", ""))
                try:
                    got = self._port.readback(self._session, key)
                except (SandboxTimeout, SandboxUnavailable):
                    return ToolResult(status=ToolStatus.uncertain, call_id=call_id, source="sandbox",
                                      error="readback_unavailable")
                if got is None:
                    return ToolResult(status=ToolStatus.error, call_id=call_id, source="sandbox",
                                      error="no_such_action")
                self._unresolved.discard(key)
                return ToolResult(status=ToolStatus.ok, result_full=got.result, call_id=call_id, source="sandbox")
            kind = TOOL_ACTIONS.get(tool.id)
            if kind is None:
                return ToolResult(status=ToolStatus.error, call_id=call_id, source="sandbox",
                                  error="unsupported_action")
            self._n += 1
            key = action_key(self._exec, tool.id, idempotency_key or f"n{self._n}")
            try:
                done = self._port.act(self._session, key, self._revision, {"type": kind, "args": args})
            except SandboxTimeout:
                self._unresolved.add(key)
                return ToolResult(status=ToolStatus.uncertain, call_id=call_id, source="sandbox",
                                  result_full={"action_key": key})  # the engine verifies by readback
            except SandboxUnavailable:
                return ToolResult(status=ToolStatus.error, call_id=call_id, source="sandbox",
                                  error="sandbox_unavailable")
            self._revision = done.revision
            self._receipts.append(done.effect_receipt)
            return ToolResult(status=ToolStatus.ok, result_full=done.result, call_id=call_id, source="sandbox")

    def definition(self, tool: EntityRef) -> ToolDef:
        return self._target.registry.get(tool, ToolDef)


@dataclass
class SandboxPortAdapter:
    """Upstream `SandboxPort` over the bank port: `provision -> open`, `tools`, `teardown -> close`."""

    port: EvaluationSandboxPort
    binding_ref: str
    seed_manifest_ref: str
    execution_id: str
    ids: Any
    receipts: list[str] = field(default_factory=list)
    initial_state_digests: list[str] = field(default_factory=list)
    final_state_refs: list[str] = field(default_factory=list)
    unresolved: set[str] = field(default_factory=set)  # actions sent, timed out, never read back -> `unknown`
    _sessions: dict[str, tuple[SandboxSession, EvalTarget]] = field(default_factory=dict)
    _n: int = 0

    def provision(self, seed: SandboxSeed, target: EvalTarget) -> SandboxHandle:
        session = self.port.open(self.binding_ref, self.seed_manifest_ref)  # the seed lives in the manifest
        self._n += 1
        handle = SandboxHandle(f"bank-{self._n}")
        self._sessions[handle.handle_id] = (session, target)
        self.initial_state_digests.append(session.initial_state_digest)
        return handle

    def tools(self, handle: SandboxHandle) -> StatefulSandboxToolExecutor:
        session, target = self._sessions[handle.handle_id]
        return StatefulSandboxToolExecutor(self.port, session, self.execution_id, self.ids, target, self.receipts,
                                          self.unresolved)

    def teardown(self, handle: SandboxHandle) -> None:
        session, _ = self._sessions.pop(handle.handle_id)
        self.final_state_refs.append(self.port.close(session, "arm_done"))

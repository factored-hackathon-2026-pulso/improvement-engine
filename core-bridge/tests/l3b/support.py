"""Shared harness for the L3b tests: fake backend over httpx.MockTransport, doubles, builders."""

from __future__ import annotations

import itertools
from datetime import UTC, datetime, timedelta
from decimal import Decimal
from typing import Any

import httpx
from agent_core.composition.builder_tools import BUILDER_TOOL_DEFS
from agent_core.domain import EntityRef, JsonValue, ToolDef
from agent_core.domain.identity import AuthInfo, AuthLevel, Principal, PrincipalType
from agent_core.ports import ToolCallContext, ToolResult
from agent_core.ports.ids import IdKind
from agent_core.ports.llm import GenerationResult
from pulso_core_runtime.tools.broker import BrokerClient, ControlApiClient
from pulso_core_runtime.tools.context import (
    InvocationContext,
    InvocationRegistry,
    RegistryMutationCommitment,
)
from pulso_core_runtime.tools.dispatcher import PulsoToolDispatcher
from pulso_core_runtime.tools.guard import BindingGuardGateway, BindingGuardProvider

from .d3_fixture import FakeBackend

BASE = "http://control-api.test"
_counter = itertools.count(1)


def ref(tool_id: str) -> EntityRef:
    return EntityRef(id=tool_id, version="1.0.0")


class SeqIds:
    def __init__(self) -> None:
        self._n = itertools.count(1)

    def new_id(self, kind: IdKind) -> str:
        return f"{kind.value}-{next(self._n)}"

    def secret_token(self) -> str:
        return "t" * 22


class StubRegistry:
    def get(self, ref_: EntityRef, kind: type) -> ToolDef:
        if ref_.id in BUILDER_TOOL_DEFS:
            return BUILDER_TOOL_DEFS[ref_.id]
        return ToolDef.model_validate({
            "id": ref_.id, "version": "1.0.0", "risk_class": "read", "min_auth_level": "session",
            "idempotent": True, "description": "x", "args_schema": {"type": "object"}})


class RecordingInner:
    """Stands in for `BuilderToolExecutor`: records every delegated call (the registry-write counter)."""

    def __init__(self) -> None:
        self.calls: list[tuple[str, dict[str, JsonValue], str | None]] = []
        self.ids = SeqIds()

    def definition(self, tool: EntityRef) -> ToolDef:
        return BUILDER_TOOL_DEFS[tool.id]

    def execute(self, tool: EntityRef, args: dict[str, JsonValue], bound_params: dict[str, str],
                ctx: ToolCallContext, idempotency_key: str | None = None) -> ToolResult:
        self.calls.append((tool.id, dict(args), idempotency_key))
        value: JsonValue = {"proposal_id": "prop-1", "rev": 1, "state": "draft", "base_release_id": None}
        if tool.id == "registry/get_write":
            value = {"op": "put_draft", "proposal_id": "prop-1", "rev_after": 2, "request_hash": "h"}
        return ToolResult(status="ok", result_full=value, call_id=self.ids.new_id(IdKind.call))  # type: ignore[arg-type]

    @property
    def writes(self) -> list[tuple[str, dict[str, JsonValue], str | None]]:
        return [c for c in self.calls if BUILDER_TOOL_DEFS[c[0]].is_write]


class CountingGateway:
    def __init__(self) -> None:
        self.calls = 0

    def generate(self, prompt: Any, inputs_model_view: Any, locale: Any, schema: Any = None) -> GenerationResult:
        self.calls += 1
        return GenerationResult(output={"x": 1}, tokens_in=1, tokens_out=1, cost_usd=Decimal(0), model="m")


class CountingProvider:
    name = "jev"

    def __init__(self) -> None:
        self.calls = 0

    def predict(self, spec: Any, inputs_model_view: Any, schema: Any, locale: Any) -> Any:
        self.calls += 1
        return None


def principal(ic: InvocationContext, *, roles: list[str] | None = None, with_ref: bool = True) -> Principal:
    attrs = {"tenant": ic.tenant_id, "job": ic.job_id, "stage": ic.stage, "attempt": str(ic.attempt),
             "pin_release_id": "rel-1"}
    if with_ref:
        attrs["task_binding_ref"] = ic.binding_ref
    return Principal(type=PrincipalType.builder, id=f"builder:pulso-{ic.stage}:{ic.tenant_id}",
                     roles=roles if roles is not None else (["constructor"] if ic.stage == "writer" else []),
                     attrs=attrs, auth=AuthInfo(level=AuthLevel.session, at=datetime.now(UTC)),
                     exp=datetime.now(UTC) + timedelta(minutes=10))


def tcx(ic: InvocationContext, **kw: Any) -> ToolCallContext:
    return ToolCallContext(run_id="run-" + ic.binding_ref, release="rel-1", principal=principal(ic, **kw))


class AnyAdmission:
    """Double: every `evaluation_context_ref` resolves to an admission (L3b tests do not exercise L5)."""

    def get(self, ref: str) -> Any:
        from types import SimpleNamespace
        return SimpleNamespace(candidate_hash="c" * 64, suite_id="s", suite_version="1.0.0", suite_digest="d" * 64)


class Env:
    def __init__(self, evaluate_gate: Any = None) -> None:
        self.backend = FakeBackend()
        self.http = httpx.Client(transport=httpx.MockTransport(self.backend.handle), base_url=BASE)
        self.contexts = InvocationRegistry()
        self.broker = BrokerClient(BASE, lambda scope, claims: "jwt-" + scope, http=self.http, sleep=lambda s: None,
                                   identity=self._identity)
        self.control = ControlApiClient(BASE, lambda scope, claims: "jwt-" + scope, http=self.http)
        self.inner = RecordingInner()
        self.gate = evaluate_gate
        self.admissions: Any = AnyAdmission()
        self.dispatcher = PulsoToolDispatcher(
            StubRegistry(), SeqIds(), contexts=self.contexts, broker=self.broker, control=self.control,
            builder_factory=self._builder)
        self.gateway = CountingGateway()
        self.provider = CountingProvider()
        self.guard = BindingGuardGateway(self.gateway, self.contexts)
        self.provider_guard = BindingGuardProvider(self.provider, self.contexts)

    def _identity(self, ref: str) -> tuple[str, str] | None:
        try:
            ic = self.contexts.lookup(ref)
        except Exception:
            return None
        return ic.tenant_id, ic.job_id

    def _builder(self, ic: InvocationContext) -> Any:
        from pulso_core_runtime.tools.builder import ProtectedBuilderToolExecutor
        return ProtectedBuilderToolExecutor(self.inner, self.contexts, self.broker, gate=self.gate,
                                            admissions=self.admissions)

    def invocation(self, stage: str = "scout", *, tenant: str | None = None, job: str | None = None,
                   commitment: RegistryMutationCommitment | None = None, confirmed: bool = False,
                   **kw: Any) -> InvocationContext:
        n = next(_counter)
        ic = InvocationContext(
            tenant_id=tenant or f"tenant-{n}", job_id=job or f"job-{n}", stage=stage, attempt=1,
            binding_ref=f"bind-{n}", command_key=f"cmd-{n}", request_digest="r" * 64,
            bridge_instance_id="bridge-1", expires_at=datetime.now(UTC) + timedelta(minutes=15),
            commitment=commitment, **{"memory_snapshot_ref": "mem-1", "extract_manifest_ref": "extract-1", **kw})
        self.contexts.register(ic)
        if confirmed:
            self.contexts.confirm(ic.binding_ref)
        return ic

    def call(self, ic: InvocationContext, tool_id: str, args: dict[str, JsonValue] | None = None,
             key: str | None = None) -> ToolResult:
        return self.dispatcher.execute(ref(tool_id), args or {}, {}, tcx(ic), key)

    def bind(self, ic: InvocationContext) -> ToolResult:
        return self.call(ic, "pulso/bind_context")

"""`PulsoToolDispatcher`: one `ToolExecutor` for every stage (Core has no `executor_ref`).

Closed catalogue `pulso/*` (+ the protected `registry/*` writer tools). Order of gates per call:
1. unknown tool -> `error unregistered_tool`; 2. context via signed principal attrs (`denied
pulso:context_missing|context_mismatch`); 3. binding `confirmed` (every tool but `bind_context` is `denied`
before); 4. stage allow-list; 5. closed args (`additionalProperties:false`; no tenant/job/grant fields);
6. broker authorisation check; 7. handler. Handler exceptions never escape as 500s: timeout -> `timeout`,
broker errors -> `error`."""

from __future__ import annotations

from collections.abc import Callable
from typing import Any

from agent_core.composition.builder_tools import BUILDER_TOOL_DEFS
from agent_core.domain import EntityRef, JsonValue, ToolDef
from agent_core.domain.shared import ToolStatus
from agent_core.ports import ToolCallContext, ToolResult
from agent_core.ports.ids import IdKind

from pulso_core_runtime.stages.catalog import stage_allows
from pulso_core_runtime.tools import authcheck
from pulso_core_runtime.tools._common import Args, Deps, Handler, Outcome, err
from pulso_core_runtime.tools.artifacts import artifact_get
from pulso_core_runtime.tools.bind import bind_context
from pulso_core_runtime.tools.broker import (
    BrokerClient,
    BrokerError,
    BrokerTimeout,
    BrokerUnavailable,
    ControlApiClient,
)
from pulso_core_runtime.tools.context import ContextError, InvocationContext, InvocationRegistry, resolve_context
from pulso_core_runtime.tools.lab import lab_get_result, lab_query
from pulso_core_runtime.tools.wiki import wiki_explore, wiki_read, wiki_transform

BIND = "pulso/bind_context"


class _Spec:
    __slots__ = ("args", "handler", "operation", "required", "resource")

    def __init__(self, handler: Handler, args: tuple[str, ...], required: tuple[str, ...], operation: str,
                 resource: Callable[[InvocationContext, Args], list[str]]) -> None:
        self.handler, self.args, self.required, self.operation, self.resource = handler, args, required, operation, resource


# tool id -> closed argument names. No tenant/job/grant/binding fields exist anywhere in here.
CATALOGUE: dict[str, _Spec] = {
    "pulso/bind_context": _Spec(bind_context, (), (), "bind_context", lambda ic, a: [ic.binding_ref]),
    "pulso/lab_query": _Spec(lab_query, ("sql",), ("sql",), "lab_query",
                             lambda ic, a: [ic.extract_manifest_ref or ic.binding_ref]),
    "pulso/lab_get_result": _Spec(lab_get_result, ("result_ref", "cursor"), ("result_ref",), "lab_get_result",
                                  lambda ic, a: [str(a.get("result_ref"))]),
    "pulso/wiki_read": _Spec(wiki_read, ("path",), ("path",), "wiki_read",
                             lambda ic, a: [ic.memory_snapshot_ref or ic.binding_ref]),
    "pulso/wiki_explore": _Spec(wiki_explore, ("query",), ("query",), "wiki_explore",
                                lambda ic, a: [ic.memory_snapshot_ref or ic.binding_ref]),
    "pulso/wiki_transform": _Spec(wiki_transform, ("source_ref", "transform"), ("source_ref", "transform"),
                                  "wiki_transform", lambda ic, a: [ic.memory_snapshot_ref or ic.binding_ref]),
    "pulso/artifact_get": _Spec(artifact_get, ("artifact_ref",), ("artifact_ref",), "artifact_get",
                                lambda ic, a: [str(a.get("artifact_ref"))]),
}
PULSO_TOOLS = frozenset(CATALOGUE)
BuilderFactory = Callable[[InvocationContext], Any]  # -> ProtectedBuilderToolExecutor


class PulsoToolDispatcher:
    def __init__(self, registry: Any, ids: Any, *, contexts: InvocationRegistry, broker: BrokerClient,
                 control: ControlApiClient, builder_factory: BuilderFactory | None = None,
                 leak_signal: Callable[[str, str], None] | None = None) -> None:
        self._registry, self._ids = registry, ids
        self._contexts, self._broker = contexts, broker
        self._deps = Deps(contexts, broker, control)
        if leak_signal is not None:
            self._deps.leak_signal = leak_signal
        self._builder_factory = builder_factory

    @property
    def deps(self) -> Deps:
        return self._deps

    def set_builder_factory(self, factory: BuilderFactory) -> None:
        self._builder_factory = factory

    # -- ToolExecutor ------------------------------------------------------------------------------
    def definition(self, tool: EntityRef) -> ToolDef:
        result: ToolDef = self._registry.get(tool, ToolDef)
        return result

    def execute(self, tool: EntityRef, args: dict[str, JsonValue], bound_params: dict[str, str],
                ctx: ToolCallContext, idempotency_key: str | None = None) -> ToolResult:
        status, value, error = self._run(tool, args, ctx, idempotency_key)
        if isinstance(status, ToolResult):  # builder path returns a finished result
            return status
        return ToolResult(status=status, result_full=value, error=error, call_id=self._ids.new_id(IdKind.call))

    def _run(self, tool: EntityRef, args: Args, ctx: ToolCallContext,
             key: str | None) -> tuple[ToolStatus | ToolResult, JsonValue, str | None]:
        is_pulso = tool.id in CATALOGUE and _is_v1(tool.version)
        is_registry = tool.id in BUILDER_TOOL_DEFS and _is_v1(tool.version)
        if not (is_pulso or is_registry):
            return err("unregistered_tool")
        try:
            ic = resolve_context(ctx, self._contexts)
        except ContextError as exc:
            return err(exc.code, ToolStatus.denied)
        if tool.id != BIND and not self._contexts.is_confirmed(ic.binding_ref):
            return err("pulso:binding_unconfirmed", ToolStatus.denied)
        if not stage_allows(ic.stage, tool.id + "@" + tool.version):
            return err("tool_not_allowed", ToolStatus.denied)
        if is_registry:
            return self._registry_tool(tool, args, ctx, key, ic)
        spec = CATALOGUE[tool.id]
        if not set(args) <= set(spec.args) or not set(spec.required) <= set(args):
            return err("invalid_args", ToolStatus.denied)
        if tool.id != BIND:
            try:
                resources = spec.resource(ic, args)
            except Exception:  # noqa: BLE001
                return err("invalid_args", ToolStatus.denied)
            denied = authcheck.check(self._contexts, self._broker, ic, spec.operation, resources)
            if denied:
                return err(denied, ToolStatus.denied)
        return self._invoke(spec.handler, ic, args, ctx.run_id)

    def _invoke(self, handler: Handler, ic: InvocationContext, args: Args, run_id: str) -> Outcome:
        try:
            return handler(self._deps, ic, args, run_id)
        except ValueError:  # malformed args (empty/non-string): no effect happened
            return err("invalid_args", ToolStatus.denied)
        except BrokerTimeout:
            return ToolStatus.timeout, None, "pulso:broker_timeout"
        except BrokerUnavailable:
            return err("pulso:broker_unavailable")
        except BrokerError as exc:
            return err(f"pulso:broker_{exc.status}:{exc.code}")
        except Exception:  # noqa: BLE001 - never let a handler bug become an engine crash
            return err("pulso:tool_internal_error")

    def _registry_tool(self, tool: EntityRef, args: Args, ctx: ToolCallContext, key: str | None,
                       ic: InvocationContext) -> tuple[ToolStatus | ToolResult, JsonValue, str | None]:
        if self._builder_factory is None:
            return err("unregistered_tool")
        executor = self._builder_factory(ic)
        finished: ToolResult = executor.execute(tool, args, {}, ctx, key)
        return finished, None, None


def _is_v1(version: str) -> bool:
    return version.split(".")[0] == "1"


__all__ = ["CATALOGUE", "PULSO_TOOLS", "PulsoToolDispatcher"]

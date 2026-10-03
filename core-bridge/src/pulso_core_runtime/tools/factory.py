"""Wiring helpers for L2/L3a (L3b exposes factories; it never touches `app.py`).

Wiring (done by the owners of those files):
  * `factories.tools` (L2) should become `from pulso_core_runtime.tools.factory import tools`; until
    `configure(...)` is called the dispatcher is fail-closed (empty registry -> `denied context_missing`).
  * `main` (after the `RegistryService` exists) calls
    `configure(ToolRuntime(contexts, broker, control, builder_factory=protected_builder_factory(...)))`,
    wraps the LLM gateway/Jev provider with `BindingGuardGateway`/`BindingGuardProvider(contexts)`, and the
    invoke service registers/removes `InvocationContext`s in the SAME `InvocationRegistry`."""

from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from typing import Any

from agent_core.composition.builder_tools import BuilderToolExecutor
from agent_core.domain.identity import Principal

from pulso_core_runtime.tools.broker import BrokerClient, ControlApiClient
from pulso_core_runtime.tools.builder import EvaluationGate, ProtectedBuilderToolExecutor
from pulso_core_runtime.tools.context import InvocationContext, InvocationRegistry
from pulso_core_runtime.tools.dispatcher import BuilderFactory, PulsoToolDispatcher


@dataclass
class ToolRuntime:
    contexts: InvocationRegistry
    broker: BrokerClient
    control: ControlApiClient
    builder_factory: BuilderFactory | None = None
    leak_signal: Callable[[str, str], None] | None = None


_RUNTIME: ToolRuntime | None = None


def configure(runtime: ToolRuntime) -> None:
    global _RUNTIME
    _RUNTIME = runtime


def current_runtime() -> ToolRuntime | None:
    return _RUNTIME


def protected_builder_factory(service: Any, actor_for: Callable[[InvocationContext], Principal], ids: Any,
                              contexts: InvocationRegistry, broker: BrokerClient,
                              gate: EvaluationGate | None = None) -> BuilderFactory:
    """`actor_for(ic)` returns the bot constructor principal (own credential, never the run's principal)."""

    def factory(ic: InvocationContext) -> ProtectedBuilderToolExecutor:
        return ProtectedBuilderToolExecutor(BuilderToolExecutor(service, actor_for(ic), ids), contexts, broker,
                                            gate=gate, ids=ids)

    return factory


def make_dispatcher(registry: Any, ids: Any, runtime: ToolRuntime) -> PulsoToolDispatcher:
    return PulsoToolDispatcher(registry, ids, contexts=runtime.contexts, broker=runtime.broker,
                               control=runtime.control, builder_factory=runtime.builder_factory,
                               leak_signal=runtime.leak_signal)


def tools(ctx: Any) -> PulsoToolDispatcher:
    """`--tools` factory (`fn(DemoContext)`)."""
    runtime = _RUNTIME
    if runtime is None:  # fail-closed placeholder: nothing is registered, every tool is denied
        contexts = InvocationRegistry()
        runtime = ToolRuntime(contexts, BrokerClient("http://unconfigured.invalid", lambda s: ""),
                              ControlApiClient("http://unconfigured.invalid", lambda s: ""))
    return make_dispatcher(ctx.registry, ctx.ids, runtime)


def register(app: Any, deps: Any) -> ToolRuntime:
    """`register(app, deps)`-style entry: `deps` carries `contexts`, `broker`, `control` (+ optional
    `builder_factory`, `leak_signal`). Returns and installs the process-wide runtime."""
    runtime = ToolRuntime(deps.contexts, deps.broker, deps.control, getattr(deps, "builder_factory", None),
                          getattr(deps, "leak_signal", None))
    configure(runtime)
    return runtime

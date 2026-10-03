"""`BindingGuardGateway` / `BindingGuardProvider` (plan 17.3.3 "Binding"): wrap the LLM gateway and the Jev
decision provider so an Agent node running before `bind_context` has confirmed the binding cannot spend model
budget. Fail-closed: no current binding, an unknown/expired one, or one that is not `confirmed` -> refuse before
the inner call (zero tokens, zero cost).

The gateway/provider ports carry no principal, so the only available channel here is the ContextVar set by the
invoke service around the in-process call (`use_binding`); absence is a refusal, never a pass-through."""

from __future__ import annotations

from collections.abc import Callable
from typing import Any

from agent_core.domain.base import Locale
from agent_core.domain.errors import GatewayError, GatewayErrorKind
from agent_core.domain.json import JsonValue
from agent_core.domain.refs import EntityRef
from agent_core.ports.llm import GenerationResult

from pulso_core_runtime.tools.context import InvocationRegistry, current_binding


class BindingNotConfirmed(Exception):
    """Raised by the provider guard (also an acceptable `ProviderError` stand-in for callers)."""

    code = "pulso:binding_unconfirmed"

    def __init__(self) -> None:
        super().__init__("pulso:binding_unconfirmed: binding not confirmed")


class _Guard:
    def __init__(self, contexts: InvocationRegistry, current: Callable[[], str | None]) -> None:
        self._contexts, self._current = contexts, current

    def _confirmed(self) -> bool:
        return self._contexts.is_confirmed(self._current())


class BindingGuardGateway(_Guard):
    def __init__(self, inner: Any, contexts: InvocationRegistry,
                 current: Callable[[], str | None] = current_binding) -> None:
        super().__init__(contexts, current)
        self._inner = inner

    def generate(self, prompt: EntityRef, inputs_model_view: dict[str, JsonValue], locale: Locale,
                 schema: dict[str, JsonValue] | None = None) -> GenerationResult:
        if not self._confirmed():
            raise GatewayError(GatewayErrorKind.refused)
        result: GenerationResult = self._inner.generate(prompt, inputs_model_view, locale, schema)
        return result


class BindingGuardProvider(_Guard):
    def __init__(self, inner: Any, contexts: InvocationRegistry,
                 current: Callable[[], str | None] = current_binding) -> None:
        super().__init__(contexts, current)
        self._inner = inner
        self.name = inner.name

    def predict(self, spec: Any, inputs_model_view: dict[str, JsonValue], schema: dict[str, JsonValue],
                locale: Locale) -> Any:
        if not self._confirmed():
            from agent_core.decision.types import ProviderError
            raise ProviderError(BindingNotConfirmed.code)
        return self._inner.predict(spec, inputs_model_view, schema, locale)

"""Context channel: one `InvocationRegistry` shared with the L3b tools (`tools/context.py`). The invoke service
registers the frozen context under the signed `task_binding_ref`; `ConfirmingRegistry` mirrors the tool-side
binding confirmation into the receipt CAS (`sent -> binding_confirmed`)."""

from __future__ import annotations

from typing import Any

from pulso_core_runtime.tools.context import (
    ContextError,
    InvocationContext,
    InvocationRegistry,
    current_binding,
    use_binding,
)

__all__ = ["ConfirmingRegistry", "ContextError", "InvocationContext", "InvocationRegistry", "current_binding",
           "use_binding"]


class ConfirmingRegistry(InvocationRegistry):
    def __init__(self, store: Any, **kw: Any) -> None:
        super().__init__(**kw)
        self._receipts = store

    def confirm(self, binding_ref: str) -> None:
        ctx = self.lookup(binding_ref)
        super().confirm(binding_ref)
        self._receipts.transition(ctx.tenant_id, ctx.command_key, "binding_confirmed")

    def deny(self, binding_ref: str) -> None:
        super().deny(binding_ref)

    def binding_unproven(self, binding_ref: str) -> None:
        """Timeout / network failure / 5xx on the binding callback: the control-api may have recorded it."""
        try:
            ctx = self.lookup(binding_ref)
        except ContextError:
            return
        self._receipts.transition(ctx.tenant_id, ctx.command_key, "manual_reconcile", reason="binding_unproven")

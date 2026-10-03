"""Both binding paths (tools/bind.py live path and invoke/binding.py BindingService) must treat an unproven callback
(timeout, network failure, 5xx) the same way: the receipt moves to `manual_reconcile` (`binding_unproven`);
a definite refusal (404/409) never does."""

from __future__ import annotations

import pytest
from agent_core.domain.shared import ToolStatus

from pulso_core_runtime.invoke.context import ConfirmingRegistry

from .support import Env


class Store:
    def __init__(self) -> None:
        self.moves: list[tuple[str, str, str, str | None]] = []

    def transition(self, tenant: str, key: str, to: str, **kw: object) -> None:
        self.moves.append((tenant, key, to, str(kw.get("reason")) if kw.get("reason") else None))


@pytest.mark.parametrize(("mode", "unproven"), [("timeout", True), ("unavailable", True),
                                                 ("conflict", False), ("not_found", False)])
def test_tool_binding_path_marks_manual_reconcile_only_when_unproven(mode: str, unproven: bool) -> None:
    store = Store()
    env = Env(contexts=ConfirmingRegistry(store))
    ic = env.invocation("scout")
    env.backend.bind_mode = mode
    assert env.bind(ic).status is ToolStatus.denied
    moves = [m for m in store.moves if m[2] == "manual_reconcile"]
    assert bool(moves) is unproven
    if unproven:
        assert moves == [(ic.tenant_id, ic.command_key, "manual_reconcile", "binding_unproven")]

"""Tool double for the LOCAL battery core: agent-core demo tools plus per-scenario SEEDED replies.

Loaded by `agentcore serve --tools battery_tools:tools` (PYTHONPATH carries scripts/battery; only with
AGENTCORE_ALLOW_DEMO=1, local use). It mirrors the `seed.tools` semantics of agent-core `evaluate` (scripted tool
replies, consumed in order per run, the last one repeats): the runner writes `seeds.json`
({principal_id: {tool_id: [{"result": ...}, ...]}}) into BATTERY_SEED_FILE and the double picks the replies by the
scenario principal id, so a live /v1/runs conversation sees exactly the data of the scripted scenario. A tool with no
seed falls back to the agent-core demo handlers. Nothing here is imported by the engine or by production code.
"""
from __future__ import annotations

import json
import os
from decimal import Decimal
from pathlib import Path

from agent_core.composition.serve_ports import DemoContext
from agent_core.domain import EntityRef, ToolDef
from agent_core.ports import ToolCallContext, ToolResult, ToolStatus
from testing.e2e_demo import _HANDLERS, E2ETools
from testing.engine_world import _radicar
from testing.fakes.tools import Scripted

_cache: dict[str, object] = {"mtime": None, "data": {}}


def _seeds() -> dict:
    path = Path(os.environ.get("BATTERY_SEED_FILE", ""))
    if not path.is_file():
        return {}
    mtime = path.stat().st_mtime_ns
    if _cache["mtime"] != mtime:
        _cache["data"] = json.loads(path.read_text(encoding="utf-8"), parse_float=Decimal)
        _cache["mtime"] = mtime
    return _cache["data"]  # type: ignore[return-value]


class BatteryTools(E2ETools):
    """Seeded replies are pushed on the FakeToolExecutor script queue (or returned directly for read-backs)."""

    def __init__(self, ctx: DemoContext) -> None:
        super().__init__(ctx)
        self._counts: dict[tuple[str, str], int] = {}

    def _ensure(self, tool: EntityRef) -> None:
        """Same lazy registration as E2ETools.execute, without executing anything."""
        if tool in self._seen:
            return
        definition = self._registry.get(tool, ToolDef)
        if tool.id == "obtener_pqr":
            self._inner.register_readback(definition, of=EntityRef(id="radicar_pqr", version=tool.version))
        else:
            self._inner.register(definition, handler=_HANDLERS.get(tool.id, _radicar))
        self._seen.add(tool)

    def execute(self, tool: EntityRef, args, bound_params, ctx: ToolCallContext, idempotency_key=None) -> ToolResult:
        replies = _seeds().get(ctx.principal.id, {}).get(tool.id)
        if not replies:
            return super().execute(tool, args, bound_params, ctx, idempotency_key)
        self._ensure(tool)
        key = (ctx.run_id, tool.id)
        index = self._counts.get(key, 0)
        self._counts[key] = index + 1
        reply = replies[min(index, len(replies) - 1)]
        scripted = Scripted(ToolStatus(reply.get("status", "ok")), result=reply.get("result"), error=reply.get("error"))
        inner = self._inner
        if tool in inner._readbacks:
            return inner._result(tool, args, bound_params, idempotency_key, inner._defs[tool], scripted)
        inner._scripts[tool].append(scripted)
        return inner.execute(tool, args, bound_params, ctx, idempotency_key)


def tools(ctx: DemoContext) -> BatteryTools:
    return BatteryTools(ctx)

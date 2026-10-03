"""Factory wiring (fail-closed default) and ToolDef <-> handler CI check against agent-core-assets."""

from __future__ import annotations

from pathlib import Path
from types import SimpleNamespace

import yaml
from agent_core.domain.shared import ToolStatus

from pulso_core_runtime.tools import factory
from pulso_core_runtime.tools.dispatcher import PULSO_TOOLS

from .support import Env, SeqIds, StubRegistry, ref, tcx

TOOLS = Path(__file__).resolve().parents[3] / "agent-core-assets" / "worlds" / "pulso-evolution" / "tools" / "pulso"


def test_every_pulso_tooldef_has_a_handler_and_extras_are_explicit() -> None:
    defs = {yaml.safe_load(f.read_text(encoding="utf-8"))["id"] for f in TOOLS.glob("*.yaml")}
    assert defs <= PULSO_TOOLS, f"ToolDef without handler: {defs - PULSO_TOOLS}"
    # handler without ToolDef: lab_get_result is folded into lab_query by the assets (plan allows it);
    # it stays unreachable because no stage allow-list names it.
    assert PULSO_TOOLS - defs == {"pulso/lab_get_result"}
    from pulso_core_runtime.stages.catalog import CATALOG
    assert not any("lab_get_result" in t for s in CATALOG.values() for t in s.tools_allowed)


def test_tools_factory_is_fail_closed_until_configured() -> None:
    factory.configure(None)  # type: ignore[arg-type]
    ctx = SimpleNamespace(registry=StubRegistry(), ids=SeqIds())
    d = factory.tools(ctx)
    env = Env()
    ic = env.invocation("scout", confirmed=True)  # registered elsewhere: this dispatcher knows nothing
    r = d.execute(ref("pulso/wiki_read"), {"path": "a"}, {}, tcx(ic))
    assert r.status is ToolStatus.denied and r.error == "pulso:context_missing"


def test_register_installs_the_shared_runtime() -> None:
    env = Env()
    rt = factory.register(None, SimpleNamespace(contexts=env.contexts, broker=env.broker, control=env.control))
    assert factory.current_runtime() is rt
    d = factory.tools(SimpleNamespace(registry=StubRegistry(), ids=SeqIds()))
    ic = env.invocation("scout")
    assert d.execute(ref("pulso/bind_context"), {}, {}, tcx(ic)).status is ToolStatus.ok
    factory.configure(None)  # type: ignore[arg-type]

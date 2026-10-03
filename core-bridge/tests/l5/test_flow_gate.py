"""FlowEvaluationGate against L3b's EvaluationGate protocol shape."""

from __future__ import annotations

from types import SimpleNamespace

import pytest
from agent_core.domain.shared import ToolStatus

from l5.test_evaluate_path import World, suite_draft
from l5.world import bot_actor
from pulso_core_runtime.registry_service import FlowEvaluationGate
from pulso_core_runtime.tools.builder import eval_key

pytestmark = [pytest.mark.runtime, pytest.mark.pg]


def _ic(ref: str, enabled: bool = True) -> SimpleNamespace:
    c = SimpleNamespace(evaluate_enabled=enabled, evaluation_context_ref=ref)
    return SimpleNamespace(tenant_id="t1", job_id="job-1", binding_ref="bind-1", commitment=c,
                           evaluation_context_ref=ref)


def test_pass_fail_denied_through_the_l3b_protocol(pg) -> None:  # type: ignore[no-untyped-def]
    w = World(pg)
    gate = FlowEvaluationGate(w.rt, lambda ic: bot_actor())
    pid, chash = w.frozen_proposal()
    w.admit(pid, chash)
    st, res, err = gate.evaluate(_ic("ctx-1"), pid, eval_key("ctx-1"), (None, None))
    assert st is ToolStatus.ok and res["verdict"] == "pass" and err is None and res["report_digest"]
    st, res, err = gate.evaluate(_ic("ctx-1"), pid, eval_key("ctx-1"), (None, None))  # replay
    assert st is ToolStatus.ok and w.storage.jobs == 1
    st, _, err = gate.evaluate(_ic("ctx-none"), pid, eval_key("ctx-none"), (None, None))
    assert st is ToolStatus.denied and err == "pulso:admission_missing"
    st, _, err = gate.evaluate(_ic("ctx-1"), pid, "pulso-eval:other", (None, None))
    assert st is ToolStatus.denied and err == "pulso:commitment_mismatch"
    st, _, err = gate.evaluate(_ic("ctx-1", enabled=False), pid, eval_key("ctx-1"), (None, None))
    assert st is ToolStatus.denied and err == "pulso:evaluate_disabled"
    _ = suite_draft


def test_gate_failed_maps_to_error_with_report_refs(pg) -> None:  # type: ignore[no-untyped-def]
    w = World(pg)
    gate = FlowEvaluationGate(w.rt, lambda ic: bot_actor())
    pid, chash = w.frozen_proposal(suite_draft(outcome="escalated"))
    w.admit(pid, chash, "ctx-f")
    st, res, err = gate.evaluate(_ic("ctx-f"), pid, eval_key("ctx-f"), (None, None))
    assert st is ToolStatus.error and err == "gate_failed" and res["verdict"] == "fail"
    assert res["report_digest"] == w.rt.reports.list_for(pid)[0].report_digest

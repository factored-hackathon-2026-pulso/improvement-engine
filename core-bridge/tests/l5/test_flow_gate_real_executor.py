"""FlowEvaluationGate behind the REAL L3b ProtectedBuilderToolExecutor (via the pulso/* dispatcher)."""

from __future__ import annotations

from datetime import timedelta
from typing import Any

import pytest
from agent_core.domain.shared import ToolStatus

from l3b.support import Env
from l5.test_evaluate_path import World
from l5.world import bot_actor
from pulso_core_runtime.evaluation.admission import Admission
from pulso_core_runtime.registry_service import FlowEvaluationGate
from pulso_core_runtime.tools.context import RegistryMutationCommitment as C

pytestmark = [pytest.mark.runtime, pytest.mark.pg]


def _setup(pg: Any, ref: str = "ctx-real") -> tuple[World, Env, Any, str]:
    w = World(pg)
    pid, chash = w.frozen_proposal()
    env = Env(evaluate_gate=FlowEvaluationGate(w.rt, lambda ic: bot_actor()))
    commitment = C(mode="evaluate_only", proposal_id=pid, expected_rev=None, base_release_id="rel-0",
                   evaluate_enabled=True, evaluation_context_ref=ref, create_agent_id=None, create_origin=None,
                   create_title=None, put_draft_digest=None)
    ic = env.invocation("writer", commitment=commitment)
    assert env.bind(ic).status is ToolStatus.ok
    digest = w.rt._suite_digest(pid, "disputas-suite", "1.0.0") or "x"
    w.rt.admissions.create(Admission(ref, ic.tenant_id, ic.job_id, ic.binding_ref, pid, chash, "disputas-suite",
                                     "1.0.0", digest, 1, "bud-1", w.clock_now + timedelta(hours=1), "d" * 64))
    return w, env, ic, pid


def test_real_executor_evaluates_once_and_replays(pg) -> None:  # type: ignore[no-untyped-def]
    w, env, ic, pid = _setup(pg)
    r = env.call(ic, "registry/evaluate", {"proposal_id": pid}, key="eng-1")
    assert r.status is ToolStatus.ok, r.error
    assert r.result_full["verdict"] == "pass" and r.result_full["report_digest"]  # type: ignore[index]
    r2 = env.call(ic, "registry/evaluate", {"proposal_id": pid}, key="eng-2")
    assert r2.status is ToolStatus.ok and w.storage.jobs == 1
    assert env.inner.calls == []  # never delegated to BuilderToolExecutor._evaluate


def test_real_executor_tool_args_cannot_pick_another_admission(pg) -> None:  # type: ignore[no-untyped-def]
    w, env, ic, pid = _setup(pg)
    r = env.call(ic, "registry/evaluate", {"proposal_id": pid, "evaluation_context_ref": "other"}, key="e")
    assert r.status is ToolStatus.denied and r.error == "invalid_args" and w.storage.jobs == 0


def test_newline_terminated_ref_is_denied_not_an_exception(pg) -> None:  # type: ignore[no-untyped-def]
    """L3b's regex uses `$` (accepts a trailing newline); L5 must deny, never crash into `uncertain`."""
    w, env, ic, pid = _setup(pg, ref="ctx-nl\n")
    r = env.call(ic, "registry/evaluate", {"proposal_id": pid}, key="e")
    assert r.status is ToolStatus.denied, (r.status, r.error)
    assert w.storage.jobs == 0

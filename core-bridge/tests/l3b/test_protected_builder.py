"""ProtectedBuilderToolExecutor: allow-list, commitment, key derivation, evaluate gating."""

from __future__ import annotations

import hashlib
from types import SimpleNamespace
from typing import Any

import pytest
from agent_core.composition.builder_tools import BuilderToolExecutor
from agent_core.domain.shared import ToolStatus

from pulso_core_runtime.tools.builder import (
    ProtectedBuilderToolExecutor,
    eval_key,
    put_draft_digest,
    write_key,
)
from pulso_core_runtime.tools.context import RegistryMutationCommitment as C

from .support import Env, SeqIds, ref, tcx

CHANGES = [{"kind": "flow", "content": {"id": "x"}, "docs": {"description": "d", "rationale": "r", "changelog": "c"}}]
TITLE = "Improve X pulso-key:abc"


class Gate:
    def __init__(self) -> None:
        self.calls: list[tuple[str, str, tuple[Any, Any]]] = []

    def evaluate(self, ic: Any, proposal_id: str, idempotency_key: str, requested_suite: Any) -> Any:
        self.calls.append((proposal_id, idempotency_key, requested_suite))
        return ToolStatus.ok, {"verdict": "pass"}, None


def commitment(**kw: Any) -> C:
    base: dict[str, Any] = {"mode": "write", "proposal_id": None, "expected_rev": None, "base_release_id": "rel-0",
                                "evaluate_enabled": False, "create_agent_id": "agent-1", "create_origin": "builder_chat",
                                "create_title": TITLE, "put_draft_digest": put_draft_digest(None, None, CHANGES)}
    base.update(kw)
    return C(**base)


def writer(c: C | None = None, gate: Gate | None = None) -> tuple[Env, Any]:
    env = Env(evaluate_gate=gate)
    ic = env.invocation("writer", commitment=c or commitment())
    assert env.bind(ic).status is ToolStatus.ok
    return env, ic


CREATE = {"agent_id": "agent-1", "origin": "builder_chat", "title": TITLE}


def test_derived_write_keys_follow_the_formula_with_stable_ordinals() -> None:
    env, ic = writer()
    r = env.call(ic, "registry/create_proposal", CREATE, key="engine-1")
    assert r.status is ToolStatus.ok
    k0 = write_key(ic.command_key, "writer", 0)
    assert k0 == "pulso-w:" + hashlib.sha256(f"{ic.command_key}|writer|0".encode()).hexdigest()[:48]
    assert env.inner.calls[-1][2] == k0
    assert r.result_full["key_digest"] == hashlib.sha256(k0.encode()).hexdigest()
    put = {"proposal_id": "prop-1", "expected_rev": 1, "changes": CHANGES}
    # fresh proposal: put_draft digest is bound with null id/rev; the id must be the one created here
    assert env.call(ic, "registry/put_draft", put, key="engine-2").status is ToolStatus.ok
    assert env.inner.calls[-1][2] == write_key(ic.command_key, "writer", 1)
    env.call(ic, "registry/create_proposal", CREATE, key="engine-1")  # replay of the same engine key
    assert env.inner.calls[-1][2] == k0
    # get_write translates the engine's key to the derived one
    env.call(ic, "registry/get_write", {"idempotency_key": "engine-2"})
    assert env.inner.calls[-1][1]["idempotency_key"] == write_key(ic.command_key, "writer", 1)
    assert len(k0) <= 255 and len(eval_key("ref-1")) <= 255


def test_eval_key_bounds() -> None:
    assert eval_key("a" * 200) == "pulso-eval:" + "a" * 200
    for bad in ("a" * 201, "has space", "ñ", ""):
        with pytest.raises(ValueError):
            eval_key(bad)


@pytest.mark.parametrize("name,args", [
    ("registry/create_proposal", {**CREATE, "title": "other"}),
    ("registry/create_proposal", {**CREATE, "agent_id": "agent-2"}),
    ("registry/create_proposal", {**CREATE, "origin": "auto_detect"}),
])
def test_altered_create_is_commitment_mismatch_with_zero_effect(name: str, args: dict[str, Any]) -> None:
    env, ic = writer()
    r = env.call(ic, name, args, key="e1")
    assert r.status is ToolStatus.denied and r.error == "commitment_mismatch"
    assert env.inner.calls == []


def test_altered_put_draft_or_foreign_proposal_is_mismatch() -> None:
    env, ic = writer()
    env.call(ic, "registry/create_proposal", CREATE, key="e1")
    n = len(env.inner.calls)
    altered = [{**CHANGES[0], "content": {"id": "EVIL"}}]
    r = env.call(ic, "registry/put_draft", {"proposal_id": "prop-1", "expected_rev": 1, "changes": altered}, key="e2")
    assert r.error == "commitment_mismatch"
    r = env.call(ic, "registry/put_draft", {"proposal_id": "prop-OTHER", "expected_rev": 1, "changes": CHANGES}, key="e3")
    assert r.error == "commitment_mismatch"
    assert env.call(ic, "registry/freeze", {"proposal_id": "prop-OTHER"}, key="e4").error == "commitment_mismatch"
    assert len(env.inner.calls) == n


def test_existing_proposal_commitment_binds_id_rev_and_digest() -> None:
    digest = put_draft_digest("prop-9", 4, CHANGES)
    env, ic = writer(commitment(proposal_id="prop-9", expected_rev=4, put_draft_digest=digest, create_title=None))
    assert env.call(ic, "registry/create_proposal", CREATE, key="e0").error == "commitment_mismatch"
    ok = {"proposal_id": "prop-9", "expected_rev": 4, "changes": CHANGES}
    assert env.call(ic, "registry/put_draft", {**ok, "expected_rev": 5}, key="e1").error == "commitment_mismatch"
    assert env.call(ic, "registry/put_draft", ok, key="e2").status is ToolStatus.ok
    for tool in ("registry/validate", "registry/freeze", "registry/get_proposal"):
        assert env.call(ic, tool, {"proposal_id": "prop-9"}, key="k-" + tool).status is ToolStatus.ok, tool
    assert env.call(ic, "registry/reopen", {"proposal_id": "prop-9"}, key="kr").status is ToolStatus.ok


def test_evaluate_requires_flag_ref_gate_and_authorisation_and_never_hits_the_inner_executor() -> None:
    gate = Gate()
    env, ic = writer(commitment(proposal_id="prop-9", create_title=None, evaluate_enabled=False), gate)
    assert env.call(ic, "registry/evaluate", {"proposal_id": "prop-9"}, key="e").error == "tool_not_allowed"
    assert gate.calls == []

    gate = Gate()
    env, ic = writer(commitment(proposal_id="prop-9", create_title=None, evaluate_enabled=True,
                                evaluation_context_ref="evalctx-1"), gate)
    r = env.call(ic, "registry/evaluate", {"proposal_id": "prop-9", "suite_id": "s1", "suite_version": "1.0.0"}, key="eng")
    assert r.status is ToolStatus.ok
    assert gate.calls == [("prop-9", "pulso-eval:evalctx-1", ("s1", "1.0.0"))]
    assert env.inner.calls == []  # never delegated to BuilderToolExecutor._evaluate
    assert any(b and b.get("operation") == "native_evaluate" and b["payload_digest"] for b in env.backend.bodies)
    # get_write on the engine key is translated to the evaluation key
    env.call(ic, "registry/get_write", {"idempotency_key": "eng"})
    assert env.inner.calls[-1][1]["idempotency_key"] == "pulso-eval:evalctx-1"
    # tool args cannot select another proposal
    assert env.call(ic, "registry/evaluate", {"proposal_id": "prop-X"}, key="e2").error == "commitment_mismatch"


def test_evaluate_fail_closed_cases() -> None:
    env, ic = writer(commitment(proposal_id="p", create_title=None, evaluate_enabled=True,
                                evaluation_context_ref="r1"), gate=None)
    assert env.call(ic, "registry/evaluate", {"proposal_id": "p"}, key="e").error == "pulso:evaluation_gate_unavailable"
    gate = Gate()
    env, ic = writer(commitment(proposal_id="p", create_title=None, evaluate_enabled=True,
                                evaluation_context_ref="bad ref"), gate)
    assert env.call(ic, "registry/evaluate", {"proposal_id": "p"}, key="e").error == "pulso:evaluation_context_ref_invalid"
    gate = Gate()
    env, ic = writer(commitment(proposal_id="p", create_title=None, evaluate_enabled=True,
                                evaluation_context_ref="r2"), gate)
    env.backend.auth_allowed = False
    assert env.call(ic, "registry/evaluate", {"proposal_id": "p"}, key="e").status is ToolStatus.denied
    assert gate.calls == []


def test_evaluate_only_invocation_denies_every_other_mutator() -> None:
    gate = Gate()
    env, ic = writer(commitment(mode="evaluate_only", proposal_id="p", create_title=None, evaluate_enabled=True,
                                evaluation_context_ref="r3"), gate)
    for tool, args in (("registry/create_proposal", CREATE), ("registry/put_draft", {"proposal_id": "p", "expected_rev": 1, "changes": CHANGES}),
                       ("registry/freeze", {"proposal_id": "p"}), ("registry/reopen", {"proposal_id": "p"})):
        assert env.call(ic, tool, args, key="k-" + tool).error == "tool_not_allowed", tool
    assert env.inner.writes == []
    assert env.call(ic, "registry/get_proposal", {"proposal_id": "p"}).status is ToolStatus.ok
    assert env.call(ic, "registry/evaluate", {"proposal_id": "p"}, key="k").status is ToolStatus.ok


def test_reads_tools_outside_the_allow_list_and_other_stages_are_denied() -> None:
    env, ic = writer()
    assert env.call(ic, "registry/get_entity", {"kind": "flow", "entity_id": "x"}).error == "tool_not_allowed"
    env2 = Env()
    other = env2.invocation("scout", commitment=commitment())
    env2.bind(other)
    assert env2.call(other, "registry/create_proposal", CREATE, key="k").error == "tool_not_allowed"
    assert env2.inner.calls == []
    # the executor stands alone even if the dispatcher is bypassed
    direct = ProtectedBuilderToolExecutor(env2.inner, env2.contexts, env2.broker)
    assert direct.execute(ref("registry/create_proposal"), CREATE, {}, tcx(other), "k").error == "tool_not_allowed"
    assert direct.execute(ref("registry/approve"), {}, {}, tcx(other), "k").error == "unregistered_tool"


def test_unconfirmed_binding_or_authorisation_failure_means_zero_effect() -> None:
    env = Env()
    ic = env.invocation("writer", commitment=commitment())
    direct = ProtectedBuilderToolExecutor(env.inner, env.contexts, env.broker)
    assert direct.execute(ref("registry/create_proposal"), CREATE, {}, tcx(ic), "k").error == "pulso:binding_unconfirmed"
    env.bind(ic)
    env.backend.auth_mode = "5xx"
    assert env.call(ic, "registry/create_proposal", CREATE, key="k").status is ToolStatus.denied
    assert env.inner.calls == []


def test_wraps_the_real_builder_executor_and_delivers_the_derived_key() -> None:
    seen: dict[str, Any] = {}

    class Service:
        def create_proposal(self, actor: Any, agent_id: str, origin: Any, title: str, idempotency_key: str | None = None,
                            audit: Any = None) -> Any:
            seen.update(key=idempotency_key, agent=agent_id)
            return SimpleNamespace(proposal_id="prop-1", rev=1, state=SimpleNamespace(value="draft"), base_release_id=None)

    env = Env()
    ic = env.invocation("writer", commitment=commitment())
    env.bind(ic)
    real = BuilderToolExecutor(Service(), SimpleNamespace(id="bot"), SeqIds())  # type: ignore[arg-type]
    ex = ProtectedBuilderToolExecutor(real, env.contexts, env.broker, ids=SeqIds())
    r = ex.execute(ref("registry/create_proposal"), CREATE, {}, tcx(ic), "engine-key")
    assert r.status is ToolStatus.ok and seen["key"] == write_key(ic.command_key, "writer", 0)
    assert ex.definition(ref("registry/put_draft")).is_write

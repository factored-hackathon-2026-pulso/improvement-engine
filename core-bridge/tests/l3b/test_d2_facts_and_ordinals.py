"""Plan annex D.2 fact shapes (+16.16.1 write-key ordinals): first RED of the L3b reconciliation."""

from __future__ import annotations

import copy
import hashlib
from types import SimpleNamespace
from typing import Any

import pytest
from agent_core.domain.shared import ToolStatus

from pulso_core_runtime.facts import whitelist as wl
from pulso_core_runtime.facts.whitelist import (
    FactError,
    compose_writer_receipts,
    project_result,
)
from pulso_core_runtime.stages.catalog import CATALOG
from pulso_core_runtime.tools.builder import write_key
from pulso_core_runtime.tools.context import RegistryMutationCommitment as C

from .support import Env
from .test_protected_builder import CHANGES, CREATE, TITLE, commitment

D1, D2 = "a" * 64, "b" * 64
A1 = {"id": "art-1", "digest": D1, "media_type": "application/json"}
A2 = {"id": "art-2", "digest": D2, "media_type": "text/plain"}
SEEN = {"art-1": {"digest": D1, "media_type": "application/json"}, "art-2": {"digest": D2, "media_type": "text/plain"}}

HYP = {"schema_version": "1", "hypotheses": [{
    "id": "h1", "statement": "s", "mechanism": "m", "evidence_refs": [A1], "counterevidence_refs": [],
    "missing_evidence": ["x"], "next_queries": ["select 1"]}]}
VER = {"schema_version": "1", "assessments": [{
    "hypothesis_id": "h1", "verdict": "supported", "evidence_refs": [A1], "counterevidence_refs": [A2],
    "limitations": []}]}
SPEC = {"schema_version": "1", "change_spec": {"kind": "flow"}, "rationale": "why", "evidence_refs": [A1],
        "alternatives": [{"id": "alt0", "kind": "do_nothing", "summary": "keep"},
                         {"id": "alt1", "kind": "proposed_change", "summary": "edit", "limitations": ["l"]}]}
CASES = [("scout", "pulso_hypotheses", HYP), ("verifier", "pulso_verification", VER),
         ("builder_design", "pulso_change_spec", SPEC)]


def _mut(value: Any, fn: Any) -> Any:
    v = copy.deepcopy(value)
    fn(v)
    return v


@pytest.mark.parametrize(("stage", "fact", "value"), CASES)
def test_d2_shapes_validate(stage: str, fact: str, value: Any) -> None:
    from jsonschema import Draft202012Validator
    assert len(wl.validate_fact(fact, value, call_log_refs=SEEN)) == 64
    assert Draft202012Validator(CATALOG[stage].core_output_schema).is_valid(value)
    assert wl.uses_only_core_subset(CATALOG[stage].core_output_schema) and wl.is_relaxation(
        CATALOG[stage].core_output_schema, wl.strict_schema(fact))


@pytest.mark.parametrize(("fact", "value", "mutation"), [
    ("pulso_hypotheses", HYP, lambda v: v["hypotheses"][0].pop("mechanism")),
    ("pulso_hypotheses", HYP, lambda v: v["hypotheses"][0].pop("next_queries")),
    ("pulso_hypotheses", HYP, lambda v: v["hypotheses"][0].update(hypothesis_id="h")),
    ("pulso_hypotheses", HYP, lambda v: v["hypotheses"][0].update(evidence_refs=["art-1"])),
    ("pulso_hypotheses", HYP, lambda v: v["hypotheses"][0]["evidence_refs"][0].pop("media_type")),
    ("pulso_hypotheses", HYP, lambda v: v["hypotheses"][0]["evidence_refs"][0].update(digest="sha256:" + D1)),
    ("pulso_verification", VER, lambda v: v.update(verdicts=v.pop("assessments"))),
    ("pulso_verification", VER, lambda v: v["assessments"][0].pop("limitations")),
    ("pulso_verification", VER, lambda v: v["assessments"][0].update(evidence_refs=[])),  # supported needs evidence
    ("pulso_verification", VER, lambda v: v["assessments"][0].update(verdict="refuted", counterevidence_refs=[])),
    ("pulso_change_spec", SPEC, lambda v: v.pop("rationale")),
    ("pulso_change_spec", SPEC, lambda v: v.pop("alternatives")),
    ("pulso_change_spec", SPEC, lambda v: v.update(alternatives=[v["alternatives"][1]])),  # no do_nothing
    ("pulso_change_spec", SPEC, lambda v: v.update(evidence_refs=[])),
])
def test_d2_violations_rejected(fact: str, value: Any, mutation: Any) -> None:
    with pytest.raises(FactError) as e:
        wl.validate_fact(fact, _mut(value, mutation))
    assert e.value.code == "pulso:fact_schema_violation"


def test_refuted_with_counterevidence_and_inconclusive_without_evidence_are_valid() -> None:
    wl.validate_fact("pulso_verification", _mut(VER, lambda v: v["assessments"][0].update(
        verdict="refuted", evidence_refs=[])))
    wl.validate_fact("pulso_verification", _mut(VER, lambda v: v["assessments"][0].update(
        verdict="inconclusive", evidence_refs=[], counterevidence_refs=[])))


@pytest.mark.parametrize(("fact", "value"), [("pulso_hypotheses", HYP), ("pulso_verification", VER),
                                              ("pulso_change_spec", SPEC)])
def test_evidence_must_be_artifact_refs_seen_in_the_call_log(fact: str, value: Any) -> None:
    wl.validate_fact(fact, value, call_log_refs=SEEN)
    wl.validate_fact(fact, value, call_log_refs={"art-1", "art-2"})  # legacy id-only log
    for seen in ({"art-2": SEEN["art-2"]},  # id never fetched
                 {"art-1": {"digest": D2, "media_type": "application/json"}, "art-2": SEEN["art-2"]},  # digest
                 {"art-1": {"digest": D1, "media_type": "text/csv"}, "art-2": SEEN["art-2"]}):  # media type
        with pytest.raises(FactError):
            wl.validate_fact(fact, value, call_log_refs=seen)


def test_wiki_path_is_not_evidence() -> None:
    wiki = {"art-1": {"digest": None, "media_type": None}}
    bad = _mut(HYP, lambda v: v["hypotheses"][0].update(evidence_refs=[{"id": "wiki/notes.md", "digest": D1,
                                                                        "media_type": "text/markdown"}]))
    with pytest.raises(FactError):
        wl.validate_fact("pulso_hypotheses", bad, call_log_refs=wiki)


def test_wiki_read_records_no_path_and_artifacts_are_recorded_structurally() -> None:
    env = Env()
    ic = env.invocation("scout", confirmed=True)
    env.call(ic, "pulso/wiki_read", {"path": "notes/a.md"})
    assert "notes/a.md" not in env.contexts.seen_refs(ic.binding_ref)
    assert "notes/a.md" not in env.contexts.seen_artifacts(ic.binding_ref)
    env.contexts.record_artifact(ic.binding_ref, "art-9", digest="sha256:" + D1, media_type="application/json")
    assert env.contexts.seen_artifacts(ic.binding_ref)["art-9"] == {"digest": D1, "media_type": "application/json"}


def _fact(v: Any) -> SimpleNamespace:
    return SimpleNamespace(value=v, source=SimpleNamespace(kind="agent"))


def test_missing_value_never_raises_keyerror() -> None:
    for facts in ({"pulso_hypotheses": {"source": {"kind": "agent"}}}, {"pulso_hypotheses": None},
                  {"pulso_hypotheses": SimpleNamespace(source=SimpleNamespace(kind="agent"))}):
        with pytest.raises(FactError) as e:
            project_result("scout", core_run_id="r", status="completed", outcome="completed", run_facts=facts)
        assert e.value.code == "pulso:output_missing"
    env = project_result("scout", core_run_id="r", status="completed", outcome="completed",
                         run_facts={"pulso_hypotheses": _fact(HYP)}, call_log_refs=SEEN)
    assert set(env["facts"]) == {"pulso_hypotheses"}


def test_writer_receipts_tolerate_facts_without_value_or_bad_rev() -> None:
    facts = {"proposal": {"source": {"kind": "tool"}}, "frozen": None,
             "put_verified": _fact({"op": "put_draft", "rev_after": "x", "request_hash": "h"})}
    out = compose_writer_receipts(facts, [])
    assert out["proposal_id"] == "" and out["candidate_hash"] is None
    assert out["write_receipts"][0]["rev_after"] == 0


# -- write-key ordinals (plan 16.16.1) ---------------------------------------------------------------
def test_golden_write_key_vectors() -> None:
    assert write_key("cmd-0001", "writer", 0) == "pulso-w:0751d86d2f2e04368ae3ece1a34112b116a1b1a6a3bc67b8"
    assert write_key("cmd-0001", "writer", 1) == "pulso-w:8ee817042bcd34504616b536a4660c088252a7bd58add614"
    assert write_key("cmd-0001", "writer", 2) == "pulso-w:46bfd7151c8b5b3578433612df353dad2fd9b18af6b90394"
    assert write_key("job-a_b.c:9", "writer", 10) == "pulso-w:6e96c30e225b12e13c903199a1f43e688dbd8dbc9423b82d"
    assert write_key("c", "writer", 7) == "pulso-w:" + hashlib.sha256(b"c|writer|7").hexdigest()[:48]


def _writer(operations: tuple[str, ...]) -> tuple[Env, Any]:
    env = Env()
    ic = env.invocation("writer", commitment=commitment(operations=operations))
    assert env.bind(ic).status is ToolStatus.ok
    return env, ic


def test_ordinal_is_the_index_in_the_committed_operation_array() -> None:
    # array order differs from call order: freeze is committed at index 2, put at 1; calls come create, freeze, put
    env, ic = _writer(("create_proposal", "put_draft", "freeze"))
    put = {"proposal_id": "prop-1", "expected_rev": 1, "changes": CHANGES}
    env.call(ic, "registry/create_proposal", {**CREATE, "title": TITLE}, key="e-c")
    assert env.inner.calls[-1][2] == write_key(ic.command_key, "writer", 0)
    env.call(ic, "registry/freeze", {"proposal_id": "prop-1"}, key="e-f")
    assert env.inner.calls[-1][2] == write_key(ic.command_key, "writer", 2)  # index, not call order
    env.call(ic, "registry/put_draft", put, key="e-p")
    assert env.inner.calls[-1][2] == write_key(ic.command_key, "writer", 1)
    env.call(ic, "registry/get_write", {"idempotency_key": "e-f"})
    assert env.inner.calls[-1][1]["idempotency_key"] == write_key(ic.command_key, "writer", 2)


def test_duplicate_ops_take_successive_indexes_and_extra_write_is_denied() -> None:
    env, ic = _writer(("create_proposal", "put_draft"))
    env.call(ic, "registry/create_proposal", CREATE, key="e1")
    put = {"proposal_id": "prop-1", "expected_rev": 1, "changes": CHANGES}
    assert env.call(ic, "registry/put_draft", put, key="e2").status is ToolStatus.ok
    n = len(env.inner.calls)
    r = env.call(ic, "registry/put_draft", put, key="e3")  # a third write is not in the commitment
    assert r.status is ToolStatus.denied and len(env.inner.calls) == n
    assert env.call(ic, "registry/put_draft", put, key="e2").status is ToolStatus.ok  # replay keeps its index
    assert env.inner.calls[-1][2] == write_key(ic.command_key, "writer", 1)


def test_op_absent_from_the_committed_array_is_denied_with_zero_effect() -> None:
    env, ic = _writer(("create_proposal", "put_draft"))
    env.call(ic, "registry/create_proposal", CREATE, key="e1")
    n = len(env.inner.calls)
    r = env.call(ic, "registry/freeze", {"proposal_id": "prop-1"}, key="e2")
    assert r.status is ToolStatus.denied and len(env.inner.calls) == n


def test_recovery_reads_own_keys_by_committed_array_length() -> None:
    env, ic = _writer(("create_proposal", "put_draft", "freeze"))
    for n in range(3):
        env.call(ic, "registry/get_write", {"idempotency_key": write_key(ic.command_key, "writer", n)})
        assert env.inner.calls[-1][0] == "registry/get_write"
    n = len(env.inner.calls)
    r = env.call(ic, "registry/get_write", {"idempotency_key": write_key(ic.command_key, "writer", 3)})
    assert r.status is ToolStatus.denied and len(env.inner.calls) == n


def test_commitment_without_operations_keeps_legacy_first_seen_order() -> None:
    assert C(mode="write", proposal_id=None, expected_rev=None, base_release_id=None,
             evaluate_enabled=False).operations == ()

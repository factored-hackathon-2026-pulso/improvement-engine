"""Stage catalogue vs agent-core-assets, fact whitelist, strict vs Core-subset schemas."""

from __future__ import annotations

import copy
import shutil
from pathlib import Path
from types import SimpleNamespace
from typing import Any

import pytest
import yaml

from pulso_core_runtime.facts import whitelist as wl
from pulso_core_runtime.facts.whitelist import FactError, compose_writer_receipts, project_result
from pulso_core_runtime.stages.catalog import CATALOG, check_against_assets, stage_allows

WORLD = Path(__file__).resolve().parents[3] / "agent-core-assets" / "worlds" / "pulso-evolution"


def test_catalogue_matches_the_flows_and_agents_in_assets() -> None:
    assert WORLD.is_dir()
    assert check_against_assets(WORLD) == []


def test_drift_in_save_as_output_schema_tools_or_first_node_fails_ci(tmp_path: Path) -> None:
    root = tmp_path / "w"
    shutil.copytree(WORLD, root)
    flow = root / "flows" / "pulso-scout@1.0.0.yaml"
    doc = yaml.safe_load(flow.read_text(encoding="utf-8"))
    agent_node = next(n for n in doc["nodes"] if n["type"] == "agent")
    agent_node["config"]["save_as"] = "other"
    agent_node["config"]["tools_allowed"].append("pulso/artifact_get@1")
    agent_node["config"]["output_schema"]["properties"]["extra"] = {"type": "string"}
    doc["nodes"][0]["config"]["tool"] = "pulso/wiki_read@1"
    flow.write_text(yaml.safe_dump(doc), encoding="utf-8")
    problems = "\n".join(check_against_assets(root))
    for needle in ("first node", "save_as", "tools_allowed", "output_schema"):
        assert needle in problems


def test_writer_slots_are_the_declared_ones() -> None:
    assert set(CATALOG["writer"].input_slots) == {"draft_plan_ref", "proposal_id", "base_release_id",
                                                  "evaluate_enabled", "evaluation_suite_id",
                                                  "evaluation_suite_version"}


@pytest.mark.parametrize("stage", ["scout", "verifier", "builder_design"])
def test_core_subset_is_a_relaxation_of_the_strict_schema(stage: str) -> None:
    spec = CATALOG[stage]
    core, strict = spec.core_output_schema, wl.strict_schema(spec.fact)
    assert core is not None and wl.uses_only_core_subset(core)
    assert wl.is_relaxation(core, strict)


def test_relaxation_check_detects_a_tightened_core_schema() -> None:
    strict = wl.strict_schema("pulso_verification")
    core = copy.deepcopy(CATALOG["verifier"].core_output_schema)
    assert core is not None
    core["properties"]["verdicts"]["items"]["properties"]["verdict"]["enum"] = ["supported"]
    assert not wl.is_relaxation(core, strict)


H = {"schema_version": "1", "hypotheses": [{"hypothesis_id": "h1", "statement": "s", "evidence_refs": ["art:1"]}]}


def _facts(value: Any, kind: str = "agent") -> dict[str, Any]:
    return {"pulso_hypotheses": SimpleNamespace(value=value, source=SimpleNamespace(kind=kind)),
            "binding": SimpleNamespace(value={"x": 1}, source=SimpleNamespace(kind="tool")),
            "token_map": {"secret": "x"}}


def _project(value: Any, **kw: Any) -> dict[str, Any]:
    return project_result("scout", core_run_id="r1", status="completed", outcome="completed",
                          run_facts=_facts(value), **kw)


def test_projection_keeps_only_whitelisted_facts_and_digests_jcs() -> None:
    env = _project(H)
    assert set(env["facts"]) == {"pulso_hypotheses"}
    assert env["facts"]["pulso_hypotheses"]["source_kind"] == "agent"  # never promoted
    assert len(env["facts"]["pulso_hypotheses"]["digest"]) == 64
    assert "token_map" not in str(env) and env["schema_version"] == "1"


def test_completed_without_required_fact_is_output_missing() -> None:
    with pytest.raises(FactError) as e:
        project_result("scout", core_run_id="r1", status="completed", outcome="completed", run_facts={})
    assert e.value.code == "pulso:output_missing"


def test_schema_violations_floats_and_unknown_keys_rejected() -> None:
    bad = copy.deepcopy(H)
    bad["hypotheses"][0]["extra"] = 1
    for value in (bad, {"schema_version": "2", "hypotheses": H["hypotheses"]},
                  {**H, "hypotheses": [{**H["hypotheses"][0], "evidence_refs": []}]},
                  {**H, "hypotheses": [{**H["hypotheses"][0], "statement": 1.5}]}):
        with pytest.raises(FactError) as e:
            _project(value)
        assert e.value.code == "pulso:fact_schema_violation"


def test_fact_and_result_caps() -> None:
    big = {"schema_version": "1", "change_spec": {"k": "x" * (130 * 1024)}}
    with pytest.raises(FactError) as e:
        wl.validate_fact("pulso_change_spec", big)
    assert e.value.code == "pulso:output_too_large"


def test_canary_scan_and_call_log_evidence() -> None:
    with pytest.raises(FactError) as e:
        _project({**H, "hypotheses": [{**H["hypotheses"][0], "statement": "leak CANARY-123 here"}]},
                 canaries=["CANARY-123"])
    assert e.value.code == "pulso:canary_detected"
    _project(H, call_log_refs={"art:1"})
    with pytest.raises(FactError):
        _project(H, call_log_refs={"art:other"})


def test_writer_receipts_projection_unknown_when_action_uncertain_without_readback() -> None:
    def fact(v: Any) -> SimpleNamespace:
        return SimpleNamespace(value=v, source=SimpleNamespace(kind="tool"))

    facts = {"proposal": fact({"proposal_id": "p1", "rev": 2}),
             "put_verified": fact({"op": "put_draft", "rev_after": 2, "request_hash": "h"}),
             "frozen": fact({"candidate_hash": "sha256-abc"}),
             "freeze_verified": fact({"op": "freeze", "rev_after": 3, "request_hash": "h2"})}
    actions = [{"tool": {"id": "registry/put_draft"}, "state": "verified", "idempotency_key": "pulso-w:a"},
               {"tool": {"id": "registry/freeze"}, "state": "verified", "idempotency_key": "pulso-w:b"}]
    ok = compose_writer_receipts(facts, actions)
    assert ok["state"] == "confirmed" and ok["proposal_id"] == "p1" and len(ok["write_receipts"]) == 2
    wl.validate_fact("pulso_writer_receipts", ok)
    actions.append({"tool": {"id": "registry/evaluate"}, "state": "uncertain", "idempotency_key": "pulso-eval:x"})
    assert compose_writer_receipts(facts, actions)["state"] == "unknown"
    env = project_result("writer", core_run_id="r", status="completed", outcome="completed", run_facts={},
                         writer_receipts=ok)
    assert set(env["facts"]) == {"pulso_writer_receipts"}


def test_stage_allow_lists() -> None:
    assert stage_allows("scout", "pulso/lab_query@1.0.0")
    assert not stage_allows("builder_design", "pulso/wiki_transform@1")
    assert not stage_allows("writer", "pulso/lab_query@1")
    assert not stage_allows("builder_design", "registry/put_draft@1")


def test_strict_is_stricter_than_core_for_the_same_document() -> None:
    from jsonschema import Draft202012Validator

    loose = {"schema_version": "1", "hypotheses": []}
    assert Draft202012Validator(CATALOG["scout"].core_output_schema).is_valid(loose)
    assert not Draft202012Validator(wl.strict_schema("pulso_hypotheses")).is_valid(loose)

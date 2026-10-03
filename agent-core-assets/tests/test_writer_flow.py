"""Writer flow follows the protected writer flow of plan 17.3.3 (offline structural checks)."""

from __future__ import annotations

from pathlib import Path

import yaml

WORLD = Path(__file__).resolve().parents[1] / "worlds/pulso-evolution"


# PyYAML (YAML 1.1) turns the rule branch keys `true`/`false` into bools; Core's loader keeps them as labels.


def _flow() -> dict:
    return yaml.safe_load((WORLD / "flows/pulso-writer@1.0.0.yaml").read_text("utf-8"))


def _nodes() -> dict:
    return {n["id"]: n for n in _flow()["nodes"]}


def _tools(nodes: dict) -> list[str]:
    return [n["config"]["tool"] for n in nodes.values() if n["type"] == "tool"]


def test_writer_has_proposal_exists_rule_before_create() -> None:
    nodes = _nodes()
    rules = [n for n in nodes.values() if n["type"] == "rule"]
    exists = [n for n in rules if "facts.binding.value.proposal_id" in yaml.safe_dump(n["config"])]
    assert exists, "no rule on facts.binding.value.proposal_id"
    nxt = exists[0]["next"]
    assert nodes[nxt[True]]["config"].get("tool") == "registry/create_proposal@1"
    assert nodes[nxt[False]]["config"].get("tool") == "registry/get_proposal@1"


def test_writer_checks_base_release_id_before_put_draft() -> None:
    nodes = _nodes()
    base = [n for n in nodes.values() if n["type"] == "rule" and "base_release_id" in yaml.safe_dump(n["config"])]
    assert base and "facts.binding.value.base_release_id" in yaml.safe_dump(base[0]["config"])
    nxt = base[0]["next"]
    assert nodes[nxt[True]]["config"]["tool"] == "registry/put_draft@1"
    assert nodes[nxt[False]]["type"] == "escalate"


def test_writer_validates_before_freeze() -> None:
    nodes = _nodes()
    assert nodes["verify_put"]["next"]["verified"] == "validate"
    assert nodes["validate"]["config"]["tool"] == "registry/validate@1"
    gate = nodes[nodes["validate"]["next"]["ok"]]
    assert gate["type"] == "rule" and "validation" in yaml.safe_dump(gate["config"])
    assert nodes[gate["next"][True]]["config"]["tool"] == "registry/freeze@1"  # PyYAML reads `true:` as a bool key


def test_writer_evaluates_only_when_enabled_after_freeze() -> None:
    nodes = _nodes()
    enabled = [n for n in nodes.values() if n["type"] == "rule" and "evaluate_enabled" in yaml.safe_dump(n["config"])]
    assert enabled
    nxt = enabled[0]["next"]
    assert nodes[nxt[True]]["config"]["tool"] == "registry/evaluate@1"
    assert nodes[nxt[False]]["type"] == "end"
    assert nodes["verify_freeze"]["next"]["verified"] == enabled[0]["id"]
    assert nodes[nodes["evaluate"]["next"]["ok"]]["type"] == "verify"


def test_writer_agent_allowlist_matches_flow_tools_and_has_no_approve() -> None:
    agent = yaml.safe_load((WORLD / "agents/pulso-writer@1.0.0.yaml").read_text("utf-8"))
    used = set(_tools(_nodes()))
    assert used <= set(agent["tools_allowed"])
    assert not any(t.split("/")[1].startswith(("approve", "publish", "promote", "put_alias")) for t in agent["tools_allowed"])

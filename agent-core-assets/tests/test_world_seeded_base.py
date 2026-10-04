"""WRLD0 FIRST RED: the seeded base world must expose a dispute agent with a replaceable prompt and an eval-suite slot.

Offline tier (PyYAML only). The declaration lives in `worlds/seeded-base.world.yaml` (a file, not a world directory, so
it does not change any world `files_digest`, release id or expected state).
"""

from __future__ import annotations

import shutil
from pathlib import Path

import yaml

import assetcheck
import worldcheck

META = "worlds/seeded-base.world.yaml"


def codes(violations) -> set[str]:
    return {v.code for v in violations}


def meta_of(root: Path) -> dict:
    return yaml.safe_load((root / META).read_text("utf-8"))


def test_seeded_base_world_conforms(assets: Path) -> None:
    assert worldcheck.check_seeded_base(assets) == []


def test_label_and_agent_shape(assets: Path) -> None:
    m = meta_of(assets)
    assert m["base_world"] == "seeded"
    assert m["world"] == "attention-task"
    assert m["agent"]["id"] == "atencion-tarea"
    assert m["replaceable_prompt"]["id"] == "p/resumen_radicado"
    assert m["eval_suite_slot"]["agent_id"] == "atencion-tarea"


def test_world_files_digest_unchanged_by_declaration(assets: Path) -> None:
    assert assetcheck.check_manifest(assets) == []
    assert assetcheck.check_all(assets) == []


def test_missing_declaration_fails(assets: Path) -> None:
    (assets / META).unlink()
    assert "world_missing_declaration" in codes(worldcheck.check_seeded_base(assets))


def test_no_agent_with_replaceable_prompt_fails(assets: Path) -> None:
    """The row's first RED: a world whose agent has no replaceable prompt must not conform."""
    flow = assets / "worlds/attention-task/flows/disputa-tarea@1.0.0.yaml"
    flow.write_text(flow.read_text("utf-8").replace("prompt_ref: p/resumen_radicado@1, ", ""), "utf-8")
    assert "no_replaceable_prompt" in codes(worldcheck.check_seeded_base(assets))


def test_prompt_declared_but_absent_fails(assets: Path) -> None:
    (assets / "worlds/attention-task/prompts/p/resumen_radicado@1.0.0.yaml").unlink()
    assert "prompt_missing" in codes(worldcheck.check_seeded_base(assets))


def test_suite_slot_without_suite_fails(assets: Path) -> None:
    shutil.rmtree(assets / "worlds/attention-task/eval_suites")
    assert "suite_slot_empty" in codes(worldcheck.check_seeded_base(assets))


def test_authors_recorded_and_judge_is_different(assets: Path) -> None:
    m = meta_of(assets)
    authors = m["authors"]
    for role in ("world", "suite", "judge"):
        assert authors[role], role
    assert authors["judge"] not in (authors["world"], authors["suite"])


def test_judge_equal_to_world_author_fails(assets: Path) -> None:
    m = meta_of(assets)
    m["authors"]["judge"] = m["authors"]["world"]
    (assets / META).write_text(yaml.safe_dump(m, sort_keys=True), "utf-8")
    assert "author_not_separated" in codes(worldcheck.check_seeded_base(assets))


def test_missing_author_fails(assets: Path) -> None:
    m = meta_of(assets)
    del m["authors"]["suite"]
    (assets / META).write_text(yaml.safe_dump(m, sort_keys=True), "utf-8")
    assert "author_missing" in codes(worldcheck.check_seeded_base(assets))


def test_not_keyed_to_a_finding_category(assets: Path) -> None:
    """The world declaration names no category or finding; adding one fails (rename/permute honesty, test 3)."""
    m = meta_of(assets)
    assert not worldcheck.category_keyed(m)
    m["winning_category"] = "abstention-on-dispute"
    (assets / META).write_text(yaml.safe_dump(m, sort_keys=True), "utf-8")
    assert "keyed_to_category" in codes(worldcheck.check_seeded_base(assets))


def test_declares_the_two_targetable_kinds_only(assets: Path) -> None:
    assert sorted(meta_of(assets)["targetable_kinds"]) == ["add_eval_suite", "replace_prompt"]


def _edit(path: Path, old: str, new: str) -> None:
    text = path.read_text("utf-8")
    assert old in text, old
    path.write_text(text.replace(old, new), "utf-8")


def test_agent_is_evaluable_on_the_pinned_core(assets: Path) -> None:
    """INT0: the pinned Core composes no `jev` provider and serves an empty scripted `classifier`, so the seeded agent
    must be a task agent whose flow has no decide node and that declares no understand/slots model."""
    assert codes(worldcheck.check_seeded_base(assets)) == set()
    world = assets / "worlds/attention-task"
    agent = world / "agents/atencion-tarea@1.0.0.yaml"
    assert "mode: task" in agent.read_text("utf-8")


def test_conversational_agent_is_not_evaluable(assets: Path) -> None:
    agent = assets / "worlds/attention-task/agents/atencion-tarea@1.0.0.yaml"
    _edit(agent, "mode: task", "mode: conversational")
    assert "needs_decision_provider" in codes(worldcheck.check_seeded_base(assets))


def test_understand_model_is_not_evaluable(assets: Path) -> None:
    agent = assets / "worlds/attention-task/agents/atencion-tarea@1.0.0.yaml"
    agent.write_text(agent.read_text("utf-8") + "understand: understand-turno@1\n", "utf-8")
    assert "needs_decision_provider" in codes(worldcheck.check_seeded_base(assets))


def test_decide_node_is_not_evaluable(assets: Path) -> None:
    flow = assets / "worlds/attention-task/flows/disputa-tarea@1.0.0.yaml"
    _edit(flow, "  - {id: fin, type: end", "  - {id: coincide, type: decide, config: {model: match-cargo@2, branch_on: match}, next: {unica: fin}}\n  - {id: fin, type: end")
    assert "needs_decision_provider" in codes(worldcheck.check_seeded_base(assets))


def test_suite_scenarios_are_start_only(assets: Path) -> None:
    """A turn step needs `understand` (a model); the suite of a task agent only starts the run."""
    suite = assets / "worlds/attention-task/eval_suites/disputas-tarea-suite@1.0.0.yaml"
    _edit(suite, "- {op: start}", "- {op: start}\n      - {op: turn, text: hola}")
    assert "needs_decision_provider" in codes(worldcheck.check_seeded_base(assets))

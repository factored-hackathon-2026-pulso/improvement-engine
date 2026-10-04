"""WRLD0 conformance of the seeded base world (offline, PyYAML only).

The declaration `worlds/seeded-base.world.yaml` names the dispute agent, its one replaceable prompt and its eval-suite
slot, records author separation, and must not be keyed to any finding category.
"""

from __future__ import annotations

import re
from pathlib import Path
from typing import Any

import assetcheck
from assetcheck import Violation, load_yaml, walk, yaml_files

DECLARATION = "worlds/seeded-base.world.yaml"
KINDS = ("add_eval_suite", "replace_prompt")
AUTHOR_ROLES = ("world", "suite", "judge")
_CATEGORY_KEY = re.compile(r"categor|finding|winning|signal_family|hypothesis", re.I)


def category_keyed(meta: Any) -> bool:
    """True when the declaration carries a category, finding or winner field (it must stay category-neutral)."""
    return any(key and _CATEGORY_KEY.search(str(key)) for _, key, _ in walk(meta))


def _prompt_refs(world: Path, agent_id: str) -> set[str]:
    refs: set[str] = set()
    for a in (load_yaml(p) for p in yaml_files(world, "agents")):
        if a["id"] != agent_id:
            continue
        flow_id = a["entry_flow"].split("@")[0]
        for p in yaml_files(world, "flows"):
            f = load_yaml(p)
            if f["id"] != flow_id:
                continue
            for _, key, val in walk(f):
                if key == "prompt_ref" and isinstance(val, str):
                    refs.add(val.split("@")[0])
    return refs


def _agent_doc(world: Path, agent_id: str) -> dict[str, Any] | None:
    return next((a for a in (load_yaml(p) for p in yaml_files(world, "agents")) if a["id"] == agent_id), None)


def check_evaluable(world: Path, agent_id: str) -> list[Violation]:
    """The pinned Core (c814c2b) composes no `jev` provider and serves only an empty scripted `classifier`, so a seeded
    agent can only be evaluated (native evaluation, arms) when no step of it needs a decision model: a task agent, no
    `understand`/`slots_model`, no `decide` node in its entry flow, and a suite whose scenarios only start the run."""
    out: list[Violation] = []
    agent = _agent_doc(world, agent_id)
    if agent is None:
        return out
    at = f"{world.name}/agents/{agent_id}"
    if agent.get("mode") != "task":
        out.append(Violation("needs_decision_provider", at, "conversational turns need an `understand` model"))
    for field in ("understand", "slots_model"):
        if agent.get(field):
            out.append(Violation("needs_decision_provider", at, f"agent declares {field}"))
    flow_id = str(agent["entry_flow"]).split("@")[0]
    for p in yaml_files(world, "flows"):
        f = load_yaml(p)
        if f["id"] == flow_id:
            for n in f.get("nodes", []):
                if n.get("type") in ("decide", "collect", "confirm", "transfer", "await_approval"):
                    out.append(Violation("needs_decision_provider", f"{world.name}/flows/{flow_id}#{n.get('id')}",
                                         f"node type {n.get('type')} needs a model or a principal turn"))
    for p in yaml_files(world, "eval_suites"):
        s = load_yaml(p)
        if s["agent_id"] != agent_id:
            continue
        for sc in s.get("scenarios", []):
            for where, _key, val in walk(sc):
                if isinstance(val, float):
                    out.append(Violation("non_integer_number", f"{world.name}/eval_suites/{s['id']}#{sc['id']}{where}",
                                         "a non-integer JSON number in a draft is canonicalised differently by Core and by "
                                         "the Python draft digest; the writer commitment is denied (use integers or strings)"))
            if any(step.get("op") != "start" for step in sc.get("steps", [])):
                out.append(Violation("needs_decision_provider", f"{world.name}/eval_suites/{s['id']}#{sc['id']}",
                                     "a turn or confirm step needs `understand`; task scenarios only start"))
    return out


def check_seeded_base(root: Path) -> list[Violation]:
    path = root / DECLARATION
    if not path.is_file():
        return [Violation("world_missing_declaration", DECLARATION, "seeded base world is not declared")]
    meta = load_yaml(path)
    out: list[Violation] = []
    if meta.get("base_world") != "seeded":
        out.append(Violation("label", DECLARATION, "base_world must be 'seeded'"))
    if category_keyed(meta):
        out.append(Violation("keyed_to_category", DECLARATION, "declaration must not name a category or finding"))
    if sorted(meta.get("targetable_kinds", [])) != list(KINDS):
        out.append(Violation("kinds", DECLARATION, f"targetable_kinds must be {list(KINDS)}"))
    authors = meta.get("authors") or {}
    for role in AUTHOR_ROLES:
        if not authors.get(role):
            out.append(Violation("author_missing", DECLARATION, f"authors.{role} is required"))
    if authors.get("judge") and authors["judge"] in (authors.get("world"), authors.get("suite")):
        out.append(Violation("author_not_separated", DECLARATION, "judge author must differ from world and suite authors"))
    world = root / "worlds" / str(meta.get("world"))
    agent_id = (meta.get("agent") or {}).get("id")
    if agent_id not in assetcheck.agent_ids(root) or assetcheck.agent_ids(root)[agent_id] != world.name:
        out.append(Violation("agent_missing", DECLARATION, f"agent {agent_id} not in world {world.name}"))
        return out
    prompt_id = (meta.get("replaceable_prompt") or {}).get("id")
    if prompt_id not in _prompt_refs(world, agent_id):
        out.append(Violation("no_replaceable_prompt", DECLARATION, f"agent {agent_id} flow does not use prompt {prompt_id}"))
    if not any(load_yaml(p)["id"] == prompt_id for p in yaml_files(world, "prompts")):
        out.append(Violation("prompt_missing", DECLARATION, f"prompt {prompt_id} is not in the world"))
    out += check_evaluable(world, agent_id)
    slot_agent = (meta.get("eval_suite_slot") or {}).get("agent_id")
    if not any(load_yaml(p)["agent_id"] == slot_agent for p in yaml_files(world, "eval_suites")):
        out.append(Violation("suite_slot_empty", DECLARATION, f"no eval suite for agent {slot_agent}"))
    return out

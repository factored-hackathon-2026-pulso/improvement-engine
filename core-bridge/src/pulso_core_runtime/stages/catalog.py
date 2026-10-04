"""Stage catalogue (plan 17.3.3 "The four stages"): the single bridge-side truth that must equal the Flows in
`agent-core-assets/worlds/pulso-evolution` (L4 owns the YAML; CI fails on drift: `check_against_assets`).

L3 owns this file and the fact schemas. `output_schema` here is the Core-subset (`type, enum, properties,
required, additionalProperties(bool), items`) that is embedded in the Flow; the strict schemas live in
`facts/schemas/*.strict.json`."""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
from typing import Any

BIND_TOOL = "pulso/bind_context@1"
ENTRY_NODE_TOOL = BIND_TOOL

_V1: dict[str, Any] = {"type": "string", "enum": ["1"]}
_STR: dict[str, Any] = {"type": "string"}
_STR_LIST: dict[str, Any] = {"type": "array", "items": _STR}


def _obj(required: list[str], properties: dict[str, Any]) -> dict[str, Any]:
    return {"type": "object", "additionalProperties": False, "required": required, "properties": properties}


_ARTIFACT_REF = _obj(["id", "digest", "media_type"], {"id": _STR, "digest": _STR, "media_type": _STR})
_REFS: dict[str, Any] = {"type": "array", "items": _ARTIFACT_REF}
_VERDICT = {"type": "string", "enum": ["supported", "refuted", "inconclusive"]}
_KIND = {"type": "string", "enum": ["do_nothing", "proposed_change"]}

# Annex D.2 shapes. Core's `output_schema` has no minItems/pattern/contains: the strict schemas carry those.
HYPOTHESES_CORE = _obj(["schema_version", "hypotheses"], {
    "schema_version": _V1,
    "hypotheses": {"type": "array", "items": _obj(
        ["id", "statement", "mechanism", "evidence_refs", "counterevidence_refs", "missing_evidence",
         "next_queries"],
        {"id": _STR, "statement": _STR, "mechanism": _STR, "evidence_refs": _REFS,
         "counterevidence_refs": _REFS, "missing_evidence": _STR_LIST, "next_queries": _STR_LIST})}})
VERIFICATION_CORE = _obj(["schema_version", "assessments"], {
    "schema_version": _V1,
    "assessments": {"type": "array", "items": _obj(
        ["hypothesis_id", "verdict", "evidence_refs", "counterevidence_refs", "limitations"],
        {"hypothesis_id": _STR, "verdict": _VERDICT, "evidence_refs": _REFS, "counterevidence_refs": _REFS,
         "limitations": _STR_LIST})}})
CHANGE_SPEC_CORE = _obj(["schema_version", "change_spec", "rationale", "evidence_refs", "alternatives"], {
    "schema_version": _V1, "change_spec": {"type": "object"}, "rationale": _STR, "evidence_refs": _REFS,
    "alternatives": {"type": "array", "items": _obj(
        ["id", "kind", "summary"], {"id": _STR, "kind": _KIND, "summary": _STR, "limitations": _STR_LIST})}})


@dataclass(frozen=True)
class StageSpec:
    stage: str
    agent_id: str
    flow_id: str
    fact: str  # whitelisted fact / projection name
    tools_allowed: tuple[str, ...]  # agent-level allow-list (Agent.tools_allowed)
    input_slots: tuple[str, ...]  # run inputs read by the Flow as `facts.binding.value.X`
    node_tools: tuple[str, ...]  # tool ids usable by the engine's tool nodes of the Flow
    agent_node: str | None  # node id of the single agent node (None for the writer)
    agent_node_tools: tuple[str, ...] = ()
    core_output_schema: dict[str, Any] | None = None
    projection: bool = False  # fact composed by the bridge (writer) instead of an agent `save_as`
    first_node_tool: str = ENTRY_NODE_TOOL


_BIND, _ART = BIND_TOOL, "pulso/artifact_get@1"
_LAB, _WR, _WE, _WT = "pulso/lab_query@1", "pulso/wiki_read@1", "pulso/wiki_explore@1", "pulso/wiki_transform@1"
_REG = tuple(f"registry/{n}@1" for n in ("create_proposal", "put_draft", "freeze", "reopen", "validate",
                                         "evaluate", "get_proposal", "get_write"))

CATALOG: dict[str, StageSpec] = {s.stage: s for s in (
    StageSpec("scout", "pulso-scout", "pulso-scout", "pulso_hypotheses",
              (_BIND, _LAB, _WR, _WE, _WT), ("briefing_ref",), (_BIND, _WR), "investigate",
              (_LAB, _WR, _WE, _WT), HYPOTHESES_CORE),
    StageSpec("verifier", "pulso-verifier", "pulso-verifier", "pulso_verification",
              (_BIND, _LAB, _WR, _WE, _WT, _ART), ("hypotheses_ref",), (_BIND, _ART), "verify_hypotheses",
              (_LAB, _WR, _WE, _WT), VERIFICATION_CORE),
    StageSpec("builder_design", "pulso-builder-design", "pulso-builder-design", "pulso_change_spec",
              (_BIND, _ART, _WR, _LAB), ("design_input_ref",), (_BIND, _ART), "design",
              (_WR, _LAB), CHANGE_SPEC_CORE),
    StageSpec("writer", "pulso-writer", "pulso-writer", "pulso_writer_receipts",
              (_BIND, _ART, *_REG),
              ("draft_plan_ref", "proposal_id", "base_release_id", "evaluate_enabled",
               "evaluation_suite_id", "evaluation_suite_version"),
              (_BIND, _ART, *(t for t in _REG if "get_write" not in t)), None, (), None, projection=True),
)}
STAGES = tuple(CATALOG)

# M3: per-stage cap on agent steps for role-played (and local-model) runs. The asset `max_steps: 16` is unchanged
# (a registry change moves release digests); the responder finalises early. The writer is a projection: no model.
STEP_CAPS: dict[str, int] = {"scout": 5, "verifier": 5, "builder_design": 7}

# Closed subset of JSON Schema the Core `agent` node and the gateway understand (agent_core.domain.schema).
_SCHEMA_KEYWORDS = frozenset({"type", "enum", "properties", "required", "additionalProperties", "items"})
_SCHEMA_ANNOTATIONS = frozenset({"description", "title", "default", "examples", "$schema", "$id"})


def core_schema_problems(schema: dict[str, Any], path: str = "") -> list[str]:
    """Paths and keywords of `schema` outside the Core subset (nested `properties` and `items` included)."""
    where = path or "/"
    out = [f"{where}: unsupported keyword {k!r}" for k in schema
           if k not in _SCHEMA_KEYWORDS and k not in _SCHEMA_ANNOTATIONS]
    props = schema.get("properties")
    for name, child in (props.items() if isinstance(props, dict) else ()):
        if isinstance(child, dict):
            out += core_schema_problems(child, f"{path}/{name}")
    if isinstance(schema.get("items"), dict):
        out += core_schema_problems(schema["items"], f"{path}/items")
    return out


def tool_key(tool_id: str) -> str:
    """`pulso/lab_query@1.0.0` -> `pulso/lab_query@1`."""
    ident, _, version = tool_id.partition("@")
    return f"{ident}@{version.split('.')[0]}" if version else ident


def stage_allows(stage: str, tool_id: str) -> bool:
    spec = CATALOG.get(stage)
    return spec is not None and tool_key(tool_id) in spec.tools_allowed


# -- CI: catalogue vs Flows/Agents in agent-core-assets ---------------------------------------------
_INPUT_PREFIX = "facts.binding.value."


def _slots(node: Any, acc: set[str], legacy: set[str] | None = None) -> None:
    """Collects the run inputs a Flow reads. Core 1.3.0 never validates run-input slots, so Flows must read them
    from `facts.binding.value.X` (provided by `pulso/bind_context`); any `slots.X` read lands in `legacy`."""
    if isinstance(node, str):
        if node.startswith(_INPUT_PREFIX):
            acc.add(node[len(_INPUT_PREFIX):].split(".", 1)[0])
        elif node.startswith("slots.") and legacy is not None:
            legacy.add(node)
    elif isinstance(node, dict):
        for v in node.values():
            _slots(v, acc, legacy)
    elif isinstance(node, list):
        for v in node:
            _slots(v, acc, legacy)


def check_against_assets(world_root: Path) -> list[str]:
    """Returns the list of discrepancies (empty = aligned) between CATALOG and the world's Flows/Agents."""
    import yaml  # local: only CI/tests need it

    problems: list[str] = []
    for spec in CATALOG.values():
        flow_file = world_root / "flows" / f"{spec.flow_id}@1.0.0.yaml"
        agent_file = world_root / "agents" / f"{spec.agent_id}@1.0.0.yaml"
        try:
            flow = yaml.safe_load(flow_file.read_text(encoding="utf-8"))
            agent = yaml.safe_load(agent_file.read_text(encoding="utf-8"))
        except OSError as exc:
            problems.append(f"{spec.stage}: cannot read {exc.filename}")
            continue
        nodes = flow["nodes"]
        first = nodes[0]
        if first.get("type") != "tool" or first["config"].get("tool") != spec.first_node_tool:
            problems.append(f"{spec.stage}: first node is not {spec.first_node_tool}")
        if tuple(agent.get("tools_allowed", ())) != spec.tools_allowed:
            problems.append(f"{spec.stage}: Agent.tools_allowed differs")
        if agent.get("mode") != "task" or agent.get("invocable_by") != ["builder"]:
            problems.append(f"{spec.stage}: Agent must be mode=task, invocable_by=[builder]")
        slots: set[str] = set()
        legacy: set[str] = set()
        _slots(nodes, slots, legacy)
        if legacy:
            problems.append(f"{spec.stage}: reads claimed slots {sorted(legacy)} (use facts.binding.value.*)")
        if slots != set(spec.input_slots):
            problems.append(f"{spec.stage}: slots {sorted(slots)} != catalogue {sorted(spec.input_slots)}")
        tool_nodes = {n["config"]["tool"] for n in nodes if n.get("type") == "tool"}
        if tool_nodes != set(spec.node_tools):
            problems.append(f"{spec.stage}: tool nodes {sorted(tool_nodes)} != {sorted(spec.node_tools)}")
        agent_nodes = [n for n in nodes if n.get("type") == "agent"]
        if spec.agent_node is None:
            if agent_nodes:
                problems.append(f"{spec.stage}: unexpected agent node")
            continue
        if len(agent_nodes) != 1 or agent_nodes[0]["id"] != spec.agent_node:
            problems.append(f"{spec.stage}: agent node mismatch")
            continue
        cfg = agent_nodes[0]["config"]
        if cfg.get("save_as") != spec.fact:
            problems.append(f"{spec.stage}: save_as {cfg.get('save_as')} != {spec.fact}")
        if tuple(cfg.get("tools_allowed", ())) != spec.agent_node_tools:
            problems.append(f"{spec.stage}: node tools_allowed differs")
        if cfg.get("output_schema") != spec.core_output_schema:
            problems.append(f"{spec.stage}: output_schema differs from the Core-subset schema")
    return problems

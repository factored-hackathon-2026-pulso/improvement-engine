"""Synthetic stage corpus (M3): per LLM stage, the scripted steps a role-playing responder would answer, built with
the same input dict `LLMAgentPort.step` sends. All data is generated (no E0, no CSV): opaque ids, enum metrics and
k-anonymous aggregate rows only. Plumbing only, `quality_claims: forbidden`."""

from __future__ import annotations

import json
import uuid
from typing import Any

from pulso_core_runtime.stages.catalog import CATALOG

LAB, WIKI = "pulso/lab_query@1.0.0", "pulso/wiki_read@1.0.0"
DIGEST = "a" * 64
REF = {"id": "ev_5a17c0de0001", "digest": DIGEST, "media_type": "application/json"}
ROWS = [{"metric_id": "synthetic_metric", "window_id": "w1", "count": 40, "rate": 0.25, "evidence_ref": "ev_5a17c0de0001"}]

# stage -> (tool for step 1, final output of step 2)
SCRIPTS: dict[str, tuple[str, dict[str, Any]]] = {
    "scout": (LAB, {"schema_version": "1", "hypotheses": [{
        "id": "h1", "statement": "Synthetic statement.", "mechanism": "Synthetic mechanism.",
        "evidence_refs": [REF], "counterevidence_refs": [], "missing_evidence": [], "next_queries": []}]}),
    "verifier": (LAB, {"schema_version": "1", "assessments": [{
        "hypothesis_id": "h1", "verdict": "supported", "evidence_refs": [REF], "counterevidence_refs": [],
        "limitations": ["synthetic"]}]}),
    "builder_design": (WIKI, {"schema_version": "1", "change_spec": {"target": "synthetic"},
                              "rationale": "Synthetic rationale.", "evidence_refs": [REF],
                              "alternatives": [{"id": "alt1", "kind": "do_nothing", "summary": "Leave it."}]}),
}
SYSTEM = "Synthetic {stage} prompt. Respond with one JSON step."


def system_prompt(stage: str) -> str:
    return SYSTEM.format(stage=stage)


def step_inputs(stage: str, step: int) -> dict[str, Any]:
    """The agent input dict of `stage` at `step` (1 or 2), with fresh volatile ids on every call."""
    spec = CATALOG[stage]
    tool = SCRIPTS[stage][0]
    observations = [] if step == 1 else [{"tool": tool, "args": {"metric_id": "synthetic_metric"},
                                          "status": "ok", "result": {"rows": ROWS}, "error": None}]
    return {
        "goal": f"Synthetic {stage} goal.",
        "inputs": {spec.input_slots[0]: f"artifact-{uuid.uuid4().hex[:12]}",
                   "binding_id": f"binding-{uuid.uuid4().hex[:12]}"},
        "step": step,
        "tools": [{"tool": tool, "description": "Synthetic tool.", "args_schema": {"type": "object"}}],
        "observations": observations,
        "feedback": None,
        "output_schema": spec.core_output_schema,
    }


def content(stage: str, step: int) -> dict[str, Any]:
    tool, final = SCRIPTS[stage]
    if step == 1:
        return {"kind": "tool_call", "tool": tool, "args": {"metric_id": "synthetic_metric"}}
    return {"kind": "final", "output": final}


def chat_body(stage: str, inputs: dict[str, Any], model: str) -> bytes:
    user = json.dumps(inputs, sort_keys=True, separators=(",", ":"))
    return json.dumps({"model": model, "temperature": 0, "max_tokens": 4000, "messages": [
        {"role": "system", "content": system_prompt(stage)}, {"role": "user", "content": user}]}).encode()

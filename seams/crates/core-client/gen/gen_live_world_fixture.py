#!/usr/bin/env python3
"""Generates tests/fixtures/live_attention_task.json: the live drafts for the seeded base world `attention-task`
(agent `atencion-tarea`, prompt `p/resumen_radicado`, suite `disputas-tarea-suite`), built from the world's own asset
files exactly as `e2e-core/.../core_hooks.py::changes_from_ops` does (base content with a new version), plus the
Core's own `content_hash` of each suite entity (the evaluation admission needs it; Rust does not recompute it).

Variants: `accept` (2.0.0, the base suite unchanged), `accept_alt` (4.0.0, same, evaluate-only captures) and `fail_suite` (3.0.0, scenario `resuelto` expects an escalation
that the agent's rules never produce, so the native evaluation fails honestly). Integers and strings only.

Usage (repo root, pinned venv with agent_core + rfc8785 + pyyaml):
  the pinned pulso-wire venv python seams/crates/core-client/gen/gen_live_world_fixture.py
"""
import copy
import hashlib
import json
import pathlib

import rfc8785
import yaml
from agent_core.registry import EvalSuite
from agent_core.registry.entities import content_hash

ROOT = pathlib.Path(__file__).resolve().parents[4]
W = ROOT / "agent-core-assets/worlds/attention-task"
OUT = pathlib.Path(__file__).resolve().parents[1] / "tests/fixtures/live_attention_task.json"
DOCS = {"description": "live K3 change", "rationale": "world asset re-versioned", "changelog": "k3-live"}


def load(p):
    return yaml.safe_load(p.read_text("utf-8"))


def digest(v) -> str:
    return hashlib.sha256(rfc8785.dumps(v)).hexdigest()


def drafts_digest(changes) -> str:
    """The `draft_plan_digest` an arm's `frozen_candidate` target carries: digest_json of the sorted EntityDraft dumps
    (e2e-core/.../core_hooks.py::_drafts_digest)."""
    from agent_core.registry.models import EntityDraft
    drafts = sorted((EntityDraft.model_validate(c) for c in changes), key=lambda d: (d.kind, str(d.content.get("id", ""))))
    return digest([d.model_dump(mode="json") for d in drafts])


def no_floats(v):
    if isinstance(v, float):
        raise SystemExit("non-integer number in a draft: Core and the draft digest canonicalise it differently")
    if isinstance(v, dict):
        [no_floats(x) for x in v.values()]
    if isinstance(v, list):
        [no_floats(x) for x in v]


def variant(version: str, mutate=None):
    prompt = load(W / "prompts/p/resumen_radicado@1.0.0.yaml")
    suite = load(W / "eval_suites/disputas-tarea-suite@1.0.0.yaml")
    prompt["version"] = suite["version"] = version
    if mutate:
        mutate(suite)
    changes = [{"kind": "prompt", "content": prompt, "docs": DOCS}, {"kind": "eval_suite", "content": suite, "docs": DOCS}]
    no_floats(changes)
    return {"version": version, "changes": changes, "suite_id": suite["id"], "suite_version": version,
            "suite_digest": str(content_hash(EvalSuite.model_validate(suite))),
            "draft_plan_digest": drafts_digest(changes),
            "put_draft_digest": digest({"proposal_id": None, "expected_rev": None, "changes": changes})}


def contradict(suite):
    sc = copy.deepcopy(suite["scenarios"][0])
    assert sc["id"] == "resuelto"
    suite["scenarios"][0]["expect"] = {"outcome": "escalated", "escalated": True}


def main() -> None:
    out = {"generated_by": "gen/gen_live_world_fixture.py", "agent_id": "atencion-tarea",
           "variants": {"accept": variant("2.0.0"), "fail_suite": variant("3.0.0", contradict), "accept_alt": variant("4.0.0")}}
    OUT.write_bytes((json.dumps(out, indent=1, ensure_ascii=True, sort_keys=True) + "\n").encode("ascii"))
    print(f"wrote {OUT}")


if __name__ == "__main__":
    main()

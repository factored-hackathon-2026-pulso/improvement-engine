#!/usr/bin/env python3
"""Generates tests/fixtures/k3_draft.json: a compiled ChangeSpec (Replace Prompt + Add EvalSuite; integers and strings
ONLY, since non-integer numbers canonicalise differently in Core and in drafts) with the digests the Python references
compute: `put_draft_digest` (codex_standin.engine.put_draft_digest(None, None, changes)) and the sealed artifact
digest (`digest_json` of the draft-plan artifact).

Usage (repo root): uv run --with rfc8785 python seams/crates/core-client/gen/gen_k3_fixture.py
"""
import hashlib
import json
import pathlib

import rfc8785

OUT = pathlib.Path(__file__).resolve().parents[1] / "tests/fixtures/k3_draft.json"
DOCS = {"description": "k3 fixture", "rationale": "mejorar", "changelog": "k3"}
CHANGES = [
    {"kind": "prompt", "docs": DOCS, "content": {
        "id": "p/resumen_radicado", "version": "1.1.0", "model_profile": "perfil-generacion@1.0.0",
        "locales": {"es": "Confirma en una frase que la disputa quedo radicada.",
                    "pt": "Confirme em uma frase que a contestacao foi registrada."}}},
    {"kind": "eval_suite", "docs": DOCS, "content": {
        "id": "disputas-suite", "version": "1.0.0", "repetitions": 1, "seed": 120,
        "scenarios": [{"id": "s1", "input": "Quiero disputar un cargo", "expect": {"outcome": "resolved"}}]}},
]
PLAN = {"agent_id": "atencion", "title": "pulso-key:k3fixture", "changes": CHANGES}


def digest(v) -> str:
    return hashlib.sha256(rfc8785.dumps(v)).hexdigest()


def main() -> None:
    out = {"generated_by": "gen/gen_k3_fixture.py", "plan": PLAN,
           "put_draft_digest": digest({"proposal_id": None, "expected_rev": None, "changes": CHANGES}),
           "artifact_digest": digest(PLAN)}
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_bytes((json.dumps(out, indent=1, ensure_ascii=True, sort_keys=True) + "\n").encode("ascii"))
    print(f"wrote {OUT}")


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""ART2: add the flow graphs, policies, ToolDefs and the tool-service snapshot to the reasoning baseline, and write an ALIGNED copy of
the agent-core registry-e2e fixtures for a local stack.

    python scripts/reasoning/export_art2_graph.py --registry-e2e <dir> --tool-service <checkout> --base seams/crates/reasoning/fixtures/base_artifacts.json [--aligned-out <dir>]

Drift (verified 2026-10-05): the registry-e2e ToolDefs carry a `source` that differs from the tool-service one (productos vs
customer_products, movimientos vs customer_transactions, pqr vs customer_cases) or none at all (obtener_pqr, buscar_transacciones). The
engine refuses a link on such a tool (`source_mismatch`/`source_missing`). ALIGNED_SOURCES is an additive FIXTURE fix (engine side only);
the agent-core fixtures and the tool-service are not touched: that is an ask to their owners.
"""
import argparse
import json
import shutil
from pathlib import Path

import yaml

# SIG1 owns the alignment: the aligned ToolDefs and the provider listing come from scripts/contracts/tool_alignment (no duplicate table here).
ALIGN = Path(__file__).resolve().parents[1] / "contracts" / "tool_alignment"
ALIGNED = json.loads((ALIGN / "aligned_tool_defs.json").read_text(encoding="utf-8"))["tool_defs"]
PROVIDER = json.loads((ALIGN / "tool_service_catalog.snapshot.json").read_text(encoding="utf-8"))["tools"]


def fix(x):
    """YAML 1.1 turns the branch labels `true`/`false` into booleans: the registry means the strings."""
    if isinstance(x, dict):
        return {("true" if k is True else "false" if k is False else k): fix(v) for k, v in x.items()}
    if isinstance(x, list):
        return [fix(v) for v in x]
    return x


def load(p):
    return fix(yaml.safe_load(Path(p).read_text(encoding="utf-8")))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--registry-e2e", type=Path, required=True)
    ap.add_argument("--tool-service", type=Path, required=False, help="unused (kept for compatibility)")
    ap.add_argument("--base", type=Path, required=True)
    ap.add_argument("--aligned-out", type=Path)
    a = ap.parse_args()
    fx = a.registry_e2e
    base = json.loads(a.base.read_text(encoding="utf-8"))
    base["flows"] = {p.stem.split("@")[0]: load(p) for p in sorted((fx / "flows").glob("*.yaml"))}
    base["policies"] = {p.stem.split("@")[0]: load(p) for p in sorted((fx / "policies").glob("*.yaml"))}
    tools = {}
    for p in sorted((fx / "tools").glob("*.yaml")):
        t = load(p)
        tools[t["id"]] = ALIGNED.get(t["id"], t)
    base["tool_defs"] = tools
    base["tool_defs_label"] = "registry-e2e with ALIGNED sources (additive engine fixture fix, see export_art2_graph.py)"
    svc = [{k: t[k] for k in ("id", "version", "risk_class", "min_auth_level", "source") if k in t} for t in PROVIDER]
    base["tool_service"] = {"tools": svc, "label": "SIG1 snapshot of the tool-service listing (GET /v1/tools shape)"}
    a.base.write_text(json.dumps(base, ensure_ascii=False, indent=1, sort_keys=True) + "\n", encoding="utf-8")
    if a.aligned_out:
        if a.aligned_out.exists():
            shutil.rmtree(a.aligned_out)
        shutil.copytree(fx, a.aligned_out)
        for p in (a.aligned_out / "tools").glob("*.yaml"):
            t = load(p)
            if t["id"] in ALIGNED:
                t = ALIGNED[t["id"]]
                p.write_text(yaml.safe_dump(t, allow_unicode=True, sort_keys=False), encoding="utf-8")
    print("ok", len(base["flows"]), len(base["policies"]), len(tools), len(svc))


if __name__ == "__main__":
    main()

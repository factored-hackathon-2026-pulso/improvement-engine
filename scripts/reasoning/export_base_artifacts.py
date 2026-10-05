#!/usr/bin/env python3
"""Export the BASELINE artifacts the Builder may patch, from an agent-core checkout (registry-e2e fixtures).

    python scripts/reasoning/export_base_artifacts.py --agent-core <checkout> --out seams/crates/reasoning/fixtures/base_artifacts.json

Honest label: these are FIXTURE baselines (the e2e/demo seed), not the live registry of a shared Core. The
engine must read the live entity (`registry/get_entity`) before it proposes for real. Needs PyYAML (the
agent-core virtualenv has it). Only agent-core configuration text is read; no data, no secrets.
"""
import argparse
import json
import subprocess
from pathlib import Path

import yaml

KINDS = {"prompts": "prompt", "templates": "template"}


def load(p: Path):
    return yaml.safe_load(p.read_text(encoding="utf-8"))


def refs_of(obj):
    """Every string in an entity that looks like an id reference (`t/x`, `p/x`, `id@N`)."""
    out = set()

    def walk(x):
        if isinstance(x, str):
            out.add(x.split("@")[0])
        elif isinstance(x, dict):
            for v in x.values():
                walk(v)
        elif isinstance(x, list):
            for v in x:
                walk(v)

    walk(obj)
    return out


def merge_fixture(out: dict, fx: Path, head: str) -> dict:
    """EVT2: ADD the artifacts and agents of another agent-core fixture directory (e.g. `copiloto-sugerencias`, the task agent that
    emits the copilot suggestions) to an existing baseline, without touching anything already in it. Idempotent: an artifact or agent
    that is already present is left as it is."""
    agents = {p.stem.split("@")[0]: load(p) for p in sorted((fx / "agents").glob("*.yaml"))}
    flows = {p.stem.split("@")[0]: load(p) for p in sorted((fx / "flows").glob("*.yaml"))}
    have = {(a["kind"], a["id"]) for a in out["artifacts"]}
    for d, kind in KINDS.items():
        for p in sorted((fx / d).rglob("*.yaml")):
            e = load(p)
            if (kind, e["id"]) in have:
                continue
            extra = {k: v for k, v in e.items() if k not in ("id", "version", "locales")}
            used = [{"kind": "agent", "id": aid, "version": ag["version"]} for aid, ag in agents.items() if e["id"] in refs_of(ag)]
            used += [{"kind": "flow", "id": fid, "version": fl["version"]} for fid, fl in flows.items() if e["id"] in refs_of(fl)]
            out["artifacts"].append({"kind": kind, "id": e["id"], "version": e["version"], "locales": e["locales"], "extra": extra, "referenced_by": used})
    for aid, ag in agents.items():
        out["agents"].setdefault(aid, ag)
    out.setdefault("sources_added", {})[fx.name] = {"repo": "agent-core", "commit": head, "path": f"tests/fixtures/{fx.name}"}
    return out


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--agent-core", type=Path, required=True)
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--add-fixture", type=Path, help="merge another fixture dir (relative to tests/fixtures) into the EXISTING --out file and stop")
    a = ap.parse_args()
    if a.add_fixture:
        head = subprocess.run(["git", "-C", str(a.agent_core), "rev-parse", "--short", "HEAD"], capture_output=True, text=True).stdout.strip()
        base = json.loads(a.out.read_text(encoding="utf-8"))
        merged = merge_fixture(base, a.agent_core / "tests" / "fixtures" / a.add_fixture, head)
        a.out.write_text(json.dumps(merged, ensure_ascii=False, indent=1, sort_keys=True) + "\n", encoding="utf-8", newline="\n")
        print(f"merged {a.add_fixture} into {a.out} ({len(merged['artifacts'])} artifacts, {len(merged['agents'])} agents)")
        return 0
    fx = a.agent_core / "tests" / "fixtures" / "registry-e2e"
    head = subprocess.run(["git", "-C", str(a.agent_core), "rev-parse", "--short", "HEAD"], capture_output=True, text=True).stdout.strip()
    agents = {p.stem.split("@")[0]: load(p) for p in sorted((fx / "agents").glob("*.yaml"))}
    flows = {p.stem.split("@")[0]: load(p) for p in sorted((fx / "flows").glob("*.yaml"))}
    arts = []
    for d, kind in KINDS.items():
        for p in sorted((fx / d).rglob("*.yaml")):
            e = load(p)
            extra = {k: v for k, v in e.items() if k not in ("id", "version", "locales")}
            used = []
            for aid, ag in agents.items():
                if e["id"] in refs_of(ag):
                    used.append({"kind": "agent", "id": aid, "version": ag["version"]})
            for fid, fl in flows.items():
                if e["id"] in refs_of(fl):
                    used.append({"kind": "flow", "id": fid, "version": fl["version"]})
            arts.append({"kind": kind, "id": e["id"], "version": e["version"], "locales": e["locales"], "extra": extra, "referenced_by": used})
    donor = "consultas"
    out = {
        "label": "fixture-baseline",
        "source": {"repo": "agent-core", "commit": head, "path": "tests/fixtures/registry-e2e"},
        "artifacts": arts,
        "agents": {k: v for k, v in agents.items()},
        "flows": {"consulta-pqr": flows["consulta-pqr"]},
        "donor": donor,
        "donor_release": load(fx / "releases" / "consultas-demo.yaml"),
    }
    a.out.parent.mkdir(parents=True, exist_ok=True)
    a.out.write_text(json.dumps(out, ensure_ascii=False, indent=1, sort_keys=True) + "\n", encoding="utf-8")
    print(f"wrote {a.out} ({len(arts)} artifacts, {len(agents)} agents)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

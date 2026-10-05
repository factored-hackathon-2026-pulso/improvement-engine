#!/usr/bin/env python3
"""Drift check between ToolDef fixtures (agent-core registry-e2e) and a tool provider's `GET /v1/tools` listing (tool-service).

The provider is the ORACLE for the tools it serves (tool-service README: "GET /v1/tools lists every tool with its declaration").
This script never fetches unless asked: with `--provider <file>` it reads a saved listing; with `--provider-url <base>` it performs
ONE read-only `GET <base>/v1/tools` with the consumer token from the environment variable named by `--token-env` (the value is never
printed). Fixtures come from a directory of ToolDef YAML files (needs PyYAML) or from a JSON snapshot `{"tools": [...]}`.

    python check_tool_alignment.py --fixtures <dir|snapshot.json> --provider <listing.json> [--baseline known_drift.json]
                                   [--align-out aligned_tool_defs.json] [--json]

Finding kinds: source_missing, source_mismatch, risk_class_mismatch, min_auth_level_mismatch, idempotent_mismatch, version_mismatch,
args_property_missing / args_property_extra / args_property_changed, args_required_mismatch, args_additional_properties_mismatch,
not_in_provider (a fixture tool the provider does not serve and that is not agent-owned), not_in_fixture (a served tool the
fixtures lack). `compute` tools and the runtime's own tools (AGENT_OWNED) are the agent runtime's, not the provider's: listed, not findings.

Exit codes: 0 no new drift (and no stale baseline entry), 1 new drift or a stale baseline entry, 2 unusable input.
The baseline (`known_drift.json`) records drift already reported upstream as `"<tool>:<kind>": "<ask>"`; a baseline entry that is no
longer true is reported as stale so the baseline shrinks when upstream fixes it.
"""
from __future__ import annotations

import argparse
import json
import os
import sys
import urllib.error
import urllib.request
from collections.abc import Iterable, Mapping
from pathlib import Path

# Tools the agent runtime implements itself (tool-service README, "Not here"): never served by a provider.
AGENT_OWNED = frozenset({"obtener_handoff", "leer_transcript", "seleccionar", "convertir_moneda"})
SCALARS = ("version", "risk_class", "min_auth_level", "idempotent")


class InputError(Exception):
    """Unusable fixtures, listing or baseline: never reported as a pass."""


def _strip_version(tool_id: str) -> str:
    return tool_id.split("@", 1)[0]


def _norm_schema(schema: Mapping | None) -> dict:
    s = schema or {}
    return {"properties": dict(s.get("properties") or {}), "required": sorted(s.get("required") or []),
            "additionalProperties": s.get("additionalProperties")}


def compare(fixtures: Iterable[Mapping], provider: Mapping) -> dict:
    fx = {_strip_version(t["id"]): t for t in fixtures}
    pv = {_strip_version(t["id"]): t for t in provider["tools"]}
    findings: list[dict] = []
    agent_owned: list[str] = []

    def add(tool: str, kind: str, fixture, prov) -> None:
        findings.append({"tool": tool, "kind": kind, "fixture": fixture, "provider": prov})

    for tid in sorted(fx):
        f, p = fx[tid], pv.get(tid)
        if p is None:
            if tid in AGENT_OWNED or f.get("risk_class") == "compute":
                agent_owned.append(tid)
            else:
                add(tid, "not_in_provider", f.get("source"), None)
            continue
        if f.get("source") != p.get("source"):
            add(tid, "source_missing" if not f.get("source") else "source_mismatch", f.get("source"), p.get("source"))
        for k in SCALARS:
            if f.get(k) != p.get(k):
                add(tid, f"{k}_mismatch", f.get(k), p.get(k))
        fs, ps = _norm_schema(f.get("args_schema")), _norm_schema(p.get("args_schema"))
        for name in sorted(set(ps["properties"]) - set(fs["properties"])):
            add(tid, "args_property_missing", None, name)
        for name in sorted(set(fs["properties"]) - set(ps["properties"])):
            add(tid, "args_property_extra", name, None)
        for name in sorted(set(fs["properties"]) & set(ps["properties"])):
            if fs["properties"][name] != ps["properties"][name]:
                add(tid, "args_property_changed", fs["properties"][name], ps["properties"][name])
        if fs["required"] != ps["required"]:
            add(tid, "args_required_mismatch", fs["required"], ps["required"])
        if fs["additionalProperties"] != ps["additionalProperties"]:
            add(tid, "args_additional_properties_mismatch", fs["additionalProperties"], ps["additionalProperties"])
    for tid in sorted(set(pv) - set(fx)):
        add(tid, "not_in_fixture", None, pv[tid].get("source"))
    return {"checked": len(set(fx) & set(pv)), "findings": findings, "agent_owned": sorted(agent_owned)}


def keys(report: Mapping) -> set[str]:
    return {f"{f['tool']}:{f['kind']}" for f in report["findings"]}


def gate(report: Mapping, baseline: Mapping[str, str]) -> dict:
    got, known = keys(report), set(baseline)
    new, stale = sorted(got - known), sorted(known - got)
    return {"ok": not new and not stale, "new": new, "stale": stale, "known": sorted(got & known)}


def align(fixtures: Iterable[Mapping], provider: Mapping) -> dict:
    """The fixtures as they must read once aligned with the provider: for every tool the provider serves, its source, version, risk
    class, auth level, idempotence and args schema; the fixture's prose (`description`) is kept. Agent-owned tools stay as they are;
    served tools the fixtures lack are added (`provider_only`). Pure data: nothing upstream is touched."""
    fx = {_strip_version(t["id"]): dict(t) for t in fixtures}
    pv = {_strip_version(t["id"]): t for t in provider["tools"]}
    defs: dict[str, dict] = {}
    prov: dict[str, str] = {}
    for tid in sorted(set(fx) | set(pv)):
        f, p = fx.get(tid), pv.get(tid)
        if p is None:
            defs[tid] = f  # type: ignore[assignment]
            prov[tid] = "agent_owned" if (tid in AGENT_OWNED or (f or {}).get("risk_class") == "compute") else "not_served"
            continue
        d = dict(f) if f else {"id": tid}
        for k in ("version", "risk_class", "min_auth_level", "idempotent", "source", "args_schema"):
            d[k] = json.loads(json.dumps(p[k]))
        defs[tid] = d
        prov[tid] = "aligned_to_provider" if f else "provider_only"
    return {"tool_defs": defs, "provenance": prov}


def load_provider(path: Path) -> dict:
    try:
        data = json.loads(Path(path).read_text("utf-8"))
    except (OSError, ValueError) as e:
        raise InputError(f"provider listing {path}: {e}") from e
    if not isinstance(data, dict) or not isinstance(data.get("tools"), list) or not all(isinstance(t, dict) and "id" in t for t in data["tools"]):
        raise InputError(f"provider listing {path}: expected {{\"tools\": [ToolInfo...]}}")
    return data


def fetch_provider(base: str, token_env: str) -> dict:
    token = os.environ.get(token_env, "")
    if not token:
        raise InputError(f"environment variable {token_env} is empty")
    req = urllib.request.Request(base.rstrip("/") + "/v1/tools", headers={"Authorization": f"Bearer {token}"}, method="GET")
    try:
        with urllib.request.urlopen(req, timeout=15) as r:  # noqa: S310 - operator-supplied local URL, read-only GET
            data = json.loads(r.read().decode("utf-8"))
    except (urllib.error.URLError, OSError, ValueError) as e:
        raise InputError(f"GET /v1/tools failed: {type(e).__name__}") from e
    if not isinstance(data, dict) or not isinstance(data.get("tools"), list):
        raise InputError("GET /v1/tools: unexpected shape")
    return data


def load_fixtures(path: Path) -> list[dict]:
    p = Path(path)
    if p.is_dir():
        try:
            import yaml
        except ImportError as e:  # pragma: no cover - environment dependent
            raise InputError("reading a directory of ToolDef YAML needs PyYAML (or pass a JSON snapshot)") from e
        out = []
        for f in sorted(p.glob("*.yaml")):
            try:
                out.append(yaml.safe_load(f.read_text("utf-8")))
            except (OSError, yaml.YAMLError) as e:
                raise InputError(f"{f}: {e}") from e
        return out
    try:
        data = json.loads(p.read_text("utf-8"))
    except (OSError, ValueError) as e:
        raise InputError(f"fixtures {p}: {e}") from e
    tools = data.get("tools") if isinstance(data, dict) else None
    if not isinstance(tools, list):
        raise InputError(f"fixtures {p}: expected {{\"tools\": [...]}}")
    return tools


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--fixtures", type=Path, required=True)
    src = ap.add_mutually_exclusive_group(required=True)
    src.add_argument("--provider", type=Path)
    src.add_argument("--provider-url")
    ap.add_argument("--token-env", default="TOOL_SERVICE_CONSUMER_TOKEN")
    ap.add_argument("--baseline", type=Path)
    ap.add_argument("--align-out", type=Path)
    ap.add_argument("--json", action="store_true")
    a = ap.parse_args(argv)
    try:
        fixtures = load_fixtures(a.fixtures)
        provider = load_provider(a.provider) if a.provider else fetch_provider(a.provider_url, a.token_env)
        baseline: dict[str, str] = {}
        if a.baseline:
            try:
                baseline = json.loads(a.baseline.read_text("utf-8"))["entries"]
            except (OSError, ValueError, KeyError) as e:
                raise InputError(f"baseline {a.baseline}: {e}") from e
    except InputError as e:
        print(f"check_tool_alignment: input error: {e}", file=sys.stderr)
        return 2
    report = compare(fixtures, provider)
    verdict = gate(report, baseline)
    if a.align_out:
        a.align_out.write_text(json.dumps(align(fixtures, provider), indent=1, sort_keys=True, ensure_ascii=False) + "\n", "utf-8")
    if a.json:
        print(json.dumps({"report": report, "gate": verdict}, indent=1, sort_keys=True))
    else:
        for f in report["findings"]:
            tag = "known" if f"{f['tool']}:{f['kind']}" in baseline else "NEW"
            print(f"{tag} {f['tool']} {f['kind']}: fixture={f['fixture']!r} provider={f['provider']!r}")
        for s in verdict["stale"]:
            print(f"STALE baseline entry {s}: no longer true (fixed upstream?) - remove it")
        print(f"checked {report['checked']} tools; agent-owned {report['agent_owned']}; "
              f"{len(verdict['known'])} known, {len(verdict['new'])} new, {len(verdict['stale'])} stale")
    return 0 if verdict["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())

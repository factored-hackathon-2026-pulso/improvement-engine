#!/usr/bin/env python
"""Cross-check of the published contract against the platform-sim `bridge_mock` schemas and source.

    python divergence.py            # rewrite divergences/mock-vs-contract.json
    python divergence.py --check    # exit 1 when the mock (or the contract) moved and the report is stale

Pure data comparison (property sets, required lists, enums, purposes, error codes); the behavioural differences are
recorded as known-different cases in conformance/known_different.py."""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path
from typing import Any

HERE = Path(__file__).resolve().parent
MOCK = HERE.parent / "platform-sim" / "bridge_mock"
OUT = HERE / "divergences" / "mock-vs-contract.json"

# contract schema -> mock schema (same wire concept)
PAIRS = {
    "CoreTaskInvocation": "CoreTaskInvocation", "CoreTaskReceipt": "CoreTaskReceipt",
    "CoreCredentialIssueRequest": "CoreCredentialIssueRequest", "CoreCredentialIssue": "CoreCredentialIssue",
    "CoreVersion": "CoreVersion", "AliasState": "AliasState",
    "CoreAuthoringDryRunRequest": "CoreAuthoringDryRunRequest", "CoreAuthoringDryRun": "CoreAuthoringDryRun",
    "ErrorEnvelope": "BridgeError", "ReadRunResult": "ReadRunResult",
}
# contract route id -> mock ROUTE_PURPOSE key
ROUTE_KEYS = {"invoke": "invoke", "read_task": "read", "read_alias": "alias", "authoring_dry_run": "dryrun",
              "version": "version", "issue_credential": "issue"}


def _load(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def _props(schema: dict[str, Any]) -> dict[str, Any]:
    return dict(schema.get("properties", {}))


def schema_diff(contract: dict[str, Any], mock: dict[str, Any]) -> dict[str, Any]:
    cp, mp = _props(contract), _props(mock)
    both = sorted(set(cp) & set(mp))
    enum_diffs = {}
    for k in both:
        ce, me = cp[k].get("enum"), mp[k].get("enum")
        if ce is not None and me is not None and sorted(ce) != sorted(me):
            enum_diffs[k] = {"contract": ce, "mock": me}
    out: dict[str, Any] = {
        "only_in_contract": sorted(set(cp) - set(mp)), "only_in_mock": sorted(set(mp) - set(cp)),
        "required_only_in_contract": sorted(set(contract.get("required", [])) - set(mock.get("required", []))),
        "required_only_in_mock": sorted(set(mock.get("required", [])) - set(contract.get("required", []))),
        "additional_properties": ({"contract": contract.get("additionalProperties"),
                                   "mock": mock.get("additionalProperties")}
                                  if contract.get("additionalProperties") != mock.get("additionalProperties") else None),
        "enum_diffs": enum_diffs}
    return {k: v for k, v in out.items() if v not in ([], {}, None) and v != {"contract": None, "mock": None}}


def mock_codes() -> list[str]:
    text = (MOCK / "app.py").read_text(encoding="utf-8")
    codes = set(re.findall(r"BridgeError\(\s*\d{3}\s*,\s*\"([a-z_]+)\"", text))
    codes |= set(re.findall(r"problem\(\s*\d{3}\s*,\s*\"([a-z_]+)\"", text))
    return sorted(codes)


def mock_purposes() -> dict[str, str]:
    text = (MOCK / "app.py").read_text(encoding="utf-8")
    block = re.search(r"ROUTE_PURPOSE\s*=\s*\{(.*?)\}", text, re.DOTALL)
    assert block, "ROUTE_PURPOSE not found in the mock"
    return dict(re.findall(r"\"(\w+)\":\s*\"(\w+)\"", block.group(1)))


def mock_routes() -> list[str]:
    text = (MOCK / "app.py").read_text(encoding="utf-8")
    return sorted(f"{m.upper()} {p}" for m, p in re.findall(r"@app\.(get|post)\(\"(/internal/v1[^\"]*)\"", text))


def report() -> dict[str, Any]:
    contract = _load(HERE / "contract.json")
    schemas = {n: _load(HERE / "schemas" / f"{n}.schema.json") for n in PAIRS}
    mock_dir = MOCK / "schemas"
    out: dict[str, Any] = {"schemas": {}, "purposes": {}, "routes": {}, "error_codes": {}}
    for c, m in PAIRS.items():
        mp = mock_dir / f"{m}.schema.json"
        out["schemas"][c] = schema_diff(schemas[c], _load(mp)) if mp.is_file() else {"mock": "no such schema"}
    mp_map = mock_purposes()
    for r in contract["routes"]:
        key = ROUTE_KEYS.get(r["id"])
        mock_p = mp_map.get(key) if key else None
        if mock_p is None or [mock_p] != r["purposes"]:
            out["purposes"][r["id"]] = {"contract": r["purposes"], "mock": mock_p}
    cpaths = {f"{r['method']} /internal/v1{r['path']}" for r in contract["routes"]}
    mroutes = set(mock_routes())
    out["routes"] = {"only_in_contract": sorted(cpaths - mroutes), "only_in_mock": sorted(mroutes - cpaths)}
    wire = set(contract["error_codes"]["wire"])
    mc = mock_codes()
    out["error_codes"] = {"mock_codes_not_in_contract": sorted(c for c in mc if f"pulso:{c}" not in wire),
                          "mock_codes_in_contract_only_with_prefix": sorted(c for c in mc if f"pulso:{c}" in wire)}
    return out


def render() -> str:
    return json.dumps(report(), indent=2, sort_keys=True) + "\n"


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--check", action="store_true")
    args = ap.parse_args(argv)
    text = render()
    if args.check:
        if not OUT.is_file() or OUT.read_text(encoding="utf-8") != text:
            print("divergence.py --check: report is stale; run python bridge-contract/divergence.py", file=sys.stderr)
            return 1
        return 0
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(text, encoding="utf-8", newline="\n")
    print(f"wrote {OUT}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

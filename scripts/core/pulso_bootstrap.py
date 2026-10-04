"""pulso-bootstrap (CAP-48): lists the seeded-world assets and their digests as bootstrap-report/v1.

  --plan/--offline (default): computed from the repo, no Core. Entity digests come from agent-core-assets/expected-state.json
      and are cross-checked (world file digests and expected-state digest vs manifest.yaml); any mismatch is `drift`.
  --apply --core-url URL: verifies against a running Core that every manifest release exists with the expected hash
      (GET /v1/registry/releases/{rid}). Seeding itself is the core-seed service (local/core/init/seed_assets.py).
Exit 0 ok, 1 drift or Core mismatch. Python 3.12 + PyYAML.
"""
from __future__ import annotations

import argparse
import json
import sys
import urllib.error
import urllib.request
from collections.abc import Callable
from pathlib import Path
from typing import Any

import yaml

REPO = Path(__file__).resolve().parents[2]
DEFAULT_ASSETS = REPO / "agent-core-assets"
Transport = Callable[[str, str], tuple[int, Any]]


def _assetcheck(root: Path):
    sys.path.insert(0, str(root / "tools"))
    import assetcheck  # type: ignore[import-not-found]
    return assetcheck


def build_plan(root: Path = DEFAULT_ASSETS) -> dict[str, Any]:
    ac = _assetcheck(root)
    state = json.loads((root / "expected-state.json").read_text(encoding="utf-8"))
    manifest = yaml.safe_load((root / "manifest.yaml").read_text(encoding="utf-8"))
    assets = [{"agent": agent, "kind": e["kind"], "id": e["id"], "version": e["version"], "digest": e["content_hash"]}
              for agent, st in sorted(state.items()) for e in st["entities"]]
    worlds = {w.name: ac.files_digest(w) for w in ac.worlds_of(root)}
    drift: list[str] = []
    for name, d in worlds.items():
        if manifest.get("worlds", {}).get(name, {}).get("files_digest") != d:
            drift.append(f"world:{name}")
    if manifest.get("expected_state_digest") != ac.expected_state_digest(state):
        drift.append("expected-state")
    if {a: v["release_id"] for a, v in state.items()} != manifest.get("release_ids"):
        drift.append("release_ids")
    return {"schema": "bootstrap-report/v1", "mode": "plan", "ok": not drift, "assets": assets,
            "worlds": worlds, "drift": drift}


def apply(root: Path, core_url: str, transport: Transport) -> dict[str, Any]:
    report = build_plan(root)
    report.update(mode="apply", core_url=core_url, releases=[])
    state = json.loads((root / "expected-state.json").read_text(encoding="utf-8"))
    for agent, st in sorted(state.items()):
        rid, want = st["release_id"], st["release_hash"]
        try:
            code, body = transport("GET", f"{core_url.rstrip('/')}/v1/registry/releases/{rid}")
        except Exception:  # noqa: BLE001 - never leak transport details
            code, body = -1, {}
        if code == 404:
            status = "missing"
        elif code != 200:
            status = "error"
        else:
            status = "verified" if body.get("release_hash") == want else "hash_mismatch"
        report["releases"].append({"agent": agent, "release_id": rid, "release_hash": want, "status": status})
    report["ok"] = report["ok"] and all(r["status"] == "verified" for r in report["releases"])
    return report


def http_transport(method: str, url: str) -> tuple[int, Any]:  # pragma: no cover - live Core only
    try:
        with urllib.request.urlopen(urllib.request.Request(url, method=method), timeout=10) as r:
            return r.status, json.loads(r.read() or b"{}")
    except urllib.error.HTTPError as exc:
        return exc.code, {}


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(prog="pulso-bootstrap")
    g = p.add_mutually_exclusive_group()
    g.add_argument("--plan", "--offline", dest="plan", action="store_true")
    g.add_argument("--apply", action="store_true")
    p.add_argument("--core-url")
    p.add_argument("--assets", type=Path, default=DEFAULT_ASSETS)
    p.add_argument("--out", type=Path, help="write bootstrap-report.json here (default: stdout)")
    a = p.parse_args(argv)
    if a.apply:
        if not a.core_url:
            p.error("--apply needs --core-url")
        report = apply(a.assets, a.core_url, http_transport)
    else:
        report = build_plan(a.assets)
    text = json.dumps(report, indent=2, sort_keys=True) + "\n"
    if a.out:
        a.out.write_text(text, encoding="utf-8")
    else:
        sys.stdout.write(text)
    return 0 if report["ok"] else 1


if __name__ == "__main__":
    sys.exit(main())

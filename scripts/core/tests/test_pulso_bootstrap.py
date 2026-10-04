"""BOOT (CAP-48): pulso-bootstrap and bootstrap-report/v1. Offline plus a fake transport; no Core needed."""
from __future__ import annotations

import json
import sys
from pathlib import Path

import pytest

jsonschema = pytest.importorskip("jsonschema")
HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
SCHEMA = json.loads((HERE.parent / "schemas" / "bootstrap-report.v1.schema.json").read_text(encoding="utf-8"))
ASSETS = HERE.parents[2] / "agent-core-assets"


def test_bootstrap_report_schema_fails_when_empty():
    with pytest.raises(jsonschema.ValidationError):
        jsonschema.validate({}, SCHEMA)
    with pytest.raises(jsonschema.ValidationError):
        jsonschema.validate({"schema": "bootstrap-report/v1", "mode": "plan", "assets": [], "worlds": {}}, SCHEMA)


def test_plan_matches_expected_state_and_validates():
    import pulso_bootstrap as pb
    report = pb.build_plan(ASSETS)
    jsonschema.validate(report, SCHEMA)
    expected = json.loads((ASSETS / "expected-state.json").read_text(encoding="utf-8"))
    n = sum(len(v["entities"]) for v in expected.values())
    assert len(report["assets"]) == n > 0
    assert report["drift"] == []
    ids = {(a["agent"], a["kind"], a["id"]): a["digest"] for a in report["assets"]}
    for agent, st in expected.items():
        for e in st["entities"]:
            assert ids[(agent, e["kind"], e["id"])] == e["content_hash"]
    assert set(report["worlds"]) >= {"attention-task", "attention-demo"}


def test_plan_reports_drift_when_world_changes(tmp_path):
    import shutil

    import pulso_bootstrap as pb
    root = tmp_path / "a"
    shutil.copytree(ASSETS, root, ignore=shutil.ignore_patterns("__pycache__", ".pytest_cache"))
    f = next((root / "worlds" / "attention-task" / "agents").iterdir())
    f.write_bytes(f.read_bytes() + b"\n# drift\n")
    assert pb.build_plan(root)["drift"]


def test_apply_with_fake_transport_verifies_releases():
    import pulso_bootstrap as pb
    expected = json.loads((ASSETS / "expected-state.json").read_text(encoding="utf-8"))
    by_rid = {v["release_id"]: v["release_hash"] for v in expected.values()}
    calls = []

    def ok(method, url):
        calls.append((method, url))
        rid = url.rsplit("/", 1)[1]
        return 200, {"release_id": rid, "release_hash": by_rid[rid]}

    rep = pb.apply(ASSETS, "http://core.test", ok)
    jsonschema.validate(rep, SCHEMA)
    assert rep["mode"] == "apply" and rep["ok"] and len(calls) == len(by_rid)
    assert all(u.startswith("http://core.test/v1/registry/releases/") for _, u in calls)

    def missing(method, url):
        return 404, {}

    bad = pb.apply(ASSETS, "http://core.test", missing)
    assert not bad["ok"] and {r["status"] for r in bad["releases"]} == {"missing"}

    def wrong(method, url):
        return 200, {"release_hash": "0" * 64}

    assert {r["status"] for r in pb.apply(ASSETS, "http://c", wrong)["releases"]} == {"hash_mismatch"}

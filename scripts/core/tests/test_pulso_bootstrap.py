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


def _copy(tmp_path):
    import shutil
    root = tmp_path / "a"
    shutil.copytree(ASSETS, root, ignore=shutil.ignore_patterns("__pycache__", ".pytest_cache"))
    return root


def test_plan_drift_on_manifest_and_expected_state_and_exit_code(tmp_path, capsys):
    import pulso_bootstrap as pb
    root = _copy(tmp_path)
    m = root / "manifest.yaml"
    m.write_text(m.read_text(encoding="utf-8").replace("expected_state_digest:", "expected_state_digest: 0", 1), encoding="utf-8")
    assert "expected-state" in pb.build_plan(root)["drift"]
    assert pb.main(["--plan", "--assets", str(root)]) == 1
    root2 = _copy(tmp_path / "b")
    es = root2 / "expected-state.json"
    st = json.loads(es.read_text(encoding="utf-8"))
    next(iter(st.values()))["entities"][0]["content_hash"] = "0" * 64
    es.write_text(json.dumps(st, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    assert pb.build_plan(root2)["drift"]


def test_plan_ignores_crlf_but_catches_content(tmp_path):
    import pulso_bootstrap as pb
    root = _copy(tmp_path)
    f = next((root / "worlds" / "attention-task" / "agents").iterdir())
    f.write_bytes(f.read_bytes().replace(b"\r\n", b"\n").replace(b"\n", b"\r\n"))
    assert pb.build_plan(root)["drift"] == []


@pytest.mark.parametrize("url", ["ftp://h", "file:///x", "http://user:pw@h", "http://h?token=1", "http://h/#f", "h"])
def test_apply_rejects_bad_core_url_without_echo(url, capsys):
    import pulso_bootstrap as pb
    with pytest.raises(SystemExit):
        pb.main(["--apply", "--core-url", url])
    assert "pw" not in capsys.readouterr().err


def test_apply_only_gets_and_report_has_no_secret():
    import pulso_bootstrap as pb
    seen = []

    def boom(method, url):
        seen.append(method)
        raise RuntimeError("Bearer SECRET")

    rep = pb.apply(ASSETS, "http://c", boom)
    assert set(seen) == {"GET"} and "SECRET" not in json.dumps(rep)
    assert {r["status"] for r in rep["releases"]} == {"error"}
    jsonschema.validate(rep, SCHEMA)


def test_schema_rejects_inconsistent_reports():
    import pulso_bootstrap as pb
    good = pb.build_plan(ASSETS)
    for mutate in (
        lambda r: r.update(ok=True, drift=["x"]),
        lambda r: r.update(mode="apply"),
        lambda r: r.update(extra=1),
        lambda r: r.update(mode="apply", core_url="http://c", releases=[
            {"agent": "a", "release_id": "r", "release_hash": "zz", "status": "verified"}]),
    ):
        bad = json.loads(json.dumps(good))
        mutate(bad)
        with pytest.raises(jsonschema.ValidationError):
            jsonschema.validate(bad, SCHEMA)

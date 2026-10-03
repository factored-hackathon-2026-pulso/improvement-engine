"""First RED of the 'derived revision' package: the candidate comes only from a supported hypothesis and the bounded automatic revision is steered
by the structured guard breach of the previous evaluation (mutate-dataset tests: another planted breach -> another revision)."""
import json

import pytest

from pulso_demo import analysis, dataset, driver


def setup(**build_kw):
    conn = dataset.build(**build_kw)
    sc = analysis.scout(conn)
    ver = analysis.verify(conn, sc["hypotheses"])
    return conn, sc, ver


def test_candidate_is_derived_from_the_supported_hypothesis_scope():
    conn, sc, ver = setup()
    c1 = analysis.design_candidate(conn, sc, ver)
    assert c1["scope"] == "flow:transfer_limit" and c1["hypothesis_key"] == "transfer_limit/otp_verify" and c1["exclude_segments"] == []
    assert c1["max_retries"] == 5 and c1["derived_from"]["query"]["sql"]  # broad over the hypothesis flow, bounded by the policy ceiling


def test_no_supported_hypothesis_means_no_candidate():
    conn, sc, ver = setup()
    refuted = {**ver, "assessments": [{**a, "verdict": "refuted"} for a in ver["assessments"]]}
    assert analysis.design_candidate(conn, sc, refuted) is None


def test_candidate_outside_the_supported_scope_is_refused():
    conn, sc, ver = setup()
    c1 = analysis.design_candidate(conn, sc, ver)
    with pytest.raises(analysis.UnsupportedCandidate):
        analysis.check_supported({**c1, "scope": "flow:address_change"}, sc, ver)
    with pytest.raises(analysis.UnsupportedCandidate):
        analysis.check_supported({**c1, "scope": "all_flows"}, sc, ver)


def test_guard_breach_reports_segment_direction_and_magnitude_from_the_data():
    conn, sc, ver = setup()
    j = analysis.judge(conn, analysis.design_candidate(conn, sc, ver))
    imp = j["improvement"]
    assert imp["status"] == "fail" and imp["reason_code"] == "guard_breach"
    b = imp["breach"]
    assert b["direction"] == "above_limit" and b["limit"] == analysis.GUARD_MAX_EXPOSURE and b["observed"] > b["limit"]
    assert b["magnitude"] == pytest.approx(b["observed"] - b["limit"], abs=1e-4)
    assert b["affected_segment"] == {"dimension": "device", "value": b["segments"][0]["value"]}
    assert sum(s["exposure"] for s in b["segments"]) == pytest.approx(b["observed"], abs=1e-4)


@pytest.mark.parametrize("hot", ["web", "mobile"])
def test_different_planted_breach_segment_gives_a_different_revision(hot):
    conn, sc, ver = setup(risk_hot=("device", hot))
    c1 = analysis.design_candidate(conn, sc, ver)
    j1 = analysis.judge(conn, c1)
    assert j1["improvement"]["breach"]["affected_segment"] == {"dimension": "device", "value": hot}
    c2 = analysis.revise(conn, c1, j1, sc, ver)
    assert c2["exclude_segments"] == [{"dimension": "device", "value": hot}] and c2["scope"] == c1["scope"] and c2["max_retries"] == c1["max_retries"]
    rev = c2["revision"]
    assert rev["of"] == "cand-1" and rev["trigger"]["reason_code"] == "guard_breach" and hot in rev["rationale"]
    assert rev["delta"] == [{"field": "exclude_segments", "from": [], "to": [{"dimension": "device", "value": hot}]}]
    assert analysis.judge(conn, c2)["improvement"]["status"] == "pass"


def test_insufficient_lift_is_not_steerable_by_narrowing(monkeypatch):
    monkeypatch.setattr(analysis, "GUARD_MAX_EXPOSURE", 1.0)
    monkeypatch.setattr(analysis, "MIN_LIFT", 0.5)
    conn, sc, ver = setup()
    c1 = analysis.design_candidate(conn, sc, ver)
    j1 = analysis.judge(conn, c1)
    assert j1["improvement"]["reason_code"] == "insufficient_lift"
    with pytest.raises(analysis.RevisionExhausted):
        analysis.revise(conn, c1, j1, sc, ver)


def run(tmp_path, *extra):
    rc = driver.main(["--out", str(tmp_path), "--offline", *extra])
    return rc, json.loads((tmp_path / "demo-report.json").read_text("utf-8")), json.loads((tmp_path / "world.json").read_text("utf-8"))


def test_world_records_failure_then_rationale_then_delta(tmp_path, monkeypatch):
    real = dataset.build
    monkeypatch.setattr(dataset, "build", lambda *a, **k: real(*a, **({"risk_hot": ("device", "web")} if not k.get("post") else {}), **k))
    rc, rep, world = run(tmp_path)
    a1, a2 = world["demo"]["attempts"][:2]
    assert a1["improvement"]["reason_code"] == "guard_breach" and a1["failure"]["affected_segment"] == {"dimension": "device", "value": "web"}
    assert a2["revision_of"] == "cand-1" and a2["revision"]["trigger"]["breach"]["affected_segment"]["value"] == "web"
    assert "web" in a2["revision"]["rationale"] and a2["revision"]["delta"][0]["field"] == "exclude_segments"
    assert rep["outcome"] == "ok" and {s["n"]: s["outcome"] for s in rep["steps"]}[7] == "shown"


def test_unfixable_breach_stops_bounded_with_failed_step_7_and_no_human(tmp_path, monkeypatch):
    monkeypatch.setattr(analysis, "GUARD_MAX_EXPOSURE", 0.0001)
    rc, rep, world = run(tmp_path)
    assert rc == 0 and len(rep["attempts"]) == 3 and rep["outcome"] == "no_candidate_passed_gates" and rep["human"] is None  # 1 + max_revisions(2)
    assert {s["n"]: s["outcome"] for s in rep["steps"]}[7] == "failed" and {s["n"]: s["outcome"] for s in rep["steps"]}[8] == "pending"
    assert [a["revision"]["action"] for a in rep["attempts"][1:]] == ["exclude_segment", "limit_retries"]
    rc, rep1, _ = run(tmp_path / "one", "--max-revisions", "1")
    assert len(rep1["attempts"]) == 2


def test_no_supported_otp_hypothesis_is_no_opportunity_and_hold(tmp_path, monkeypatch):
    real = dataset.build
    monkeypatch.setattr(dataset, "build", lambda *a, **k: real(*a, **{**k, "plant": False}))
    rc, rep, world = run(tmp_path)
    assert rc == 0 and rep["outcome"] == "no_opportunity" and rep["attempts"] == [] and world["gates"]["combined"]["decision"] == "hold"


def test_unsteerable_failure_stops_without_revision(tmp_path, monkeypatch):
    monkeypatch.setattr(analysis, "GUARD_MAX_EXPOSURE", 1.0)
    monkeypatch.setattr(analysis, "MIN_LIFT", 0.5)
    rc, rep, world = run(tmp_path)
    assert len(rep["attempts"]) == 1 and rep["outcome"] == "no_candidate_passed_gates" and {s["n"]: s["outcome"] for s in rep["steps"]}[7] == "failed"


def test_no_breach_no_revision(tmp_path, monkeypatch):
    monkeypatch.setattr(analysis, "GUARD_MAX_EXPOSURE", 1.0)
    rc, rep, world = run(tmp_path)
    assert len(rep["attempts"]) == 1 and rep["attempts"][0]["revision"] is None and rep["attempts"][0]["failure"] is None

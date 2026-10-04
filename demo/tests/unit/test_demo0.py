"""DEMO-0 entry point: the summary lists every stand-in FIRST and refuses a report that fails the G1 check(). No containers."""
import copy
import json
import shutil
import subprocess
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "scripts" / "gov"))
from pulso_demo import demo0  # noqa: E402

REPORT = ROOT / "docs" / "reports" / "gates" / "gt0" / "report.json"


@pytest.fixture()
def report():
    return json.loads(REPORT.read_text(encoding="utf-8"))


def test_summary_states_every_double_before_any_step(report):
    text = demo0.summarize(report)
    first_step = min(text.index(s["id"], text.index("STEPS")) for s in report["steps"])
    head = text[:first_step]
    assert "NOT REAL" in head
    for d in report["doubles"]:
        lines = [l for l in head.splitlines() if l.startswith(f"- {d['part']}: {d['status']}")]   # a part may appear twice
        assert lines, f"double {d['part']}={d['status']} not listed before the steps"
    assert "DEMO-0" in text and "quality_claims: forbidden" in text


def test_summary_labels_every_step(report):
    text = demo0.summarize(report)
    steps = text[text.index("STEPS"):]
    for s in report["steps"]:
        row = [l for l in steps.splitlines() if s["id"] in l]
        assert row and s["status"] in row[0] and s["data_class"] in row[0]


def test_summary_refuses_a_report_failing_g1_check(report):
    bad = copy.deepcopy(report)
    scout = next(x for x in bad["steps"] if x["id"] == "scout")
    scout["status"] = "real"   # claims real for an agent_roleplay step
    from pulso_demo.demo0 import Demo0Refused
    with pytest.raises(Demo0Refused):
        demo0.summarize(bad)


def test_summary_refuses_when_a_double_is_missing(report):
    bad = copy.deepcopy(report)
    bad["doubles"] = [d for d in bad["doubles"] if d["part"] != "jev"]
    with pytest.raises(demo0.Demo0Refused):
        demo0.summarize(bad)


def test_live_and_realcore_notes_do_not_spawn_anything():
    live = demo0.live_note()
    assert "roleplay-llm/RUNBOOK.md" in live and "does not spawn" in live
    core = demo0.realcore_note()
    assert "e2e-core/run.ps1" in core and "one stack" in core and "torn down" in core


@pytest.mark.skipif(shutil.which("pwsh") is None, reason="pwsh not available")
def test_entry_point_summarises_an_existing_report(tmp_path):
    ps1 = ROOT / "demo" / "run-demo0.ps1"
    r = subprocess.run(["pwsh", "-NoProfile", "-File", str(ps1), "-FromReport", str(REPORT), "-Live", "-RealCore"],
                       capture_output=True, text=True)
    assert r.returncode == 0, r.stdout + r.stderr
    assert r.stdout.index("NOT REAL") < r.stdout.index("STEPS")
    assert "RUNBOOK.md" in r.stdout and "e2e-core/run.ps1" in r.stdout

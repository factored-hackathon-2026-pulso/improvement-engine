"""Thread over an E0-shaped package (SYNTHETIC fixture here; real E0 is only read at runtime in a live window).

The lab is fed by the pyarrow feeder with an ephemeral salt; the SMAP ending is honest `unlinked`
(no_exact_supported_flow_mapping), steps 5-10 are not_exercised, and nothing raw reaches any file."""
import importlib.util
import os
import sys
from pathlib import Path

import pytest

pytest.importorskip("pyarrow")
from claude_standin import ed0_detect as ed0  # noqa: E402
from claude_standin import thread01 as T  # noqa: E402

ROOT = Path(__file__).resolve().parents[3]
EXE = os.environ.get("ED0_RUNNER_EXE", "D:/cargo-targets/claude-ed0/debug/improvement-engine.exe")
pytestmark = pytest.mark.skipif(not os.path.exists(EXE), reason="runner exe not built")
LABELS = {"A": "SIGRAW-alpha-lookup", "B": "SIGRAW-beta-lookup"}


def _er():
    spec = importlib.util.spec_from_file_location("engine_run_e0", ROOT / "contracts" / "engine-run" / "engine_run.py")
    m = importlib.util.module_from_spec(spec)
    sys.modules["engine_run_e0"] = m
    spec.loader.exec_module(m)
    return m


@pytest.fixture(scope="module")
def run(tmp_path_factory):
    base = tmp_path_factory.mktemp("e0thread")
    pkg = base / "e0"
    ed0.write_synthetic_e0(str(pkg), LABELS, cases=120, arranque=60)
    q = base / "queue"
    for d in ("requests", "responses"):
        (q / d).mkdir(parents=True)
    wd = base / "work"
    cfg = T.ThreadConfig(workdir=wd, exe=EXE, queue_dir=q, mode="record", e0_path=str(pkg), e0_arranque=60,
                         e0_min_support=5)
    return {"res": T.run_thread(cfg), "base": base, "wd": wd, "q": q}


def step(run, n, id_=None):
    return next(s for s in run["res"]["steps"] if s["n"] == n and (id_ is None or s["id"] == id_))


def test_first_red_no_raw_id_or_label_in_any_file_the_run_wrote(run):
    needles = (b"private-case", b"SIGRAW", b"alpha-lookup", b"beta-lookup", b"private-query", b"private-call")
    for p in (run["wd"], run["q"]):
        for f in p.rglob("*"):
            if f.is_file():
                blob = f.read_bytes()
                for n in needles:
                    assert n not in blob, (f.name, n)


def test_e0_window_steps_and_honest_unlinked_ending(run):
    assert [step(run, n)["status"] for n in (1, 2)] == ["stand-in", "real-narrow"]
    assert step(run, 3, "scout")["status"] == "agent_roleplay"
    smap = step(run, 4)["detail"]["smap"]
    assert smap["verdict"] == "unlinked" and smap["target"] is None
    for n in range(5, 11):
        s = step(run, n)
        assert s["status"] == "not_exercised", (n, s["status"], s.get("error"))
        assert "no_exact_supported_flow_mapping" in s["detail"]["reason"]


def test_e0_report_passes_g1_and_never_calls_e0_derived_data_generated(run):
    rep = run["res"]["report"]
    assert _er().check(rep) == []
    assert rep["quality_claims"] == "forbidden"
    classes = {s["n"]: s["data_class"] for s in rep["steps"] if s["status"] != "not_exercised"}
    assert classes == {1: "E0", 2: "E0", 3: "original-treated", 4: "original-treated"}
    assert {d["part"]: d["status"] for d in rep["doubles"]}["data.origin"] == "E0-treated-aggregates"


def test_category_is_a_hashed_group_never_a_label(run):
    cat = step(run, 4)["detail"]["category"]
    assert len(cat) == 16 and all(c in "0123456789abcdef" for c in cat)

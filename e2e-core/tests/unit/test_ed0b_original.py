"""ED0b RED/GREEN: original bank CSV dataset through the ED0/ED0L pipeline (`--source original`).

Fixtures are SYNTHETIC CSV-shaped (hive partitions like the original dataset). Real CSVs are never fixtures."""
import importlib.util
import json
import logging
import os
import sys
from pathlib import Path

import pytest

from claude_standin import ed0_lab as L
from claude_standin import ed0_original as O
from claude_standin import thread01 as T

ROOT = Path(__file__).resolve().parents[3]
EXE = os.environ.get("ED0_RUNNER_EXE", "D:/cargo-targets/claude-ed0/debug/improvement-engine.exe")
SALT = b"synthetic-original-salt-01"
MAP = {"table": "complaints", "case_col": "customer_id", "group_col": "complaint_category"}
NEEDLES = (b"RAWCUST", b"RAWCAT", b"alpha-cat", b"beta-cat", b"FREETEXT")


def _spec(path):
    s = importlib.util.spec_from_file_location("tps_scanner_orig", ROOT / "roleplay-llm/roleplay_llm/scanner.py")
    m = importlib.util.module_from_spec(s)
    sys.modules["tps_scanner_orig"] = m
    s.loader.exec_module(m)
    return m


TPS = _spec(None)


def _write(root, rows):
    d = Path(root) / "complaints" / "year=2025" / "month=01" / "day=01"
    d.mkdir(parents=True)
    lines = ["customer_id,complaint_category,description"] + [f"{c},{g},FREETEXT-secret" for c, g in rows]
    (d / "part-0.csv").write_text("\n".join(lines), encoding="utf-8-sig")
    return str(root)


@pytest.fixture
def pkg(tmp_path):
    rows = []
    for i in range(30):
        rows += [(f"RAWCUST-a{i:03d}", "RAWCAT-alpha-cat")] * (2 if i < 12 else 1)
    for i in range(14):
        rows += [(f"RAWCUST-b{i:03d}", "RAWCAT-beta-cat")] * (2 if i < 3 else 1)
    for i in range(3):
        rows += [(f"RAWCUST-c{i:03d}", "RAWCAT-tiny-cat")] * 2
    return _write(tmp_path / "orig", rows)


def test_first_red_no_raw_value_in_lab_scanner_payload_or_logs(pkg, tmp_path, caplog, capsys):
    caplog.set_level(logging.DEBUG)
    db = L.build_lab(tmp_path / "lab.sqlite", O.feed(pkg, SALT, **MAP), SALT, min_cell=L.K)
    rows = L.lab_query(db, L.METRIC, "w1")
    payload = {"goal": "g", "step": 1, "tools": [], "observations": [
        {"tool": "pulso/lab_query@1.0.0", "args": {"metric_id": L.METRIC, "window_id": "w1"}, "status": "ok",
         "result": rows, "error": None}]}
    assert TPS.scan_payload(payload).ok
    for blob in (Path(db).read_bytes(), json.dumps(payload).encode(), caplog.text.encode(), capsys.readouterr().out.encode()):
        for n in NEEDLES:
            assert n not in blob
    assert len(rows["rows"]) == 1  # only alpha has numerator and complement >= k


def test_feed_keys_are_pseudonymous_and_recurrence_is_two_rows(pkg):
    out = list(O.feed(pkg, SALT, **MAP))
    assert all(len(t[0]) == 16 and not t[0].startswith("RAWCUST") and t[2] == "w1" for t in out)
    by = {}
    for _k, g, _w, o in out:
        n, h = by.get(g, (0, 0))
        by[g] = (n + 1, h + (1 if o else 0))
    assert by == {"RAWCAT-alpha-cat": (30, 12), "RAWCAT-beta-cat": (14, 3), "RAWCAT-tiny-cat": (3, 3)}


def test_only_the_mapped_columns_are_kept_and_missing_column_error_has_no_value(pkg):
    with pytest.raises(ValueError) as e:
        list(O.feed(pkg, SALT, "complaints", "customer_id", "nope"))
    assert "RAWCAT" not in str(e.value) and "FREETEXT" not in str(e.value)


def test_path_env_and_unknown_table(monkeypatch, tmp_path):
    monkeypatch.delenv("ED0_ORIGINAL_PATH", raising=False)
    with pytest.raises(RuntimeError, match="ED0_ORIGINAL_PATH"):
        O.original_path()
    monkeypatch.setenv("ED0_ORIGINAL_PATH", str(tmp_path))
    assert O.original_path() == str(tmp_path)
    with pytest.raises(ValueError):
        list(O.feed(str(tmp_path), SALT, "../x", "a", "b"))


pytestmark_exe = pytest.mark.skipif(not os.path.exists(EXE), reason="runner exe not built")


@pytest.fixture(scope="module")
def run(tmp_path_factory):
    base = tmp_path_factory.mktemp("origthread")
    pkg = _write(base / "orig", [(f"RAWCUST-a{i:03d}", "RAWCAT-alpha-cat") for i in range(40)] * 1
                 + [(f"RAWCUST-a{i:03d}", "RAWCAT-alpha-cat") for i in range(15)])
    q = base / "queue"
    for d in ("requests", "responses"):
        (q / d).mkdir(parents=True)
    cfg = T.ThreadConfig(workdir=base / "work", exe=EXE, queue_dir=q, mode="record", original_path=pkg,
                         original_map=MAP)
    return {"res": T.run_thread(cfg), "base": base, "wd": base / "work", "q": q}


def _er():
    s = importlib.util.spec_from_file_location("engine_run_orig", ROOT / "contracts" / "engine-run" / "engine_run.py")
    m = importlib.util.module_from_spec(s)
    sys.modules["engine_run_orig"] = m
    s.loader.exec_module(m)
    return m


@pytestmark_exe
def test_original_labelled_original_treated_everywhere_never_generated_sample(run):
    rep = run["res"]["report"]
    assert _er().check(rep) == []
    classes = {s["n"]: s["data_class"] for s in rep["steps"] if s["status"] != "not_exercised"}
    assert set(classes.values()) <= {"original", "original-treated"} and classes[3] == "original-treated"
    assert "generated_sample" not in json.dumps(rep["doubles"]) and "E0" not in json.dumps(rep["doubles"])
    assert {d["part"]: d["status"] for d in rep["doubles"]}["data.origin"] == "original-treated-aggregates"


@pytestmark_exe
def test_original_ends_honestly_unlinked_not_supported(run):
    smap = next(s for s in run["res"]["steps"] if s["n"] == 4)["detail"]["smap"]
    assert smap["verdict"] == "unlinked" and smap["target"] is None
    for s in run["res"]["steps"]:
        if s["n"] >= 5:
            assert s["status"] == "not_exercised"


@pytestmark_exe
def test_original_no_raw_value_in_any_file_or_error(run):
    for p in (run["wd"], run["q"]):
        for f in p.rglob("*"):
            if f.is_file():
                for n in NEEDLES:
                    assert n not in f.read_bytes(), (f.name, n)
    assert all("RAW" not in str(s.get("error", "")) for s in run["res"]["steps"])


def test_failure_message_is_type_only_in_original_mode():
    e = ValueError("bad value RAWCUST-1")
    assert T.safe_error(e, True) == "ValueError"


def test_hosted_gateway_profile_still_rejects_original_classes():
    import copy
    gw = ROOT / "local" / "core" / "gateway"
    sys.path.insert(0, str(gw))
    import profile_check as pc
    prof = json.loads((gw / "profiles" / "gw-hosted.json").read_text("utf-8"))
    for cls in ("original", "original-treated", "Original Treated"):
        p = copy.deepcopy(prof)
        p["data_classes"].append(cls)
        assert "hosted_accepts_restricted" in {v.rule for v in pc.check_profile(p)}, cls

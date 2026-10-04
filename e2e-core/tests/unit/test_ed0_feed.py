"""ED0F RED/GREEN: pyarrow feeder from an E0-shaped parquet package into the ED0L lab.

Fixtures are SYNTHETIC parquet written in-test. Real E0 is never a fixture; it is only read at runtime."""
import importlib.util
import json
import logging
import sys
from pathlib import Path

import pytest

pa = pytest.importorskip("pyarrow")
pq = pytest.importorskip("pyarrow.parquet")

from claude_standin import ed0_feed as F  # noqa: E402
from claude_standin import ed0_lab as L  # noqa: E402

ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location("tps_scanner_feed", ROOT / "roleplay-llm/roleplay_llm/scanner.py")
TPS = importlib.util.module_from_spec(spec)
sys.modules["tps_scanner_feed"] = TPS
spec.loader.exec_module(TPS)

SALT = b"synthetic-feed-salt-01"
SIG_BIG, SIG_MID, SIG_SMALL = "SIGRAW-alpha-lookup", "SIGRAW-beta-lookup", "SIGRAW-tiny-lookup"
FREE_TEXT = "FREETEXT-must-not-be-read"


def _package(root, spec_rows):
    """spec_rows: list of (case_id, signature, n_queries). Writes datos/copilot_query.parquet."""
    d = Path(root) / "datos"
    d.mkdir(parents=True)
    rows = [(f"q-{i}-{j}", cid, sig) for i, (cid, sig, n) in enumerate(spec_rows) for j in range(n)]
    pq.write_table(pa.table({
        "query_id": [r[0] for r in rows], "case_id": [r[1] for r in rows],
        "query_signature": [r[2] for r in rows], "question_text": [FREE_TEXT] * len(rows),
        "answer": [FREE_TEXT] * len(rows)}), d / "copilot_query.parquet")
    return str(root)


@pytest.fixture
def pkg(tmp_path):
    spec_rows = []
    for i in range(30):   # 30 cases, 12 recurring (2 queries)
        spec_rows.append((f"RAWCASE-a{i:03d}", SIG_BIG, 2 if i < 12 else 1))
    for i in range(14):   # 14 cases, 3 recurring
        spec_rows.append((f"RAWCASE-b{i:03d}", SIG_MID, 2 if i < 3 else 1))
    for i in range(3):    # 3 cases that asked BOTH signatures once each: a repeat contact across signatures
        spec_rows.append((f"RAWCASE-d{i:03d}", SIG_BIG, 1))
        spec_rows.append((f"RAWCASE-d{i:03d}", SIG_MID, 1))
    for i in range(4):    # 4 cases: below k
        spec_rows.append((f"RAWCASE-c{i:03d}", SIG_SMALL, 2))
    return _package(tmp_path / "e0", spec_rows)


def test_first_red_no_raw_id_or_label_in_lab_scanner_payload_or_logs(pkg, tmp_path, caplog, capsys):
    caplog.set_level(logging.DEBUG)
    db = L.build_lab(tmp_path / "lab.sqlite", F.feed(pkg, SALT), SALT)
    rows = L.lab_query(db, L.METRIC, "w1")
    payload = {"goal": "g", "step": 1, "tools": [], "observations": [
        {"tool": "pulso/lab_query@1.0.0", "args": {"metric_id": L.METRIC, "window_id": "w1"}, "status": "ok",
         "result": rows, "error": None}]}
    assert TPS.scan_payload(payload).ok
    seen = [Path(db).read_bytes(), json.dumps(payload).encode(), caplog.text.encode(), capsys.readouterr().out.encode()]
    for blob in seen:
        for needle in (b"RAWCASE", b"SIGRAW", b"alpha", b"beta", b"tiny", FREE_TEXT.encode()):
            assert needle not in blob


def test_feed_yields_pseudonymous_case_keys_and_raw_group_in_memory_only(pkg):
    out = list(F.feed(pkg, SALT))
    assert out and all(len(t) == 4 and t[2] == "w1" for t in out)
    assert all(not t[0].startswith("RAWCASE") and len(t[0]) == 16 for t in out)
    assert {t[1] for t in out} == {SIG_BIG, SIG_MID, SIG_SMALL}


def test_recurrence_is_a_case_with_at_least_two_copilot_queries(pkg):
    by = {}
    for _k, g, _w, o in F.feed(pkg, SALT):
        n, h = by.get(g, (0, 0))
        by[g] = (n + 1, h + (1 if o else 0))
    assert by == {SIG_BIG: (33, 15), SIG_MID: (17, 6), SIG_SMALL: (4, 4)}


def test_k_anonymity_drops_small_groups_and_rates_recompute(pkg, tmp_path):
    db = L.build_lab(tmp_path / "lab.sqlite", F.feed(pkg, SALT), SALT)
    rows = L.lab_query(db, L.METRIC, "w1")["rows"]
    assert sorted((r["count"], r["rate"]) for r in rows) == [(17, 0.35), (33, 0.45)]
    for r in rows:
        assert L.verify_claim(db, L.scout_figure(db, r["evidence_ref"]), SALT)["ok"]


def test_only_needed_columns_are_read(pkg, monkeypatch):
    seen = []
    real = pq.read_table
    monkeypatch.setattr(pq, "read_table", lambda *a, **k: (seen.append(k.get("columns")), real(*a, **k))[1])
    list(F.feed(pkg, SALT))
    assert seen == [["case_id", "query_signature"]]


def test_path_comes_from_env_and_missing_path_is_a_clear_error(monkeypatch, tmp_path):
    monkeypatch.delenv("ED0_E0_PATH", raising=False)
    with pytest.raises(RuntimeError, match="ED0_E0_PATH"):
        F.e0_path()
    monkeypatch.setenv("ED0_E0_PATH", str(tmp_path))
    assert F.e0_path() == str(tmp_path)


def test_salt_is_ephemeral_or_env_and_never_stored(monkeypatch, tmp_path):
    monkeypatch.delenv("ED0_LAB_SALT", raising=False)
    a, b = F.lab_salt(), F.lab_salt()
    assert a != b and len(a) >= 16
    monkeypatch.setenv("ED0_LAB_SALT", "env-salt-0123456789abcdef")
    assert F.lab_salt() == b"env-salt-0123456789abcdef"
    monkeypatch.setenv("ED0_LAB_SALT", "short")
    with pytest.raises(ValueError):
        F.lab_salt()


def test_feed_does_not_leave_files_behind(pkg, tmp_path):
    before = sorted(p.name for p in tmp_path.rglob("*"))
    list(F.feed(pkg, SALT))
    assert sorted(p.name for p in tmp_path.rglob("*")) == before

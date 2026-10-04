"""ED0L RED/GREEN: treated lab of k-anonymous aggregates, scout figure and verifier recompute."""
import importlib.util
import sqlite3
from pathlib import Path

import pytest

from claude_standin import ed0_lab as L

ROOT = Path(__file__).resolve().parents[3]


def _load(rel, name):
    spec = importlib.util.spec_from_file_location(name, ROOT / rel)
    m = importlib.util.module_from_spec(spec)
    import sys
    sys.modules[name] = m
    spec.loader.exec_module(m)
    return m


TPS = _load("roleplay-llm/roleplay_llm/scanner.py", "tps_scanner")
DC0 = _load("scripts/dc/dataclass_gate.py", "dc0_gate")
SALT = b"synthetic-salt-0001"


def cases(seed, n_a=40, n_b=25, n_small=4):
    """Synthetic treated cases: (case_id, group, window, outcome). Group `small` is below k."""
    out, i = [], 0
    for grp, n, hits in (("alpha", n_a, 13 + seed), ("beta", n_b, 7), ("small", n_small, 2)):
        for j in range(n):
            i += 1
            out.append((f"private-case-{i}", grp, "w1", j < hits))
    return out


@pytest.fixture
def lab(tmp_path):
    return L.build_lab(tmp_path / "lab.sqlite", cases(0), SALT)


def test_first_red_every_evidence_ref_resolves_in_lab(lab):
    rows = L.lab_query(lab, "recurrence_rate", "w1")["rows"]
    assert rows
    for r in rows:
        assert L.resolve_ref(lab, r["evidence_ref"]) is not None
    assert L.resolve_ref(lab, "ev_00000000deadbeef") is None
    claim = {"evidence_ref": "ev_00000000deadbeef", "rate": 0.5, "count": 20}
    assert L.verify_claim(lab, claim, SALT)["ok"] is False


def test_lab_holds_only_k_anonymous_aggregates(lab):
    con = sqlite3.connect(lab)
    tables = {r[0] for r in con.execute("select name from sqlite_master where type='table'")}
    assert tables == {"lab_rows", "lab_meta"}
    counts = [r[0] for r in con.execute("select count from lab_rows")]
    assert counts and min(counts) >= L.K
    blob = b"".join(Path(lab).read_bytes() for _ in [0])
    for needle in (b"private-case", b"alpha", b"beta", b"small"):
        assert needle not in blob


def test_rows_pass_the_treated_payload_scanner(lab):
    rows = L.lab_query(lab, "recurrence_rate", "w1")["rows"]
    assert all(any(k.startswith("g_") for k in r) for r in rows)
    payload = {"goal": "g", "step": 0, "tools": [], "observations": [
        {"tool": "pulso/lab_query@1.0.0", "args": {"metric_id": "recurrence_rate"}, "status": "ok",
         "result": {"rows": rows}, "error": None}]}
    res = TPS.scan_payload(payload)
    assert res.ok, res.violations


def test_group_keys_are_salted_truncated_hmac(lab):
    rows = L.lab_query(lab, "recurrence_rate", "w1")["rows"]
    keys = {r["g_group"] for r in rows}
    assert keys == {L.group_hash(SALT, "group", "alpha"), L.group_hash(SALT, "group", "beta")}
    assert L.group_hash(b"other-salt", "group", "alpha") not in keys
    assert len(L.group_hash(SALT, "group", "alpha")) == 16


def test_no_raw_row_crosses_dc0_scanner(lab, tmp_path):
    rows = L.lab_query(lab, "recurrence_rate", "w1")
    p = tmp_path / "obs.json"
    import json
    p.write_text(json.dumps(rows))
    assert DC0.scan_file(p) == []


@pytest.mark.parametrize("seed", range(5))
def test_verifier_recompute_equals_scout_bit_for_bit(tmp_path, seed):
    db = L.build_lab(tmp_path / f"lab{seed}.sqlite", cases(seed, n_a=40 + seed, n_b=25 + 2 * seed), SALT)
    for r in L.lab_query(db, "recurrence_rate", "w1")["rows"]:
        figure = L.scout_figure(db, r["evidence_ref"])
        verdict = L.verify_claim(db, figure, SALT)
        assert verdict["ok"] is True
        assert verdict["recomputed"].hex() == figure["rate"].hex() if hasattr(figure["rate"], "hex") else True
        assert verdict["recomputed"] == figure["rate"]
        assert verdict["recomputed"] == r["rate"]


def test_tampered_numerator_fails(lab):
    ref = L.lab_query(lab, "recurrence_rate", "w1")["rows"][0]["evidence_ref"]
    figure = L.scout_figure(lab, ref)
    con = sqlite3.connect(lab)
    con.execute("update lab_rows set numerator = numerator + 3 where evidence_ref = ?", (ref,))
    con.commit()
    con.close()
    v = L.verify_claim(lab, figure, SALT)
    assert v["ok"] is False and "digest" in " ".join(v["reasons"])


def test_tampered_claim_rate_fails(lab):
    ref = L.lab_query(lab, "recurrence_rate", "w1")["rows"][0]["evidence_ref"]
    figure = dict(L.scout_figure(lab, ref))
    figure["rate"] = round(figure["rate"] + 0.07, 2)
    assert L.verify_claim(lab, figure, SALT)["ok"] is False


def test_below_k_group_is_not_stored(lab):
    con = sqlite3.connect(lab)
    n = con.execute("select count(*) from lab_rows").fetchone()[0]
    assert n == 2
    assert con.execute("select count(*) from lab_meta where key like '%suppress%'").fetchone()[0] == 0


def test_forged_numerator_with_recomputed_digest_fails(lab):
    ref = L.lab_query(lab, "recurrence_rate", "w1")["rows"][0]["evidence_ref"]
    figure = L.scout_figure(lab, ref)
    con = sqlite3.connect(lab)
    m, w, g, c = con.execute("select metric_id, window_id, g_group, count from lab_rows where evidence_ref=?", (ref,)).fetchone()
    import hashlib
    con.execute("update lab_rows set numerator = 1, digest = ? where evidence_ref = ?",
                (hashlib.sha256(f"{m}|{w}|{g}|1|{c}".encode()).hexdigest(), ref))
    con.commit()
    con.close()
    assert L.verify_claim(lab, figure, SALT)["ok"] is False


def test_swapped_evidence_ref_fails(lab):
    refs = [r["evidence_ref"] for r in L.lab_query(lab, "recurrence_rate", "w1")["rows"]]
    con = sqlite3.connect(lab)
    con.execute("update lab_rows set evidence_ref='tmp' where evidence_ref=?", (refs[0],))
    con.execute("update lab_rows set evidence_ref=? where evidence_ref=?", (refs[0], refs[1]))
    con.execute("update lab_rows set evidence_ref=? where evidence_ref='tmp'", (refs[1],))
    con.commit()
    con.close()
    assert L.verify_claim(lab, {"evidence_ref": refs[0], "rate": 0.5, "count": 1}, SALT)["ok"] is False


def test_short_salt_rejected(tmp_path):
    with pytest.raises(ValueError):
        L.build_lab(tmp_path / "x.sqlite", cases(0), b"short")

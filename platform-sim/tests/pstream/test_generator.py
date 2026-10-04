import json
import re

from product_stream.catalog_view import PRODUCT_COLUMNS
from product_stream.generator import ProductStream


def collect(seed=1, n=400, scenario="null", **kw):
    g = ProductStream(seed=seed, scenario=scenario, horizon_events=n, **kw)
    out = {t: [] for t in PRODUCT_COLUMNS}
    while g.last_sequence < n:
        for t, rows in g.next_batch(50).items():
            out[t].extend(rows)
    return g, out


def test_deterministic_for_same_seed_and_differs_for_other():
    _, a = collect(seed=3)
    _, b = collect(seed=3)
    _, c = collect(seed=4)
    assert a == b and a != c


def test_batching_does_not_change_the_stream():
    g1 = ProductStream(seed=5, horizon_events=300)
    g2 = ProductStream(seed=5, horizon_events=300)
    a = [r for _ in range(6) for r in g1.next_batch(50)["event_log"]]
    b = [r for _ in range(30) for r in g2.next_batch(10)["event_log"]]
    assert a == b


def test_event_log_sequence_dense_monotonic_and_times():
    _, d = collect()
    ev = d["event_log"]
    assert [e["sequence"] for e in ev] == list(range(1, len(ev) + 1))
    times = [e["event_time"] for e in ev]
    assert times == sorted(times)
    assert all(e["ingested_at"] >= e["event_time"] for e in ev)
    assert all(e["tenant_id"] for e in ev)


def test_only_catalog_columns_and_no_free_text_or_personal_data():
    _, d = collect()
    for t, rows in d.items():
        for r in rows:
            assert set(r) <= set(PRODUCT_COLUMNS[t])
    blob = json.dumps(d)
    assert "@" not in blob
    for e in d["event_log"]:
        assert all(isinstance(v, (int, float, bool, str, type(None))) for v in e["payload"].values())
        assert all(len(str(v)) <= 40 for v in e["payload"].values())
    for t in ("cases", "customers", "staff"):
        assert all(re.fullmatch(r"[A-Z]{3}-[0-9a-f]{8}", r.get("id", "X-00000000".replace("X", "XXX"))) for r in d[t])


def test_event_types_are_admitted_in_catalog_1_1_0():
    cat = json.load(open("platform-contract/event-catalog.json"))
    ok = {e["event_type"] for e in cat["event_types"] if e["status"] == "admitted"}
    _, d = collect(n=800)
    assert {e["event_type"] for e in d["event_log"]} <= ok
    assert {"case.opened", "case.assigned", "turn.created", "case.closed"} <= {e["event_type"] for e in d["event_log"]}


def test_referential_integrity():
    _, d = collect(n=800)
    cases = {c["id"] for c in d["cases"]}
    custs = {c["id"] for c in d["customers"]}
    staff = {s["id"] for s in d["staff"]}
    assert all(c["customer_id"] in custs for c in d["cases"])
    assert all(a["case_id"] in cases and a["staff_id"] in staff for a in d["assignments"])
    assert all(t["case_id"] in cases for t in d["turns"])
    assert all(c["previous_case_id"] in cases for c in d["cases"] if c["previous_case_id"])
    for c in d["cases"]:
        assert c["sla_due_at"] > c["opened_at"]


def test_turn_sequence_per_case_is_contiguous():
    _, d = collect(n=800)
    by = {}
    for t in d["turns"]:
        by.setdefault(t["case_id"], []).append(t["sequence"])
    assert all(s == list(range(1, len(s) + 1)) for s in by.values())

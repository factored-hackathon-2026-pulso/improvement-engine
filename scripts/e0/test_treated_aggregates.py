"""treated_aggregates: aggregates only, k-anonymity, no ids / free text, honest about missing suggestion rows. SYNTHETIC fixtures only."""
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from treated_aggregates import ALLOWED_KEYS, aggregate  # noqa: E402


def _cases(topic, n, start=0):
    return [{"case_id": f"c{topic}{i + start}", "topic": topic} for i in range(n)]


def _queries(cases, sig):
    return [{"case_id": c["case_id"], "query_signature": sig} for c in cases]


def test_counts_repeat_q_tool_use_and_declares_no_drafts():
    cs = _cases("cobro_duplicado", 30)
    q = _queries(cs[:25], "recent_charges") + _queries(cs[25:], "other")
    tools = [{"case_id": c["case_id"], "actor_role": "analyst"} for c in cs[:20]]
    out = aggregate(cs, q, tools, suggestion_rows=0, k=10)
    r = {x["type_id"]: x for x in out["case_types"]}["cobro_indebido"]
    assert (r["repeat_q_cases"], r["tool_applicable"], r["tool_used"], r["copilot_questions"]) == (25, 25, 20, 30)
    assert "drafts" not in r
    assert out["source"] == "e0_treated" and out["simulated"] is False
    assert out["has_suggestion_rows"] is False and "no_draft_rows" in out["not_computable"]["draft_accept_100"]


def test_small_types_are_suppressed_and_unmapped_topics_dropped():
    cs = _cases("cobro_duplicado", 9) + _cases("fuera_de_alcance", 50)
    out = aggregate(cs, _queries(cs, "s"), [], suggestion_rows=0, k=10)
    assert out["case_types"] == []
    assert out["types_suppressed"] is True


def test_output_has_no_ids_or_text():
    cs = _cases("problema_app", 12)
    out = aggregate(cs, _queries(cs, "s"), [], suggestion_rows=0, k=10)
    blob = json.dumps(out)
    assert "cproblema_app" not in blob
    for r in out["case_types"]:
        assert set(r) <= set(ALLOWED_KEYS)


def test_k_below_10_is_refused():
    try:
        aggregate([], [], [], suggestion_rows=0, k=5)
    except ValueError:
        return
    raise AssertionError("k<10 must be refused")


def _cells(o):
    if isinstance(o, bool) or o is None or isinstance(o, str):
        return []
    if isinstance(o, (int, float)):
        return [o]
    if isinstance(o, dict):
        return [c for v in o.values() for c in _cells(v)]
    return [c for v in o for c in _cells(v)]


def test_every_emitted_numeric_cell_is_at_least_k_or_null():
    cs = _cases("cobro_duplicado", 30)
    q = _queries(cs[:4], "rare") + _queries(cs[4:], "common")
    tools = [{"case_id": c["case_id"], "actor_role": "analyst"} for c in cs[:3]]
    out = aggregate(cs, q, tools, suggestion_rows=3, k=10)
    r = out["case_types"][0]
    assert r["tool_used"] is None
    small = [c for c in _cells({k: v for k, v in out.items() if k != "k_min"}) if c < 10]
    assert small == [], small

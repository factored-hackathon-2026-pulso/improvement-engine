"""platform_live simulator speaks contract 1.2.0 (additive): assistant/copilot/rating events, ids and enums only.
Draft disposition and tool use are NOT platform events at eeb73a8: the simulator keeps them as sim-side ground truth."""

import json

from platform_contract import conformance
import platform_contract as pc
from platform_live import PlatformLiveSim


def _types(sim):
    return [r[0] for r in sim.conn.execute("select event_type from event_log order by sequence")]


def _events(sim, t):
    return [json.loads(r[0]) for r in sim.conn.execute("select payload from event_log where event_type=? order by sequence", (t,))]


def _assisted(sim):
    cust = sim.customer_ids(simulator=False)[0]
    return sim.open_case_with_assistant(cust, text="hola")


def test_ddl_admits_1_2_0_enums_and_keeps_1_1_0():
    sim = PlatformLiveSim(seed=1)
    cid = _assisted(sim)
    assert sim.conn.execute("select status from cases where id=?", (cid,)).fetchone()[0] == "with_assistant"
    sim.assistant_answer(cid)
    assert sim.conn.execute("select count(*) from turns where author_role='assistant'").fetchone()[0] == 1


def test_assistant_resolution_flow_emits_contract_events_only():
    sim = PlatformLiveSim(seed=2)
    cid = _assisted(sim)
    sim.assistant_answer(cid)
    sim.assistant_resolve(cid)
    t = _types(sim)
    for e in ("case.assistant_started", "assistant.session_started", "assistant.turn_answered", "assistant.ended", "case.closed"):
        assert e in t, e
    assert _events(sim, "assistant.ended")[0]["result"] == "resolved"
    assert all(pc.classify_event_type(x) == "admitted" for x in t)
    assert conformance.check_database(sim.conn)["violations"] == []


def test_assistant_escalation_queues_or_assigns_with_assistant_handoff_reason():
    sim = PlatformLiveSim(seed=3)
    cid = _assisted(sim)
    sim.assistant_escalate(cid)
    assert _events(sim, "case.assistant_released")[0]["reason"] == "escalated"
    assert _events(sim, "assistant.ended")[0]["result"] == "escalated"
    assert sim.conn.execute("select reason from assignments where case_id=?", (cid,)).fetchone()[0] == "assistant_handoff"
    assert conformance.check_database(sim.conn)["violations"] == []


def test_copilot_events_carry_ids_enums_and_sizes_only_and_labels_stay_sim_side():
    sim = PlatformLiveSim(seed=4)
    cust = sim.customer_ids(simulator=False)[0]
    cid = sim.open_case(cust)
    analyst = sim.conn.execute("select assigned_analyst_id from cases where id=?", (cid,)).fetchone()[0]
    qid = sim.copilot_ask(cid, analyst, case_type="disputa_cargo", draft_outcome="sent_as_is")
    asked, answered = _events(sim, "copilot.query_asked")[0], _events(sim, "copilot.answered")[0]
    assert set(asked) == {"question_id", "question_length"} and asked["question_id"] == qid
    assert set(answered) == {"question_id", "agent", "run_id", "trace_id", "status", "messages"}
    blob = json.dumps([asked, answered])
    assert "disputa_cargo" not in blob and "sent_as_is" not in blob
    assert sim.copilot_labels[qid] == {"case_id": cid, "case_type": "disputa_cargo", "draft_outcome": "sent_as_is"}
    assert "copilot_labels" not in sim.conn.execute("select group_concat(sql) from sqlite_master").fetchone()[0]


def test_rating_event_has_score_without_comment():
    sim = PlatformLiveSim(seed=5)
    cust = sim.customer_ids(simulator=False)[0]
    cid = sim.open_case(cust)
    sim.close_case(cid)
    sim.rate_case(cid, 4)
    p = _events(sim, "case.rated")[0]
    assert p["score"] == 4 and "comment" not in p
    assert sim.conn.execute("select rating_score from cases where id=?", (cid,)).fetchone()[0] == 4


def test_default_generate_is_unchanged_1_1_0_vocabulary():
    sim = PlatformLiveSim(seed=3)
    sim.generate(n_cases=20)
    assert not any(t.startswith(("assistant.", "copilot.")) for t in _types(sim))

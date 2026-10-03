"""PL-L2 first RED: platform_live simulator reproduces the real model and passes conformance."""

import json
import sqlite3
from datetime import datetime

import pytest

from platform_contract import conformance
from platform_live import PlatformLiveSim, SlotOccupied, render_ddl

TABLES_11 = {
    "customers", "cases", "turns", "assignments", "customer_case_slots", "staff",
    "login_accounts", "mfa_challenges", "staff_sessions", "analyst_availability", "event_log",
}


def _tables(conn):
    return {r[0] for r in conn.execute("select name from sqlite_master where type='table'")
            if not r[0].startswith("sqlite_")}


def _dump(sim):
    out = {}
    for t in sorted(_tables(sim.conn)):
        out[t] = [tuple(r) for r in sim.conn.execute(f"select * from {t} order by 1")]
    return json.dumps(out, default=str, sort_keys=True)


def _ts(s):
    return datetime.fromisoformat(s.replace("Z", "+00:00"))


def test_creates_the_eleven_table_model():
    sim = PlatformLiveSim(seed=1)
    assert _tables(sim.conn) == TABLES_11


def test_ddl_is_portable_to_postgres():
    pg = "\n".join(render_ddl("postgres"))
    assert "AUTOINCREMENT" not in pg.upper()
    assert "TIMESTAMPTZ" in pg and "JSONB" in pg
    sqlite3.connect(":memory:").executescript(";\n".join(render_ddl("sqlite")))
    sqlglot = pytest.importorskip("sqlglot")
    for stmt in render_ddl("postgres"):
        sqlglot.parse_one(stmt, read="postgres")


def test_deterministic_seed():
    a, b, c = (PlatformLiveSim(seed=s) for s in (5, 5, 6))
    for s in (a, b, c):
        s.generate(n_cases=25)
    assert _dump(a) == _dump(b)
    assert _dump(a) != _dump(c)


def test_clean_run_passes_conformance_with_no_findings():
    sim = PlatformLiveSim(seed=3)
    sim.generate(n_cases=40)
    rep = conformance.check_database(sim.conn)
    assert rep["violations"] == []
    assert rep["findings"] == []
    assert rep["rows"]["cases"] == 40


def test_event_log_sequence_contiguous_and_ingested_after_event_time():
    sim = PlatformLiveSim(seed=4)
    sim.generate(n_cases=20)
    rows = sim.conn.execute("select sequence, event_time, ingested_at from event_log order by sequence").fetchall()
    assert [r[0] for r in rows] == list(range(1, len(rows) + 1))
    assert all(r[2] >= r[1] for r in rows)


def test_one_open_case_per_customer_and_reopen_chain():
    sim = PlatformLiveSim(seed=1)
    cus = sim.customer_ids(simulator=False)[0]
    c1 = sim.open_case(cus)
    with pytest.raises(SlotOccupied):
        sim.open_case(cus)
    sim.close_case(c1, reason="resolved")
    c2 = sim.open_case(cus)
    prev = sim.conn.execute("select previous_case_id from cases where id=?", (c2,)).fetchone()[0]
    assert prev == c1
    slot = sim.conn.execute("select open_case_id from customer_case_slots where customer_id=?", (cus,)).fetchone()
    assert slot[0] == c2
    assert sim.conn.execute("select status from cases where id=?", (c1,)).fetchone()[0] == "closed"


def test_h1_portuguese_only_to_portuguese_speakers_and_queue_drain():
    sim = PlatformLiveSim(seed=2)
    pt_staff = sim.staff_ids(language="pt")
    for s in pt_staff:
        sim.set_availability(s, "paused")
    cus = sim.customer_ids(locale="pt-BR", simulator=False)[0]
    cid = sim.open_case(cus)
    assert sim.conn.execute("select status, assigned_analyst_id from cases where id=?", (cid,)).fetchone() == ("queued", None)
    sim.advance(120)
    sim.set_availability(pt_staff[0], "available")
    status, who = sim.conn.execute("select status, assigned_analyst_id from cases where id=?", (cid,)).fetchone()
    assert (status, who) == ("assigned", pt_staff[0])
    asg = sim.conn.execute("select reason, policy_rule_id, waited_seconds from assignments where case_id=?", (cid,)).fetchone()
    assert asg[0] == "queue_drained" and asg[1] == "H1" and asg[2] >= 120


def test_sla_due_by_priority_and_first_response():
    sim = PlatformLiveSim(seed=2)
    cus = sim.customer_ids(locale="es-CO", simulator=False)
    ids = {p: sim.open_case(cus[i], priority=p) for i, p in enumerate(("high", "medium", "low"))}
    for p, minutes in (("high", 5), ("medium", 15), ("low", 60)):
        o, d = sim.conn.execute("select opened_at, sla_due_at from cases where id=?", (ids[p],)).fetchone()
        assert (_ts(d) - _ts(o)).total_seconds() == minutes * 60
    cid = ids["high"]
    sim.advance(30)
    sim.analyst_reply(cid, "respuesta")
    fr, st = sim.conn.execute("select first_response_at, status from cases where id=?", (cid,)).fetchone()
    assert fr is not None and st == "in_progress"
    sim.advance(30)
    sim.analyst_reply(cid, "otra")
    assert sim.conn.execute("select first_response_at from cases where id=?", (cid,)).fetchone()[0] == fr


def test_mutable_cases_versioned_and_append_only_tables_guarded():
    sim = PlatformLiveSim(seed=2)
    cid = sim.open_case(sim.customer_ids(simulator=False)[0])
    v0 = sim.conn.execute("select version from cases where id=?", (cid,)).fetchone()[0]
    sim.analyst_reply(cid, "hola")
    assert sim.conn.execute("select version from cases where id=?", (cid,)).fetchone()[0] > v0
    for sql in ("update turns set text='x'", "delete from turns", "update assignments set reason='x'",
                "delete from event_log", "update event_log set event_type='x'"):
        with pytest.raises(sqlite3.DatabaseError):
            sim.conn.execute(sql)


def test_turn_sequence_per_case_has_no_gaps_and_state_replays_from_events():
    sim = PlatformLiveSim(seed=8)
    sim.generate(n_cases=30)
    assert conformance.check_turn_sequences(conformance.read_table(sim.conn, "turns")) == []
    final = {}
    for cid, payload in sim.conn.execute(
            "select case_id, payload from event_log where event_type='case.status_changed' order by sequence"):
        final[cid] = json.loads(payload)["to"]
    for cid, status in sim.conn.execute("select id, status from cases"):
        assert final.get(cid, "queued") == status, cid


def test_simulator_customers_are_flagged():
    sim = PlatformLiveSim(seed=1)
    flags = {r[0] for r in sim.conn.execute("select simulator from customers")}
    assert flags == {0, 1}
    assert sim.customer_ids(simulator=True)


def test_credential_tables_hold_only_placeholders():
    sim = PlatformLiveSim(seed=1)
    hashes = [h for (h,) in sim.conn.execute("select password_hash from login_accounts")]
    assert hashes and all(h.startswith("FAKE-") for h in hashes)
    assert sim.conn.execute("select count(*) from staff where email not like '%@example.invalid'").fetchone()[0] == 0


def test_fault_late_event():
    sim = PlatformLiveSim(seed=1)
    cid = sim.open_case(sim.customer_ids(simulator=False)[0])
    sim.inject_late_event(cid, lag_seconds=7200)
    rep = [f["code"] for f in conformance.check_event_stream(
        conformance.read_table(sim.conn, "event_log"), late_after_seconds=3600)]
    assert rep == ["late_event"]
    assert conformance.check_database(sim.conn)["violations"] == []


def test_fault_sequence_gap():
    sim = PlatformLiveSim(seed=1)
    cus = sim.customer_ids(simulator=False)
    sim.open_case(cus[0])
    sim.inject_sequence_gap(3)
    sim.open_case(cus[1])
    f = [x for x in conformance.check_database(sim.conn)["findings"] if x["code"] == "gap_suspected"]
    assert len(f) == 1 and f[0]["next"] - f[0]["after"] == 4
    assert any(x["kind"] == "sequence_gap" for x in sim.faults)


def test_fault_turn_gap():
    sim = PlatformLiveSim(seed=1)
    cid = sim.open_case(sim.customer_ids(simulator=False)[0])
    sim.inject_turn_gap(cid)
    codes = [f["code"] for f in conformance.check_database(sim.conn)["findings"]]
    assert "turn_gap_suspected" in codes


def test_fault_unknown_event_type_does_not_break_schema_conformance():
    sim = PlatformLiveSim(seed=1)
    sim.open_case(sim.customer_ids(simulator=False)[0])
    sim.inject_unknown_event_type("case.escalated")
    rep = conformance.check_database(sim.conn)
    assert rep["violations"] == []
    assert [f["code"] for f in rep["findings"]] == ["unknown_event_type"]


def test_planned_teams_evolution():
    sim = PlatformLiveSim(seed=1)
    sim.evolve_teams()
    assert {"teams", "admin_roster"} <= _tables(sim.conn)
    cols = {r[1] for r in sim.conn.execute("pragma table_info(staff)")}
    assert "team_id" in cols and "team" not in cols
    rep = conformance.check_database(sim.conn)
    assert rep["violations"] == []
    assert {f["code"] for f in rep["findings"]} == {"planned_event_type"}
    sim.open_case(sim.customer_ids(simulator=False)[0])
    assert conformance.check_database(sim.conn)["violations"] == []


def test_generate_with_faults_is_deterministic_and_covers_scenarios():
    def build():
        s = PlatformLiveSim(seed=11)
        s.generate(n_cases=60, faults=True)
        return s

    a, b = build(), build()
    assert _dump(a) == _dump(b)
    codes = {f["code"] for f in conformance.check_database(a.conn)["findings"]}
    assert {"gap_suspected", "unknown_event_type"} <= codes
    q = a.conn.execute
    assert q("select count(*) from cases where previous_case_id is not null").fetchone()[0] > 0
    assert q("select count(*) from assignments where reason='queue_drained'").fetchone()[0] > 0
    assert q("select count(*) from assignments where previous_staff_id is not null").fetchone()[0] > 0
    assert q("select count(distinct close_reason) from cases").fetchone()[0] >= 3


def test_client_message_id_unique_per_author():
    sim = PlatformLiveSim(seed=1)
    cid = sim.open_case(sim.customer_ids(simulator=False)[0])
    row = sim.conn.execute("select case_id, author_id, client_message_id from turns "
                           "where client_message_id is not null").fetchone()
    with pytest.raises(sqlite3.IntegrityError):
        sim.conn.execute("insert into turns (id, case_id, sequence, kind, audience, author_role, author_id, text, "
                         "language, created_at, client_message_id) values ('TRN-X', ?, 99, 'message', 'everyone', "
                         "'customer', ?, 't', 'es', '2026-03-02T09:00:00Z', ?)", (cid, row[1], row[2]))


def test_postgres_append_only_guards_exist_and_parse():
    from platform_live.ddl import append_only_guards_postgres
    stmts = append_only_guards_postgres()
    joined = "\n".join(stmts)
    for tb in ("turns", "assignments", "event_log"):
        assert f"BEFORE UPDATE OR DELETE ON {tb}" in joined
    sqlglot = pytest.importorskip("sqlglot")
    for s in stmts:
        sqlglot.parse_one(s, read="postgres")

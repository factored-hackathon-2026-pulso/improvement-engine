"""L6 FIRST RED: a crash after the ACK (before the local cursor commit) must not double count, and the
duplicate / gap / late event cases of the audit chain. Real PG16 for Core; the ingest endpoint is a double."""

from __future__ import annotations

import psycopg
import pytest

from pulso_core_runtime.exporter import SimulatedCrash

from .conftest import Rig
from .seed import append_events, seed_run

pytestmark = [pytest.mark.pg, pytest.mark.l6]


def _raise_once():
    state = {"done": False}

    def hook() -> None:
        if not state["done"]:
            state["done"] = True
            raise SimulatedCrash()
    return hook


def test_crash_after_ack_does_not_double_count(rig: Rig) -> None:
    seed_run(rig.pg.runtime, "run-1", middle=2)
    seed_run(rig.pg.runtime, "run-2", middle=1)
    ex = rig.make()
    ex.crash_after_ack = _raise_once()
    with pytest.raises(SimulatedCrash):
        ex.poll_once()
    committed_before = rig.ingest.count("core_event", "engine_event")
    assert committed_before == 7  # server committed (3+... events) even though the exporter never learnt it
    ex.close()

    ex2 = rig.make()  # restart: the pending batch is re-POSTed byte-identically
    report = ex2.poll_once()
    assert rig.ingest.count("core_event", "engine_event") == 7  # no double count
    assert len({tuple(k) for k in rig.ingest.ledger}) == 7
    assert report.duplicate_acks >= 1
    assert len(rig.ingest.raw_bodies) == len({b for b in rig.ingest.raw_bodies})  # nothing applied twice
    # after recovery nothing is left to send
    assert ex2.poll_once().batches_sent == 0


def test_duplicate_gap_and_late_events(rig: Rig) -> None:
    seed_run(rig.pg.runtime, "ok-run", middle=1)
    # a run with a forced gap: seq 0,2 (seq 1 deleted by the DBA/restore) -> only that run is affected
    seed_run(rig.pg.runtime, "gap-run", middle=2, close=True)
    with psycopg.connect(rig.pg.runtime, autocommit=True) as c:
        c.execute("ALTER TABLE audit_events DISABLE TRIGGER audit_events_no_update")
        c.execute("DELETE FROM audit_events WHERE run_id='gap-run' AND seq=1")
    ex = rig.make()
    r1 = ex.poll_once()
    assert r1.partial_reasons == ["run_chain_gap:gap-run:1-1"]
    ids = {k[4] for k in rig.ingest.ledger}
    assert {"ok-run-e0", "ok-run-e1", "ok-run-e2"} <= ids  # unaffected run fully exported
    assert "gap-run-e0" in ids and "gap-run-e2" not in ids  # events after the gap are withheld
    # late: handoff_resolved after run_closed was exported
    with psycopg.connect(rig.pg.runtime) as c:
        append_events(c, "ok-run", ["handoff_resolved"], start_index=3)
    ex.poll_once()
    late = [e for b in rig.ingest.batches for e in b["events"] if e["native_event_id"] == "ok-run-e3"]
    assert len(late) == 1 and late[0]["coverage_marker"] == "late"
    # duplicate: polling again (and a fresh exporter with empty state in rebuild mode) never double counts
    n = rig.ingest.count()
    ex.poll_once()
    assert rig.ingest.count() == n

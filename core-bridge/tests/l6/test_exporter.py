"""L6 exporter acceptance (real PG16 for Core; ingest endpoint is the platform-sim double)."""

from __future__ import annotations

import json
from typing import Any

import psycopg
import pytest
from agent_core.audit.chain import check_chain, event_from_json
from agent_core.domain import to_jsonable

from pulso_core_runtime.exporter import (
    CoreReader,
    Exporter,
    ExporterConfig,
    ExporterError,
    ExporterState,
    MaterialError,
    build_ndjson,
    plan_chunks,
)

from .conftest import Rig
from .seed import insert_outbox, insert_reg_event, seed_run

pytestmark = [pytest.mark.pg, pytest.mark.l6]


def _admin(rig: Rig, autocommit: bool = True) -> psycopg.Connection[Any]:
    return psycopg.connect(rig.pg.runtime, autocommit=autocommit)


# ---------------------------------------------------------------- audit: happy path, projection, canary
def test_two_runs_batched_receipts_and_no_payload_leak(rig: Rig) -> None:
    rows = seed_run(rig.pg.runtime, "run-a", middle=2) + seed_run(rig.pg.runtime, "run-b", middle=1)
    ex = rig.make()
    rep = ex.poll_once()
    assert rep.batches_sent == 1 and rig.ingest.count("core_event", "engine_event") == 7
    # projection equals the pinned events field-for-field on the identity fields we export
    by_id = {e["native_event_id"]: e for b in rig.ingest.batches for e in b["events"]}
    for ev in rows:
        o = by_id[ev.event_id]
        assert (o["source_event_digest"], o["source_run_ref"], o["source_sequence"]) == (ev.hash, ev.run_id, ev.seq)
        assert o["source_event"] is None and o["source_event_ref"]["media_type"] == "application/x-ndjson"
    # a verification receipt per closed run, bound to the exact bytes of the uploaded chain
    assert {r["run_id"] for r in rig.ingest.verification_receipts} == {"run-a", "run-b"}
    for r in rig.ingest.verification_receipts:
        assert r["check_result"] == {"ok": True, "broken_at": None, "reason": None}
        assert len(r["verifier_agent_core_sha"]) == 40
        art = rig.ingest.artifacts[r["source_artifact_digest"]]["body"]["content"]
        assert art.endswith("\n") and "\n\n" not in art and "\r" not in art
        evs = [event_from_json(line) for line in art.splitlines()]
        assert check_chain(r["run_id"], evs).ok and evs[-1].hash == r["chain_head_hash"]
    # canary: no Core payload value in any POST body
    for raw in rig.ingest.raw_bodies:
        assert b"CANARY-ATTR" not in raw and b"reportable_attrs" not in raw and b'"payload"' not in raw
    assert ex.poll_once().batches_sent == 0  # idempotent
    # coverage never claims completeness without a cut
    assert ex.coverage()["coverage"] in ("partial", "unknown") and ex.coverage()["cut_ref"] is None


def test_batch_limits_500_events_and_size(rig: Rig) -> None:
    with _admin(rig, autocommit=False) as c:
        from .seed import append_events
        append_events(c, "big", ["run_started", *(["node_entered"] * 1199), "run_closed"])
    ex = rig.make()
    ex.poll_once()
    sizes = [len(b) for b in rig.ingest.raw_bodies]
    counts = [len(b["events"]) for b in rig.ingest.batches]
    assert max(counts) <= 500 and sum(counts) == 1201 and max(sizes) <= 512 * 1024
    assert rig.ingest.count() == 1201


def test_size_limit_splits_by_bytes(rig: Rig) -> None:
    seed_run(rig.pg.runtime, "r1", middle=40)
    ex = rig.make(batch_max_bytes=16 * 1024)
    ex.poll_once()
    assert len(rig.ingest.batches) > 1 and all(len(b) <= 16 * 1024 for b in rig.ingest.raw_bodies)
    assert rig.ingest.count() == 42


def test_audit_batches_never_span_a_utc_day(rig: Rig) -> None:
    seed_run(rig.pg.runtime, "d1", middle=1, ts="2026-10-03T23:59:00Z")
    seed_run(rig.pg.runtime, "d2", middle=1, ts="2026-10-04T00:01:00Z")
    rig.make().poll_once()
    assert sorted(b["partition"] for b in rig.ingest.batches) == ["audit:2026-10-03", "audit:2026-10-04"]


# ---------------------------------------------------------------- artifact bytes
def test_ndjson_exact_bytes_and_chunking() -> None:
    lines = ['{"a":1}', '{"b":2.50}', '{"c":3}']
    assert build_ndjson(lines) == b'{"a":1}\n{"b":2.50}\n{"c":3}\n'
    with pytest.raises(MaterialError):
        build_ndjson(['{"a":\n1}'])
    with pytest.raises(MaterialError):
        build_ndjson(['{"a":1}\r'])
    chunks = plan_chunks([json.dumps({"i": i, "pad": "x" * 100}) for i in range(100)], 2000, 10**9)
    assert len(chunks) > 1 and all(len(c.data) <= 2000 and c.data.endswith(b"\n") for c in chunks)
    assert [c.from_seq for c in chunks][0] == 0 and all(
        a.to_seq + 1 == b.from_seq for a, b in zip(chunks, chunks[1:], strict=False))
    with pytest.raises(MaterialError):
        plan_chunks(["x" * 100] * 10, 2000, 500)  # over the verification material limit


def test_large_chain_uses_chunks_and_manifest(rig: Rig) -> None:
    seed_run(rig.pg.runtime, "long", middle=30)
    ex = rig.make(artifact_max_bytes=4096)
    ex.poll_once()
    rec = rig.ingest.verification_receipts[0]
    manifest = rig.ingest.artifacts[next(d for d, a in rig.ingest.artifacts.items()
                                         if a["ref"] == rec["source_artifact_ref"])]["body"]["content"]
    assert len(manifest["chunks"]) > 1 and manifest["total_events"] == 32
    assert manifest["material_digest"] == rec["source_artifact_digest"]  # raw digest, not the manifest digest


# ---------------------------------------------------------------- tamper / conflict
def test_tampered_hash_is_source_conflict_and_stops_partition(rig: Rig) -> None:
    seed_run(rig.pg.runtime, "t1", middle=1)
    ex = rig.make()
    ex.poll_once()
    with _admin(rig) as c:  # restore/tamper outside the app: triggers disabled by a DBA
        c.execute("ALTER TABLE audit_events DISABLE TRIGGER audit_events_no_update")
        c.execute("UPDATE audit_events SET hash=repeat('a',64) WHERE run_id='t1' AND seq=1")
    rep = ex.rescan()
    assert any(v.startswith("source_conflict:t1:1") for v in rep.stopped.values())
    n = len(rig.ingest.batches)
    seed_run(rig.pg.runtime, "t2", middle=0)
    ex.poll_once()
    assert len(rig.ingest.batches) == n  # audit partition stopped


def test_new_events_with_bad_hash_exclude_only_that_run(rig: Rig) -> None:
    seed_run(rig.pg.runtime, "good", middle=1)
    seed_run(rig.pg.runtime, "bad", middle=1)
    with _admin(rig) as c:
        c.execute("ALTER TABLE audit_events DISABLE TRIGGER audit_events_no_update")
        c.execute("UPDATE audit_events SET hash=repeat('b',64) WHERE run_id='bad' AND seq=1")
    rep = rig.make().poll_once()
    assert "run_chain_broken:bad:1" in rep.partial_reasons
    ids = {k[4] for k in rig.ingest.ledger}
    assert "good-e2" in ids and "bad-e1" not in ids


# ---------------------------------------------------------------- registry / outbox sparse cursor
def test_sparse_cursor_10_to_12_then_late_13_and_rescan_once(rig: Rig) -> None:
    with _admin(rig) as c:
        for _ in range(10):
            insert_reg_event(c)
    ex = rig.make(gap_grace_seconds=120)
    ex.poll_once()
    assert rig.ingest.cursors[("core-a.registry", "registry")]["cursor"] == "s.10"
    t1 = _admin(rig, autocommit=False)  # seq 11 stays uncommitted
    insert_reg_event(t1.__enter__())
    with _admin(rig) as c2:
        insert_reg_event(c2)  # seq 12 commits first
        insert_reg_event(c2)  # seq 13
    ex.poll_once()
    assert rig.ingest.cursors[("core-a.registry", "registry")]["cursor"] == "s.10"  # hold-back at the hole
    rig.clock.advance(121)
    rep = ex.poll_once()
    assert rig.ingest.cursors[("core-a.registry", "registry")]["cursor"] == "s.13"
    assert "registry_seq_gap_unverified" in rep.partial_reasons
    first = [e for b in rig.ingest.batches for e in b["events"]]
    assert {e["native_event_id"] for e in first} >= {"reg-12", "reg-13"} and "reg-11" not in {
        e["native_event_id"] for e in first}
    t1.commit()
    t1.close()
    ex.poll_once()  # late arrival of seq 11: rescan batch, no checkpoint regression
    cur = rig.ingest.cursors[("core-a.registry", "registry")]
    assert cur["cursor"] == "s.13"
    late = [e for b in rig.ingest.batches if b["scan_mode"] == "rescan" for e in b["events"]]
    assert [e["native_event_id"] for e in late] == ["reg-11"] and late[0]["coverage_marker"] == "late"
    n = rig.ingest.count()
    ex.poll_once()
    assert rig.ingest.count() == n == 13  # reg-12 identity stable, exported once
    # sparse positions carry the true SQL seq (never +1 contiguity)
    sparse = [b for b in rig.ingest.batches if b["scan_mode"] == "fast_poll" and b["partition"] == "registry"]
    assert any(b["from_seq"] == 12 and b["to_seq"] == 13 for b in sparse)


def test_hole_filled_before_grace_is_exported_in_order(rig: Rig) -> None:
    t1 = _admin(rig, autocommit=False)
    insert_reg_event(t1.__enter__())  # seq 1 open
    with _admin(rig) as c2:
        insert_reg_event(c2)  # seq 2
    ex = rig.make()
    ex.poll_once()
    assert ("core-a.registry", "registry") not in rig.ingest.cursors
    t1.commit()
    t1.close()
    ex.poll_once()
    assert rig.ingest.cursors[("core-a.registry", "registry")]["cursor"] == "s.2"
    assert not ex.state.any_skipped()


def test_outbox_exported_without_mark_delivered_and_unprojectable_registry_skipped(rig: Rig) -> None:
    with _admin(rig) as c:
        insert_outbox(c, "msg-1")
        insert_reg_event(c, kind="published")
        insert_reg_event(c, kind="draft_saved", release_id=None)  # no projection: counted, not exported
    ex = rig.make()
    ex.poll_once()
    ob = [e for b in rig.ingest.batches if b["partition"] == "outbox" for e in b["events"]]
    assert ob[0]["native_event_id"] == "msg-1" and ob[0]["source_run_ref"] == "r-ob"
    rg = [e for b in rig.ingest.batches if b["partition"] == "registry" for e in b["events"]]
    assert [e["native_event_id"] for e in rg] == ["reg-1"]
    with _admin(rig) as c:
        assert c.execute("SELECT count(*) FROM outbox WHERE delivered_at IS NOT NULL").fetchone()[0] == 0
    assert rig.ingest.cursors[("core-a.registry", "registry")]["cursor"] == "s.2"  # advanced past the unprojectable


# ---------------------------------------------------------------- transport / recovery
def test_rescan_never_advances_checkpoint_and_dedupes(rig: Rig) -> None:
    seed_run(rig.pg.runtime, "x", middle=1)
    ex = rig.make()
    ex.poll_once()
    rev = rig.ingest.cursors[("core-a.audit", "audit:2026-10-03")]["revision"]
    # simulate local ledger loss for one event -> rescan re-exports it as late
    ex.state._db.execute("DELETE FROM ledger WHERE run_id='x' AND seq=1")
    rep = ex.rescan()
    assert rep.rescan_batches == 1
    assert rig.ingest.cursors[("core-a.audit", "audit:2026-10-03")]["revision"] == rev
    assert rig.ingest.count() == 3


def test_state_loss_adopts_pulso_cursor_and_marks_rebuild(rig: Rig, tmp_path: Any) -> None:
    seed_run(rig.pg.runtime, "x", middle=1)
    rig.make(tmp_path / "s1.sqlite").poll_once()
    ex2 = rig.make(tmp_path / "s2.sqlite")  # fresh state, same Pulso
    seed_run(rig.pg.runtime, "y", middle=0)
    ex2.poll_once()
    assert rig.ingest.count() == 5  # re-exported prefix deduped; no stale-revision wedge
    assert "exporter_rebuild" in ex2.coverage()["missing_reason"]


def test_stale_revision_409_refreshes_cursor_and_retries(rig: Rig) -> None:
    seed_run(rig.pg.runtime, "x", middle=1)
    ex = rig.make()
    ex.poll_once()
    rig.ingest.cursors[("core-a.audit", "audit:2026-10-03")]["revision"] += 5  # another poller advanced it
    seed_run(rig.pg.runtime, "z", middle=0)
    ex.poll_once()
    assert rig.ingest.count() == 5
    assert ex.state.cursor("audit:2026-10-03")[1] == rig.ingest.cursors[("core-a.audit", "audit:2026-10-03")]["revision"]


def test_429_backoff_then_success_and_422_quarantine_stops(rig: Rig) -> None:
    seed_run(rig.pg.runtime, "x", middle=0)
    rig.ingest.fail_next[:] = [(429, {"Retry-After": "3"}), (503, {})]
    ex = rig.make()
    t0 = rig.clock.now
    ex.poll_once()
    assert rig.ingest.count() == 2 and (rig.clock.now - t0).total_seconds() >= 3
    seed_run(rig.pg.runtime, "y", middle=0)
    rig.ingest.fail_next[:] = [(422, {})]
    rep = ex.poll_once()
    assert any(v.startswith("quarantined") for v in rep.stopped.values())
    assert ex.state.pending() is None
    rig.ingest.fail_next[:] = [(401, {})]


def test_401_stops_partition_and_keeps_pending(rig: Rig) -> None:
    seed_run(rig.pg.runtime, "x", middle=0)
    rig.ingest.fail_next[:] = [(401, {})]
    ex = rig.make()
    rep = ex.poll_once()
    assert rep.stopped and ex.state.pending() is not None and rig.ingest.count() == 0


def test_timeout_after_send_reposts_stored_batch_byte_identically(rig: Rig) -> None:
    import httpx

    seed_run(rig.pg.runtime, "x", middle=0)
    ex = rig.make()
    real = rig.client.post
    state = {"n": 0}

    def flaky(url: str, **kw: Any) -> Any:
        r = real(url, **kw)  # server commits
        if url.endswith("observations") and state["n"] == 0:
            state["n"] += 1
            raise httpx.ReadTimeout("lost ACK")
        return r

    rig.client.post = flaky  # type: ignore[method-assign]
    ex.poll_once()
    assert rig.ingest.count() == 2 and len(rig.ingest.raw_bodies) == 1  # re-POST was a duplicate, not re-applied
    assert ex.state.pending() is None


# ---------------------------------------------------------------- privilege / eval exclusion
def test_exporter_role_privileges(rig: Rig) -> None:
    ro = rig.pg.runtime_ro
    with psycopg.connect(ro) as c:
        c.execute("SELECT count(*) FROM audit_events")
        for stmt in ("UPDATE outbox SET delivered_at=now()", "SELECT * FROM runs",
                     "INSERT INTO audit_events(run_id,seq,event_id,type,release,ts,prev_hash,hash,event_json) "
                     "VALUES('r',0,'e','t','r',now(),'a','b','{}')", "SELECT * FROM usage"):
            with pytest.raises(psycopg.errors.Error):
                c.execute(stmt)
            c.rollback()
    eval_as_ro = rig.pg.runtime_ro.rsplit("/", 1)[0] + "/" + rig.pg.eval_db
    with pytest.raises(psycopg.errors.Error):  # CONNECT revoked from PUBLIC and never granted
        psycopg.connect(eval_as_ro)


def test_eval_db_misconfigured_fails_closed(rig: Rig) -> None:
    # pointing the exporter at the eval database (or a superuser) is refused before reading anything
    cfg = ExporterConfig(tenant_id="t", instance="core-a", expected_runtime_db=rig.pg.runtime_db,
                         expected_eval_db=rig.pg.eval_db, binding_ref="b")
    with _admin(rig) as c:
        c.execute(f'GRANT CONNECT ON DATABASE "{rig.pg.eval_db}" TO exporter_ro')
    try:
        with pytest.raises(ExporterError, match="eval_db_misconfigured"):
            with CoreReader(rig.pg.runtime_ro, cfg).snapshot():
                pass
    finally:
        with _admin(rig) as c:
            c.execute(f'REVOKE CONNECT ON DATABASE "{rig.pg.eval_db}" FROM exporter_ro')
    with pytest.raises(ExporterError, match="eval_db_misconfigured"):  # superuser DSN is over-privileged
        with CoreReader(rig.pg.runtime, cfg).snapshot():
            pass
    swapped = ExporterConfig(tenant_id="t", instance="core-a", expected_runtime_db=rig.pg.eval_db,
                             expected_eval_db=rig.pg.runtime_db, binding_ref="b")
    with pytest.raises(ExporterError):
        with CoreReader(rig.pg.runtime_ro, swapped).snapshot():
            pass


def test_schema_drift_blocks(rig: Rig) -> None:
    with _admin(rig) as c:
        c.execute("ALTER TABLE reg_events ADD COLUMN extra text")
        c.execute("GRANT SELECT ON reg_events TO exporter_ro")
    seed_run(rig.pg.runtime, "x", middle=0)
    with pytest.raises(ExporterError, match="schema_drift"):
        rig.make().poll_once()


def test_state_is_not_in_core_dbs_and_wal(rig: Rig) -> None:
    ex = rig.make()
    assert ex.state._db.execute("PRAGMA journal_mode").fetchone()[0] == "wal"
    assert isinstance(ex.state, ExporterState) and isinstance(ex, Exporter)
    assert to_jsonable({"a": 1}) == {"a": 1}

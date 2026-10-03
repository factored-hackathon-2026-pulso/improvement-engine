"""L6 adversarial-review regressions (each test was RED against commit 8a820b1 before its fix)."""

from __future__ import annotations

from typing import Any

import httpx
import psycopg
import pytest
from agent_core.domain import canonical_bytes

from pulso_core_runtime.exporter import CoreReader, ExporterConfig, ExporterError, ExporterState, SimulatedCrash

from .conftest import Rig
from .seed import append_events, insert_outbox, insert_reg_event, seed_run

pytestmark = [pytest.mark.pg, pytest.mark.l6]


def _admin(rig: Rig, autocommit: bool = True) -> psycopg.Connection[Any]:
    return psycopg.connect(rig.pg.runtime, autocommit=autocommit)


def _tamper_allowed(rig: Rig) -> None:
    with _admin(rig) as c:
        c.execute("ALTER TABLE audit_events DISABLE TRIGGER audit_events_no_update")


def _once(fn: Any) -> Any:
    st = {"done": False}

    def hook() -> None:
        if not st["done"]:
            st["done"] = True
            fn()
    return hook


# ------------------------------------------------------------------ ArtifactRef (annex D)
def _is_ref(x: Any) -> bool:
    return isinstance(x, dict) and set(x) == {"id", "digest", "media_type"} and len(x["digest"]) == 64


def test_every_ref_is_an_annex_d_artifact_ref_object(rig: Rig) -> None:
    seed_run(rig.pg.runtime, "r1", middle=1)
    with _admin(rig) as c:
        insert_reg_event(c)
        insert_outbox(c, "m1")
    rig.make().poll_once()
    assert rig.ingest.count() == 5
    for b in rig.ingest.batches:
        for e in b["events"]:
            assert _is_ref(e["source_schema_ref"]) and e["trace_refs"] == []
            assert e["source_event_ref"] is None or _is_ref(e["source_event_ref"])
            assert "#" not in str(e["source_event_ref"])  # a ref never carries a fragment
        for r in b["verification_receipts"]:
            assert _is_ref(r["source_schema_ref"]) and _is_ref(r["source_artifact_ref"])
    # the schema artifact is a real stored artifact (bootstrapped with source_schema_ref=null, kind=schema)
    kinds = {a["body"]["artifact_kind"] for a in rig.ingest.artifacts.values()}
    assert "schema" in kinds and "source_material" in kinds


def test_upload_ref_digest_mismatch_is_not_trusted(rig: Rig) -> None:
    seed_run(rig.pg.runtime, "r1", middle=0)
    ex = rig.make()
    real = rig.client.post

    def lying(url: str, **kw: Any) -> Any:
        r = real(url, **kw)
        if url.endswith("artifacts") and r.status_code in (200, 201) and kw["json"]["artifact_kind"] == "source_material":
            body = r.json()
            body["artifact_ref"]["digest"] = "f" * 64
            return httpx.Response(200, json=body)
        return r

    rig.client.post = lying  # type: ignore[method-assign]
    ex.poll_once()
    assert rig.ingest.count() == 0  # the exporter refuses a ref whose digest differs from what it sent


# ------------------------------------------------------------------ pending batch must never be overwritten
def test_deferred_resume_keeps_pending_and_builds_nothing_new(rig: Rig) -> None:
    seed_run(rig.pg.runtime, "r1", middle=1)
    ex = rig.make(max_retries=1)
    real = rig.client.post
    state = {"drop": True}

    def lossy(url: str, **kw: Any) -> Any:
        if url.endswith("observations"):
            r = real(url, **kw)  # server commits, ACK lost every time
            if state["drop"]:
                raise httpx.ReadTimeout("lost")
            return r
        return real(url, **kw)

    rig.client.post = lossy  # type: ignore[method-assign]
    ex.poll_once()
    first = ex.state.pending()
    assert first is not None and rig.ingest.count() == 3
    seed_run(rig.pg.runtime, "r2", middle=1)
    ex.poll_once()  # resume still fails: must not replace the stored batch with a new one
    assert ex.state.pending() is not None and ex.state.pending().idem_key == first.idem_key  # type: ignore[union-attr]
    state["drop"] = False
    ex.poll_once()
    assert ex.state.pending() is None and rig.ingest.count() == 6


# ------------------------------------------------------------------ tokens (A03): fresh per attempt, per route class
def test_token_minted_per_attempt_and_per_route_class(rig: Rig) -> None:
    seed_run(rig.pg.runtime, "r1", middle=0)
    minted: list[str] = []

    def token_for(route: str) -> str:
        minted.append(route)
        return f"t{len(minted)}"

    seen: list[tuple[str, str]] = []
    real = rig.client.post

    def spy(url: str, **kw: Any) -> Any:
        seen.append((url.rsplit("/", 1)[-1], kw["headers"]["Authorization"]))
        return real(url, **kw)

    rig.client.post = spy  # type: ignore[method-assign]
    rig.ingest.fail_next[:] = [(503, {})]
    rig.make(token_for=token_for).poll_once()
    obs = [t for u, t in seen if u == "observations"]
    assert len(obs) == 2 and len(set(obs)) == 2  # a retry never reuses the token/jti
    assert "artifacts" in minted and minted.count("observations") == 2
    assert {u for u, _ in seen} == {"artifacts", "observations"}


# ------------------------------------------------------------------ chain soundness
def test_rescan_never_exports_rows_that_fail_chain_verification(rig: Rig) -> None:
    seed_run(rig.pg.runtime, "t", middle=2)
    ex = rig.make()
    ex.poll_once()
    _tamper_allowed(rig)
    with _admin(rig) as c:  # content changed, stored hash column left intact
        c.execute("UPDATE audit_events SET event_json=replace(event_json,'\"node_id\":\"n\"','\"node_id\":\"EVIL\"') "
                  "WHERE run_id='t' AND seq=1")
    ex.state._db.execute("DELETE FROM ledger WHERE run_id='t' AND seq=1")  # make seq 1 look unexported
    n = len(rig.ingest.batches)
    rep = ex.rescan()
    assert len(rig.ingest.batches) == n
    assert any("run_chain_broken:t:1" in r for r in rep.partial_reasons)


def test_sweep_detects_content_tamper_behind_an_intact_hash_column(rig: Rig) -> None:
    seed_run(rig.pg.runtime, "t", middle=2)
    ex = rig.make()
    ex.poll_once()
    assert ex.sweep().partial_reasons == []
    _tamper_allowed(rig)
    with _admin(rig) as c:
        c.execute("UPDATE audit_events SET event_json=replace(event_json,'\"node_id\":\"n\"','\"node_id\":\"EVIL\"') "
                  "WHERE run_id='t' AND seq=1")
    rep = ex.sweep()
    assert "run_chain_broken:t:1" in rep.partial_reasons
    assert ex.coverage()["coverage"] == "partial"


@pytest.mark.parametrize("col,value", [("event_id", "'forged'"), ("type", "'run_started'")])
def test_row_columns_must_match_the_signed_event(rig: Rig, col: str, value: str) -> None:
    seed_run(rig.pg.runtime, "c", middle=1)
    _tamper_allowed(rig)
    with _admin(rig) as c:
        c.execute(f"UPDATE audit_events SET {col}={value} WHERE run_id='c' AND seq=1")
    rep = rig.make().poll_once()
    assert "run_chain_broken:c:1" in rep.partial_reasons
    assert "forged" not in {k[4] for k in rig.ingest.ledger}


def test_late_event_after_close_gets_its_own_receipt(rig: Rig) -> None:
    seed_run(rig.pg.runtime, "ok", middle=1)
    ex = rig.make()
    ex.poll_once()
    with psycopg.connect(rig.pg.runtime) as c:
        append_events(c, "ok", ["handoff_resolved"], start_index=3)
    ex.poll_once()
    assert max(r["verified_through_seq"] for r in rig.ingest.verification_receipts if r["run_id"] == "ok") == 3


# ------------------------------------------------------------------ privilege model
@pytest.mark.parametrize("sql", [
    "ALTER ROLE exporter_ro CREATEROLE",
    "GRANT SELECT ON runs TO exporter_ro",
    "GRANT UPDATE ON outbox TO exporter_ro",
    "GRANT INSERT ON audit_events TO exporter_ro",
])
def test_overprivileged_exporter_role_fails_closed(rig: Rig, sql: str) -> None:
    cfg = ExporterConfig(tenant_id="t", instance="core-a", expected_runtime_db=rig.pg.runtime_db,
                         expected_eval_db=rig.pg.eval_db, binding_ref="b")
    with _admin(rig) as c:
        c.execute(sql)
    try:
        with pytest.raises(ExporterError, match="overprivileged"):
            with CoreReader(rig.pg.runtime_ro, cfg).snapshot():
                pass
    finally:
        with _admin(rig) as c:
            for undo in ("ALTER ROLE exporter_ro NOCREATEROLE", "REVOKE ALL ON runs FROM exporter_ro",
                         "REVOKE UPDATE ON outbox FROM exporter_ro", "REVOKE INSERT ON audit_events FROM exporter_ro"):
                c.execute(undo)


# ------------------------------------------------------------------ limits
def test_batch_size_limit_counts_the_batch_digest(rig: Rig) -> None:
    seed_run(rig.pg.runtime, "r1", middle=3)
    probe = rig.make(rig.tmp / "probe.sqlite")
    with probe.reader.snapshot() as snap:
        spec = probe._build_audit(snap)
    assert spec is not None
    body_size = len(canonical_bytes(probe._body(spec)))
    tight = rig.make(rig.tmp / "tight.sqlite", batch_max_bytes=body_size + 10)  # fits without the digest, not with it
    rep = tight.poll_once()
    assert not rep.stopped and rig.ingest.count() == 5
    assert all(len(b) <= body_size + 10 for b in rig.ingest.raw_bodies)


# ------------------------------------------------------------------ sparse holes: huge gaps must not enumerate
def test_huge_sequence_gap_is_a_range_not_an_enumeration(rig: Rig, monkeypatch: pytest.MonkeyPatch) -> None:
    calls = {"n": 0}

    def boom(*a: Any, **k: Any) -> float:
        calls["n"] += 1
        if calls["n"] > 1000:
            raise AssertionError("per-seq hole enumeration")
        return 0.0

    monkeypatch.setattr(ExporterState, "note_hole", boom, raising=False)
    with _admin(rig) as c:
        insert_reg_event(c)
        c.execute("INSERT INTO reg_events(seq, event_json) SELECT 4000000000, event_json FROM reg_events LIMIT 1")
    ex = rig.make(gap_grace_seconds=0)
    rep = ex.poll_once()
    assert "registry_seq_gap_unverified" in rep.partial_reasons
    assert rig.ingest.cursors[("core-a.registry", "registry")]["cursor"] == "s.4000000000"
    with _admin(rig) as c:  # a row that finally commits inside the skipped hole is exported once as late
        c.execute("INSERT INTO reg_events(seq, event_json) SELECT 1234567, event_json FROM reg_events LIMIT 1")
    ex.poll_once()
    late = [e for b in rig.ingest.batches if b["scan_mode"] == "rescan" for e in b["events"]]
    assert [e["native_event_id"] for e in late] == ["reg-1234567"]
    n = rig.ingest.count()
    ex.poll_once()
    assert rig.ingest.count() == n and len(rig.ingest.batches) == 2


# ------------------------------------------------------------------ crash points / races
@pytest.mark.parametrize("point", ["crash_before_post", "crash_after_ack"])
def test_crash_matrix_is_exactly_once_for_all_three_sources(rig: Rig, point: str) -> None:
    seed_run(rig.pg.runtime, "r1", middle=1)
    with _admin(rig) as c:
        insert_reg_event(c)
        insert_outbox(c, "m1")
    sends = {"n": 0}

    def hook() -> None:
        sends["n"] += 1
        if sends["n"] in (1, 3, 5):  # process dies at the 1st, 3rd and 5th send, wherever it falls
            raise SimulatedCrash()

    crashes = 0
    for _ in range(12):
        ex = rig.make()
        setattr(ex, point, hook)
        try:
            ex.poll_once()
            break
        except SimulatedCrash:
            crashes += 1
            ex.close()
    assert crashes == 3
    assert rig.ingest.count() == 5 and len(rig.ingest.ledger) == 5
    assert ex.state.pending() is None and ex.poll_once().batches_sent == 0
    for part in ("audit:2026-10-03", "registry", "outbox"):
        local = ex.state.cursor(part)
        key = (f"core-a.{'audit' if part.startswith('audit') else part}", part)
        assert local is not None and local[1] == rig.ingest.cursors[key]["revision"] == 1
        assert local[0] == rig.ingest.cursors[key]["cursor"]


def test_two_exporters_race_on_the_same_partition(rig: Rig) -> None:
    with _admin(rig) as c:
        for _ in range(5):
            insert_reg_event(c)
    seed_run(rig.pg.runtime, "r1", middle=1)
    a, b = rig.make(rig.tmp / "a.sqlite"), rig.make(rig.tmp / "b.sqlite")
    a.crash_before_post = _once(lambda: b.poll_once())  # B wins the CAS while A holds a built batch
    a.poll_once()
    assert rig.ingest.count() == 8  # nothing lost, nothing duplicated
    for part, src in (("registry", "core-a.registry"), ("audit:2026-10-03", "core-a.audit")):
        srv = rig.ingest.cursors[(src, part)]
        assert b.state.cursor(part)[1] <= srv["revision"]  # type: ignore[index]
    a.poll_once()
    b.poll_once()
    assert rig.ingest.count() == 8
    with _admin(rig) as c:
        insert_reg_event(c)
    a.poll_once()
    b.poll_once()  # B's cursor is behind: stale -> refresh, no duplicate export, no regression
    assert rig.ingest.count() == 9
    assert rig.ingest.cursors[("core-a.registry", "registry")]["cursor"] == "s.6"


# ------------------------------------------------------------------ open-run prefix re-export
def test_open_run_prefix_is_reexported_on_rescan(rig: Rig) -> None:
    seed_run(rig.pg.runtime, "open", middle=3, close=False)
    seed_run(rig.pg.runtime, "done", middle=1)
    ex = rig.make()
    ex.poll_once()
    rev = rig.ingest.cursors[("core-a.audit", "audit:2026-10-03")]["revision"]
    n = rig.ingest.count()
    rep = ex.rescan()
    assert rep.rescan_batches >= 1
    resc = [e for b in rig.ingest.batches if b["scan_mode"] == "rescan" for e in b["events"]]
    assert sorted(e["source_sequence"] for e in resc if e["source_run_ref"] == "open") == [0, 1, 2, 3]
    assert {e["coverage_marker"] for e in resc} == {"open_run"}  # the closed run is not re-exported
    assert rig.ingest.count() == n
    assert rig.ingest.cursors[("core-a.audit", "audit:2026-10-03")]["revision"] == rev
    assert any(r["run_id"] == "open" and r["verified_through_seq"] == 3 for r in rig.ingest.verification_receipts)
    # a run that later closes is exported by the fast path as complete/late and drops out of the re-export set
    with psycopg.connect(rig.pg.runtime) as c:
        append_events(c, "open", ["run_closed"], start_index=4)
    ex.poll_once()
    before = len(rig.ingest.batches)
    ex.rescan()
    assert len(rig.ingest.batches) == before

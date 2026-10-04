"""A 202 receipt with disposition=quarantined (sequence_gap, catalog 1.1.0 ingest) is NOT an ACK: the checkpoint did not
move, so the exporter must keep its cursor, keep the batch for the operator and stop the partition (never loop)."""

import json

import httpx

from platform_exporter import Exporter, ExporterConfig, ExporterState, SqliteSource
from tests.conftest import FakeClock
from tests.platform_db import add_event, make_db


def test_quarantined_receipt_stops_the_partition_and_keeps_the_cursor(tmp_path):
    db = make_db(tmp_path / "p.db")
    add_event(db, 1, "case.viewed")
    posts = []

    def handler(req: httpx.Request) -> httpx.Response:
        if req.url.path.endswith("/cursor"):
            return httpx.Response(200, json={"cursor": None, "cursor_revision": 0, "open_gaps": []})
        if req.url.path.endswith("/artifacts"):
            ref = {"id": "artifact:x", "digest": "x" * 64, "media_type": "application/json"}
            return httpx.Response(201, json={"artifact_ref": ref, "receipt_ref": ref})
        posts.append(1)
        assert len(posts) < 10, "the exporter keeps re-sending a quarantined batch"
        digest = req.headers["Idempotency-Key"]
        return httpx.Response(202, json={"batch_digest": digest, "disposition": "quarantined", "quarantine_reason": "sequence_gap",
                                         "gaps": [[1, 1]], "accepted_event_count": 0, "quarantined_event_count": 1,
                                         "checkpoint_advanced": False, "current_cursor": None, "cursor_revision": 0})

    clock = FakeClock()
    cfg = ExporterConfig(tenant_id="tenant-1", instance="plat-q", binding_ref="binding-1", gap_grace_seconds=0.0)
    ex = Exporter(cfg, SqliteSource(tmp_path / "p.db"), ExporterState(tmp_path / "s.sqlite"),
                  httpx.Client(base_url="http://x", transport=httpx.MockTransport(handler)), clock=clock, sleep=lambda s: None)
    rep = ex.poll_once()
    assert ex.state.stopped() == {cfg.partition: "quarantined:sequence_gap"}
    assert ex.state.cursor(cfg.partition) is None and rep.batches_sent == 0 and len(posts) == 1
    assert [q for q in ex.state._db.execute("SELECT reason FROM batch_quarantine")] == [("sequence_gap",)]
    ex.close(); db.close()

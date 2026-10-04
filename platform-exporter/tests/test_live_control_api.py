"""P1: the exporter against the REAL ingest contract served by the Rust control-api (subprocess, prebuilt binary).

Acceptance: a platform-sim batch reaches POST /internal/v1/platform/observations and is ACKed. Skips cleanly when the
binary is missing (CONTROL_API_BIN, else the claude-seams cargo target dirs). Nothing here touches Podman or Postgres:
the server runs with its in-memory store on a loopback port with the admin channel disabled."""

from __future__ import annotations

import json
import os
import socket
import subprocess
import time
from pathlib import Path

import httpx
import pytest
import rfc8785
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat
from platform_live import PlatformLiveSim

from platform_exporter import Exporter, ExporterConfig, ExporterState, SqliteSource
from platform_exporter.auth import AudienceKey, ServiceTokenSigner
from platform_exporter.config import OBSERVATIONS_PATH
from tests.conftest import FakeClock
from tests.platform_db import add_event, make_db

pytestmark = pytest.mark.sim
BINDING, TENANT, INSTANCE = "exporter-binding-1", "tenant-1", "plat-sim"
CANDIDATES = [os.environ.get("CONTROL_API_BIN", ""), "D:/cargo-targets/claude-seams-p1/debug/control-api.exe",
              "D:/cargo-targets/claude-seams-e4rr/debug/control-api.exe"]


def _bin() -> str:
    for c in CANDIDATES:
        if c and Path(c).is_file():
            return c
    pytest.skip("control-api binary not found (set CONTROL_API_BIN or build seams/ -p control-api)")


def _b64(raw: bytes) -> str:
    import base64
    return base64.urlsafe_b64encode(raw).rstrip(b"=").decode()


def _pub(k: Ed25519PrivateKey) -> str:
    return _b64(k.public_key().public_bytes(Encoding.Raw, PublicFormat.Raw))


@pytest.fixture(scope="module")
def server():
    binary = _bin()
    ob, ex = Ed25519PrivateKey.generate(), Ed25519PrivateKey.generate()
    ring = {"ob": ["core-bridge", "control-api", _pub(ob)], "ex": ["core-bridge", "lab-broker", _pub(ex)]}
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        port = s.getsockname()[1]
    env = {**os.environ, "E2E_PORT": str(port), "CONTROL_API_HOST": "127.0.0.1", "E2E_VERIFY_KEYS": json.dumps(
        {"ring": ring, "ingest": {"binding_ref": BINDING, "tenant_id": TENANT}})}
    env.pop("CONTROL_API_DATABASE_URL", None)
    env.pop("CONTROL_API_ADMIN", None)
    proc = subprocess.Popen([binary], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        deadline = time.time() + 20
        while time.time() < deadline:
            if proc.poll() is not None:
                pytest.fail(f"control-api exited early: {proc.returncode}")
            try:
                socket.create_connection(("127.0.0.1", port), 0.2).close()
                break
            except OSError:
                time.sleep(0.05)
        time.sleep(1.1)  # the server refuses tokens minted before its own boot second
        yield {"base": f"http://127.0.0.1:{port}", "ob": ob, "ex": ex}
    finally:
        proc.terminate()
        proc.wait(10)


def _signer(server, tenant=TENANT) -> ServiceTokenSigner:
    return ServiceTokenSigner({"control-api": AudienceKey("ob", server["ob"]), "lab-broker": AudienceKey("ex", server["ex"])},
                              binding_ref=BINDING, tenant_id=tenant)


def _make(server, tmp_path, name, source_path, instance=INSTANCE, tenant=TENANT, **cfg_over):
    cfg_over.setdefault("gap_grace_seconds", 0.0)
    clock = FakeClock()
    cfg = ExporterConfig(tenant_id=tenant, instance=instance, binding_ref=BINDING,
                         token_for=_signer(server, tenant).token_for, **cfg_over)
    client = httpx.Client(base_url=server["base"], timeout=15)
    ex = Exporter(cfg, SqliteSource(source_path), ExporterState(tmp_path / f"{name}.state.sqlite"), client,
                  clock=clock, sleep=lambda s: clock.advance(s))
    return ex, client, cfg


def _cursor(server, client, cfg):
    tok = _signer(server, cfg.tenant_id).token_for("cursor")
    r = client.get(f"/internal/v1/platform/exporters/{cfg.source_id}/partitions/{cfg.partition}/cursor",
                   headers={"Authorization": "Bearer " + tok})
    assert r.status_code == 200, r.text
    return r.json()


def test_platform_sim_batch_reaches_the_real_observations_endpoint_and_is_acked(server, tmp_path):
    path = tmp_path / "sim.db"
    sim = PlatformLiveSim(seed=7, path=str(path))
    sim.generate(n_cases=20, faults=False)
    sim.conn.commit()
    ex, client, cfg = _make(server, tmp_path, "sim", path, instance="plat-sim-ack")
    try:
        rep = ex.poll_once()
        assert not rep.errors and not rep.stopped and rep.batches_sent >= 1, rep
        cur = _cursor(server, client, cfg)
        assert cur["cursor"] is not None and cur["cursor_revision"] >= 1
    finally:
        ex.close(); client.close(); sim.conn.close()


def test_declared_gap_is_acked_then_late_rows_close_it(server, tmp_path):
    path = tmp_path / "gap.db"
    db = make_db(path)
    for seq in (1, 2, 5, 6):
        add_event(db, seq, "case.viewed")
    ex, client, cfg = _make(server, tmp_path, "gap", path, instance="plat-gap")
    try:
        rep = ex.poll_once()
        assert rep.gaps == [(3, 4)] and not rep.errors and not rep.stopped, rep
        assert ex.state.open_backfills() == [(3, 4)]
        cur = _cursor(server, client, cfg)["cursor"]
        assert cur == "s.6"
        add_event(db, 3, "case.viewed")
        add_event(db, 4, "case.viewed")
        rep2 = ex.poll_once()
        assert not rep2.errors and not rep2.stopped and rep2.batches_sent >= 1, rep2
        assert ex.state.open_backfills() == []
    finally:
        ex.close(); client.close(); db.close()


def _signed_post(server, client, cfg, body, tenant=TENANT):
    body = {**body}
    body.pop("batch_digest", None)
    digest = rfc8785_digest(body)
    raw = rfc8785.dumps({**body, "batch_digest": digest})
    tok = _signer(server, tenant).token_for("observations")
    return client.post(OBSERVATIONS_PATH, content=raw, headers={
        "Authorization": "Bearer " + tok, "Idempotency-Key": digest, "Content-Type": "application/json"})


def rfc8785_digest(body) -> str:
    import hashlib
    return hashlib.sha256(rfc8785.dumps(body)).hexdigest()


def test_undeclared_gap_batch_is_quarantined_and_does_not_move_the_cursor(server, tmp_path):
    path = tmp_path / "ug.db"
    db = make_db(path)
    for seq in (1, 2, 5, 6):
        add_event(db, seq, "case.viewed")
    ex, client, cfg = _make(server, tmp_path, "ug", path, instance="plat-ug")
    try:
        spec = ex._build_fast()
        events = [e for e in spec.events if e["source_event"].get("kind") != "exporter_finding"
                  or e["source_event"]["finding_code"] != "gap_suspected"]
        assert [e["source_sequence"] for e in events if e["source_event"].get("kind") == "domain_event"] == [1, 2, 5, 6]
        body = ex._body(spec)
        body["events"] = events
        r = _signed_post(server, client, cfg, body)
        assert r.status_code == 202, r.text
        j = r.json()
        assert (j["disposition"], j["quarantine_reason"], j["gaps"]) == ("quarantined", "sequence_gap", [[3, 4]])
        assert j["checkpoint_advanced"] is False
        assert _cursor(server, client, cfg)["cursor"] is None
    finally:
        ex.close(); client.close(); db.close()


def test_release_events_are_quarantined_by_the_server_catalog_1_1_0(server, tmp_path):
    path = tmp_path / "rel.db"
    db = make_db(path)
    add_event(db, 1, "case.viewed")
    ex, client, cfg = _make(server, tmp_path, "rel", path, instance="plat-rel")
    try:
        spec = ex._build_fast()
        body = ex._body(spec)
        ev = next(e for e in body["events"] if e["source_event"].get("kind") == "domain_event")
        rel = json.loads(json.dumps(ev))
        rel["source_event"]["event_type"] = "release.published"
        rel["native_event_id"], rel["source_sequence"] = "EVT-R", 2
        rel["source_event_digest"] = "ab" * 32
        body["events"] = [ev, rel]
        body["to_seq"], body["cursor"] = 2, "s.2"
        r = _signed_post(server, client, cfg, body)
        assert r.status_code == 202, r.text
        j = r.json()
        assert j["quarantined_event_count"] >= 1 and j["accepted_event_count"] >= 1, j
    finally:
        ex.close(); client.close(); db.close()


def test_tenant_scoping_a_token_for_another_tenant_is_refused(server, tmp_path):
    path = tmp_path / "ten.db"
    db = make_db(path)
    add_event(db, 1, "case.viewed")
    ex, client, cfg = _make(server, tmp_path, "ten", path, instance="plat-ten")
    try:
        body = ex._body(ex._build_fast())
        r = _signed_post(server, client, cfg, body, tenant="tenant-OTHER")
        assert r.status_code in (401, 403), r.text
        stale = _signer(server)
        bad = ServiceTokenSigner(stale._keys, binding_ref=BINDING, tenant_id=TENANT, now=lambda: time.time() - 7200)
        r2 = client.get(f"/internal/v1/platform/exporters/{cfg.source_id}/partitions/{cfg.partition}/cursor",
                        headers={"Authorization": "Bearer " + bad.token_for("cursor")})
        assert r2.status_code == 401, r2.text
    finally:
        ex.close(); client.close(); db.close()


def test_late_row_inside_the_fast_stream_is_acked_and_leaves_no_open_gap(server, tmp_path):
    path = tmp_path / "late.db"
    db = make_db(path)
    add_event(db, 1, "case.viewed", event_time="2026-03-01T10:30:00Z", ingested_at="2026-03-01T10:31:00Z")
    add_event(db, 2, "case.viewed", event_time="2026-03-01T10:40:00Z", ingested_at="2026-03-01T11:20:00Z")  # late
    add_event(db, 3, "case.viewed", event_time="2026-03-01T11:30:00Z", ingested_at="2026-03-01T11:31:00Z")
    ex, client, cfg = _make(server, tmp_path, "late", path, instance="plat-late", window_seconds=3600)
    try:
        rep = ex.poll_once()
        assert not rep.stopped and not rep.errors, rep
        assert rep.late_events == ["EVT-2"]
        cur = _cursor(server, client, cfg)
        assert cur["cursor"] == "s.3" and cur["open_gaps"] == []
        assert ex.state.open_backfills() == []
    finally:
        ex.close(); client.close(); db.close()


def test_exporter_withheld_unknown_type_row_keeps_continuity_with_the_real_ingest(server, tmp_path):
    path = tmp_path / "unk.db"
    db = make_db(path)
    add_event(db, 1, "case.opened")
    add_event(db, 2, "team.created", payload={"secret_marker": "DO-NOT-FORWARD"})
    add_event(db, 3, "case.assigned")
    ex, client, cfg = _make(server, tmp_path, "unk", path, instance="plat-unk")
    try:
        rep = ex.poll_once()
        assert not rep.stopped and not rep.errors, rep
        assert rep.unknown_event_types == {"team.created": 1}
        assert _cursor(server, client, cfg)["cursor"] == "s.3"
    finally:
        ex.close(); client.close(); db.close()


def test_faulty_sim_run_is_acked_with_unknown_types_quarantined_and_gap_declared(server, tmp_path):
    path = tmp_path / "simf.db"
    sim = PlatformLiveSim(seed=7, path=str(path))
    sim.generate(n_cases=60, faults=True)
    sim.conn.commit()
    ex, client, cfg = _make(server, tmp_path, "simf", path, instance="plat-sim-faults", window_seconds=600)
    try:
        rep = ex.poll_once()
        assert not rep.errors and not rep.stopped, rep
        gap = next(f for f in sim.faults if f["kind"] == "sequence_gap")
        assert (gap["after"] + 1, gap["after"] + gap["size"]) in rep.gaps  # declared by the exporter, ACKed by the server
        assert rep.unknown_event_types["case.escalated"] == 1
        assert ex.state.quarantined()  # release.*/team.* style types never reach the server as domain events
        assert _cursor(server, client, cfg)["cursor"] is not None
    finally:
        ex.close(); client.close(); sim.conn.close()



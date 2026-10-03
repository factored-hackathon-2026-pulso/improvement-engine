"""Real HTTP + A03 service JWT: the ingest fixture is served by uvicorn on a loopback port and the exporter talks to it
with httpx and Ed25519-signed tokens (aud per route class). Core PG is real; the receiver is a double."""

from __future__ import annotations

import socket
import threading
import time
from collections.abc import Callable, Iterator
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import httpx
import pytest
import uvicorn
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from ingest_fixture.app import FixtureAuth, FixtureKey, IngestState, create_app

from pulso_core_runtime.exporter import CoreReader, Exporter, ExporterConfig, ExporterState
from pulso_core_runtime.exporter.__main__ import main as exporter_main
from pulso_core_runtime.exporter.auth import AudienceKey, ExporterTokenSigner
from pulso_core_runtime.internal.auth import b64url_encode

from .conftest import L6Pg
from .seed import insert_outbox, insert_reg_event, seed_run

pytestmark = [pytest.mark.pg, pytest.mark.l6]

TENANT, BINDING = "tenant-1", "binding-1"


@dataclass
class Live:
    url: str
    ingest: IngestState
    keys: dict[str, Ed25519PrivateKey]
    signer: ExporterTokenSigner


@pytest.fixture
def live() -> Iterator[Live]:
    keys = {"control-api": Ed25519PrivateKey.generate(), "lab-broker": Ed25519PrivateKey.generate()}
    fx = FixtureAuth(keys={f"exp-{a}": FixtureKey("core-bridge", a, k.public_key()) for a, k in keys.items()},
                     binding_ref=BINDING, tenant_id=TENANT)
    ingest = IngestState()
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    server = uvicorn.Server(uvicorn.Config(create_app(ingest, fx), host="127.0.0.1", port=port, log_level="error"))
    t = threading.Thread(target=server.run, daemon=True)
    t.start()
    for _ in range(100):
        if server.started:
            break
        time.sleep(0.05)
    signer = ExporterTokenSigner({a: AudienceKey(f"exp-{a}", k) for a, k in keys.items()}, binding_ref=BINDING,
                                 tenant_id=TENANT)
    try:
        yield Live(f"http://127.0.0.1:{port}", ingest, keys, signer)
    finally:
        server.should_exit = True
        t.join(5)


def _exporter(pg: L6Pg, live: Live, tmp: Path, **over: Any) -> Exporter:
    tmp.mkdir(parents=True, exist_ok=True)
    cfg = ExporterConfig(tenant_id=TENANT, instance="core-a", expected_runtime_db=pg.runtime_db,
                         expected_eval_db=pg.eval_db, binding_ref=BINDING,
                         token_for=over.pop("token_for", live.signer.token_for), **over)
    return Exporter(cfg, CoreReader(pg.runtime_ro, cfg), ExporterState(tmp / "s.sqlite"),
                    httpx.Client(base_url=live.url, timeout=10))


def test_full_flow_over_http_with_route_scoped_tokens(pg: L6Pg, live: Live, tmp_path: Path) -> None:
    import psycopg
    seed_run(pg.runtime, "r1", middle=2)
    with psycopg.connect(pg.runtime, autocommit=True) as c:
        insert_reg_event(c)
        insert_outbox(c, "m1")
    ex = _exporter(pg, live, tmp_path)
    try:
        rep = ex.poll_once()
        assert not rep.stopped and live.ingest.count() == 6
        assert ex.poll_once().batches_sent == 0
    finally:
        ex.close()
    routes = {(a["aud"], a["scope"], a["purpose"]) for a in live.ingest.auth_log}
    assert routes >= {("control-api", "observations", "platform_observations"),
                      ("lab-broker", "artifact_write", "artifact_upload")}
    assert all(a["iss"] == "core-bridge" and a["sub"] == BINDING and a["tenant_id"] == TENANT
               for a in live.ingest.auth_log)
    jtis = [a["jti"] for a in live.ingest.auth_log]
    assert len(jtis) == len(set(jtis))  # every HTTP attempt carried a fresh jti


def test_replayed_wrong_audience_and_unknown_key_tokens_are_refused(pg: L6Pg, live: Live, tmp_path: Path) -> None:
    seed_run(pg.runtime, "r1", middle=0)
    tok = live.signer.token_for("observations")
    h = {"Authorization": "Bearer " + tok}
    assert httpx.get(live.url + "/internal/v1/platform/exporters/core-a.audit/partitions/audit:x/cursor",
                     headers=h).status_code == 200
    assert httpx.get(live.url + "/internal/v1/platform/exporters/core-a.audit/partitions/audit:x/cursor",
                     headers=h).status_code == 401  # jti replay
    # an observations token presented on the artifact route (wrong audience) never writes
    ex = _exporter(pg, live, tmp_path, token_for=lambda route: live.signer.token_for("observations"))
    try:
        rep = ex.poll_once()
    finally:
        ex.close()
    assert rep.deferred >= 1 and live.ingest.count() == 0 and not live.ingest.artifacts
    # a token signed by an unregistered key stops the partition (401) and keeps the pending batch
    rogue = ExporterTokenSigner({a: AudienceKey(f"exp-{a}", Ed25519PrivateKey.generate()) for a in live.keys},
                                binding_ref=BINDING, tenant_id=TENANT)
    ex = _exporter(pg, live, tmp_path / "rogue", token_for=rogue.token_for)
    try:
        assert ex.poll_once().deferred >= 1 and live.ingest.count() == 0
    finally:
        ex.close()


def test_cross_tenant_and_foreign_binding_are_denied(pg: L6Pg, live: Live, tmp_path: Path) -> None:
    seed_run(pg.runtime, "r1", middle=0)
    other = ExporterTokenSigner({a: AudienceKey(f"exp-{a}", k) for a, k in live.keys.items()},
                                binding_ref="someone-else", tenant_id=TENANT)
    ex = _exporter(pg, live, tmp_path, token_for=other.token_for)
    try:
        ex.poll_once()
    finally:
        ex.close()
    assert live.ingest.count() == 0 and not live.ingest.artifacts


def test_cli_once_rescan_and_sweep_against_live_ingest(pg: L6Pg, live: Live, tmp_path: Path,
                                                        capsys: pytest.CaptureFixture[str]) -> None:
    seed_run(pg.runtime, "r1", middle=1)
    seed_run(pg.runtime, "open", middle=2, close=False)
    env = {"CORE_EXPORT_DATABASE_URL": pg.runtime_ro, "EXPECTED_RUNTIME_DB": pg.runtime_db,
           "EXPECTED_EVAL_DB": pg.eval_db, "PULSO_TENANT_ID": TENANT, "PULSO_CORE_INSTANCE": "core-a",
           "PULSO_INGEST_BASE_URL": live.url, "PULSO_EXPORTER_BINDING_REF": BINDING,
           "PULSO_EXPORTER_STATE_DIR": str(tmp_path / "state")}
    for var, aud in (("PULSO_EXPORTER_KEY_CONTROL_API", "control-api"), ("PULSO_EXPORTER_KEY_LAB_BROKER", "lab-broker")):
        f = tmp_path / f"{aud}.key"
        f.write_text(b64url_encode(live.keys[aud].private_bytes_raw()), encoding="ascii")
        env[var] = str(f)
        env[var + "_KID"] = f"exp-{aud}"
    assert exporter_main(["--once"], env) == 0
    assert live.ingest.count() == 6
    assert exporter_main(["--rescan"], env) == 0
    assert exporter_main(["--sweep"], env) == 0
    out = capsys.readouterr().out
    assert '"stopped": {}' in out and "open_runs:1" in out
    assert exporter_main(["--once"], {k: v for k, v in env.items() if k != "PULSO_TENANT_ID"}) == 2  # config error


def _unused(_: Callable[..., Any]) -> None:  # keep typing imports honest
    return None

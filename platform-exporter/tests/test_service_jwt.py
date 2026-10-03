"""PL-0009: per-attempt service-JWT minting (A02/A03), verified by the ingest fixture with the PUBLIC key only."""

from __future__ import annotations

import base64
import json
import logging
import os
import sqlite3
import time
from pathlib import Path

import pytest
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from fastapi.testclient import TestClient
from ingest_fixture.app import FixtureAuth, FixtureKey, IngestState, create_app

from platform_exporter import Exporter, ExporterConfig, ExporterState, SqliteSource
from platform_exporter.__main__ import build_from_env, main
from platform_exporter.auth import AudienceKey, RuntimeConfigInvalid, ServiceTokenSigner, load_seed_file
from tests.platform_db import add_event, make_db

T0 = 1_800_000_000.0
BINDING, TENANT = "binding-1", "tenant-1"


def _b64(raw: bytes) -> str:
    return base64.urlsafe_b64encode(raw).rstrip(b"=").decode()


def _dec(part: str) -> dict:
    return json.loads(base64.urlsafe_b64decode(part + "=" * (-len(part) % 4)))


def _claims(tok: str) -> dict:
    return _dec(tok.split(".")[1])


def _keys():
    return Ed25519PrivateKey.from_private_bytes(b"\x01" * 32), Ed25519PrivateKey.from_private_bytes(b"\x02" * 32)


def _signer(*, with_lab=True, now=lambda: T0, ttl=60):
    ctl, lab = _keys()
    keys = {"control-api": AudienceKey("exporter-control-api", ctl)}
    if with_lab:
        keys["lab-broker"] = AudienceKey("exporter-lab-broker", lab)
    return ServiceTokenSigner(keys, binding_ref=BINDING, tenant_id=TENANT, ttl_seconds=ttl, now=now)


def test_observations_token_claims_and_header():
    tok = _signer().token_for("observations")
    assert _dec(tok.split(".")[0]) == {"alg": "EdDSA", "kid": "exporter-control-api", "typ": "JWT"}
    c = _claims(tok)
    assert c["iss"] == "core-bridge" and c["aud"] == "control-api" and c["sub"] == BINDING
    assert c["tenant_id"] == TENANT and c["scope"] == "observations" and c["purpose"] == "platform_observations"
    assert c["iat"] == int(T0) and c["exp"] == int(T0) + 60 and c["jti"]


def test_cursor_token_is_control_api_observations_scope_without_purpose():
    c = _claims(_signer().token_for("cursor"))
    assert c["aud"] == "control-api" and c["scope"] == "observations" and "purpose" not in c


def test_artifacts_token_targets_lab_broker_with_its_own_key_and_kid():
    tok = _signer().token_for("artifacts")
    assert _dec(tok.split(".")[0])["kid"] == "exporter-lab-broker"
    c = _claims(tok)
    assert c["aud"] == "lab-broker" and c["scope"] == "artifact_write" and c["purpose"] == "artifact_upload"


def test_artifacts_without_a_lab_broker_key_fails_closed_never_reusing_the_control_api_key():
    with pytest.raises(RuntimeConfigInvalid, match="lab-broker"):
        _signer(with_lab=False).token_for("artifacts")


def test_every_call_mints_a_unique_jti():
    s = _signer()
    assert len({_claims(s.token_for("observations"))["jti"] for _ in range(50)}) == 50


def test_exp_window_follows_the_clock_and_is_capped_at_five_minutes():
    t = [T0]
    s = _signer(now=lambda: t[0], ttl=30)
    a = _claims(s.token_for("observations"))
    t[0] += 100
    b = _claims(s.token_for("observations"))
    assert (a["exp"] - a["iat"], b["iat"] - a["iat"]) == (30, 100)
    for bad in (0, 301):
        with pytest.raises(ValueError):
            _signer(ttl=bad)


def test_unknown_route_is_rejected():
    with pytest.raises(KeyError):
        _signer().token_for("other")


# ---------------------------------------------------------------- against the ingest fixture (public keys only)
def _verifying_rig(tmp_path: Path, signer: ServiceTokenSigner):
    ctl, lab = _keys()
    auth = FixtureAuth({"exporter-control-api": FixtureKey("core-bridge", "control-api", ctl.public_key()),
                        "exporter-lab-broker": FixtureKey("core-bridge", "lab-broker", lab.public_key())},
                       binding_ref=BINDING, tenant_id=TENANT)
    ingest = IngestState()
    client = TestClient(create_app(ingest, auth), base_url="http://ingest.fixture")
    db = make_db(tmp_path / "p.db")
    for i in (1, 2, 3):
        add_event(db, i, "case.viewed")
    cfg = ExporterConfig(tenant_id=TENANT, instance="plat-a", binding_ref=BINDING, gap_grace_seconds=0.0,
                         token_for=signer.token_for)
    ex = Exporter(cfg, SqliteSource(tmp_path / "p.db"), ExporterState(tmp_path / "state.sqlite"), client,
                  sleep=lambda s: None)
    return ex, ingest, db


def test_exporter_is_accepted_by_a_fixture_that_verifies_eddsa_with_the_public_key(tmp_path):
    ex, ingest, db = _verifying_rig(tmp_path, _signer(now=time.time))
    rep = ex.poll_once()
    ex.close()
    db.close()
    assert rep.batches_sent >= 1 and not rep.stopped
    assert ingest.auth_log and all(a["sub"] == BINDING and a["tenant_id"] == TENANT for a in ingest.auth_log)
    assert {(a["aud"], a["scope"]) for a in ingest.auth_log} == {
        ("control-api", "observations"), ("lab-broker", "artifact_write")}  # cursor/observations + schema upload
    assert len({a["jti"] for a in ingest.auth_log}) == len(ingest.auth_log)


def test_a_retry_mints_a_new_token_so_the_receiver_replay_store_accepts_it(tmp_path):
    ex, ingest, db = _verifying_rig(tmp_path, _signer(now=time.time))
    ingest.fail_next.extend([(503, {}), (429, {}), (503, {})])
    rep = ex.poll_once()
    ex.close()
    db.close()
    assert rep.batches_sent >= 1 and not rep.stopped  # a replayed jti would be 401 -> partition stopped
    obs = [a for a in ingest.auth_log if a["purpose"] == "platform_observations"]
    assert len(obs) >= 4 and len({a["jti"] for a in obs}) == len(obs)


def test_fixture_rejects_a_token_signed_with_the_wrong_key(tmp_path):
    other = Ed25519PrivateKey.from_private_bytes(b"\x09" * 32)
    bad = ServiceTokenSigner({"control-api": AudienceKey("exporter-control-api", other)}, binding_ref=BINDING,
                             tenant_id=TENANT, now=time.time)
    ex, ingest, db = _verifying_rig(tmp_path, bad)
    rep = ex.poll_once()
    ex.close()
    db.close()
    assert rep.batches_sent == 0 and not ingest.batches


# ---------------------------------------------------------------- key file loading / fail closed / no leakage
SEED = _b64(b"\x07" * 32)


def _env(tmp_path: Path, **over) -> dict[str, str]:
    kf = tmp_path / "exporter-control-api.key"
    kf.write_text(SEED, encoding="ascii")
    env = {"PLATFORM_DB_URL": str(tmp_path / "p.db"), "PULSO_CONTROL_API_URL": "http://ingest.fixture",
           "PULSO_TENANT_ID": TENANT, "PLATFORM_INSTANCE": "plat-a", "PULSO_BINDING_REF": BINDING,
           "EXPORTER_STATE_PATH": str(tmp_path / "state.sqlite"), "PULSO_EXPORTER_KEY_CONTROL_API": str(kf)}
    env.update(over)
    return env


def test_build_from_env_reads_the_key_file_and_mints_verifiable_tokens(tmp_path):
    make_db(tmp_path / "p.db").close()
    ex, _ = build_from_env(_env(tmp_path))
    tok = ex.cfg.token("observations")
    ex.close()
    pub = Ed25519PrivateKey.from_private_bytes(b"\x07" * 32).public_key()
    h, b, s = tok.split(".")
    pub.verify(base64.urlsafe_b64decode(s + "=" * (-len(s) % 4)), f"{h}.{b}".encode())
    assert _claims(tok)["sub"] == BINDING


@pytest.mark.parametrize("content", [None, "", "not base64 !!", _b64(b"short"), _b64(b"\x01" * 33)])
def test_missing_or_malformed_key_file_fails_closed_without_echoing_content(tmp_path, content):
    env = _env(tmp_path)
    kf = Path(env["PULSO_EXPORTER_KEY_CONTROL_API"])
    if content is None:
        kf.unlink()
    else:
        kf.write_text(content, encoding="ascii")
    with pytest.raises(RuntimeConfigInvalid) as ei:
        build_from_env(env)
    msg = str(ei.value)
    assert msg.startswith("pulso:runtime_config_invalid") and "PULSO_EXPORTER_KEY_CONTROL_API" in msg
    assert not content or content not in msg


def test_unset_key_variable_fails_closed(tmp_path):
    env = _env(tmp_path)
    del env["PULSO_EXPORTER_KEY_CONTROL_API"]
    with pytest.raises(RuntimeConfigInvalid, match="PULSO_EXPORTER_KEY_CONTROL_API"):
        build_from_env(env)


def test_main_exits_2_with_the_marker_and_no_secret_on_stderr(tmp_path, monkeypatch, capsys):
    env = _env(tmp_path)
    Path(env["PULSO_EXPORTER_KEY_CONTROL_API"]).write_text(SEED + "x!", encoding="ascii")
    for k in list(os.environ):
        if k.startswith(("PULSO_", "PLATFORM_", "EXPORTER_")):
            monkeypatch.delenv(k)
    for k, v in env.items():
        monkeypatch.setenv(k, v)
    assert main() == 2
    err = capsys.readouterr().err
    assert "pulso:runtime_config_invalid" in err and SEED not in err


def test_static_token_is_only_an_explicit_dev_override(tmp_path):
    make_db(tmp_path / "p.db").close()
    env = _env(tmp_path, PULSO_SERVICE_TOKEN="dev-static-token")
    with pytest.raises(RuntimeConfigInvalid, match="PULSO_DEV_STATIC_TOKEN"):  # set but not explicitly enabled
        build_from_env(env)
    Path(env["PULSO_EXPORTER_KEY_CONTROL_API"]).unlink()  # explicit override works without any key file
    ex, _ = build_from_env({**env, "PULSO_DEV_STATIC_TOKEN": "1"})
    assert ex.cfg.token("observations") == "dev-static-token"
    ex.close()


def test_load_seed_file_accepts_trailing_newline(tmp_path):
    p = tmp_path / "k"
    p.write_text(SEED + "\n", encoding="ascii")
    assert isinstance(load_seed_file(str(p), var="X"), Ed25519PrivateKey)


def test_neither_seed_nor_tokens_reach_logs_state_or_error_text(tmp_path, caplog):
    caplog.set_level(logging.DEBUG)
    ex, ingest, db = _verifying_rig(tmp_path, _signer(now=time.time))
    ingest.fail_next.append((503, {}))
    ex.poll_once()
    ex.close()
    db.close()
    seen = {a["jti"] for a in ingest.auth_log}
    con = sqlite3.connect(tmp_path / "state.sqlite")
    dump = "\n".join(con.iterdump()).encode()
    con.close()
    secret = b"\x01" * 32
    for blob in ((tmp_path / "state.sqlite").read_bytes(), dump, caplog.text.encode()):
        assert secret not in blob and _b64(secret).encode() not in blob and b"Bearer" not in blob
        assert not any(j.encode() in blob for j in seen)

"""PG-backed L2 tests (real Postgres 16): composed app readiness/version, jti replay table, pinned registry.

Pin-checkout test helpers (`testing`, `tests.registry`) are used only for seeding the registry; they are the
documented doubles (FakeEvaluator, FakeClock, FakeIds) of this file."""

from __future__ import annotations

import json
import time
import uuid
from datetime import timedelta
from pathlib import Path
from typing import Any

import psycopg
import pytest
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from fastapi.testclient import TestClient

from pulso_core_runtime import main as runtime_main
from pulso_core_runtime.internal.auth import b64url_encode, sign_service_jwt
from pulso_core_runtime.internal.store import PgJtiStore, ensure_schema, schema_ready
from pulso_core_runtime.pin import PIN_UNAVAILABLE, PinnedRegistryPort

from .conftest import PgDbs

pytestmark = [pytest.mark.runtime, pytest.mark.pg]


def _pub(key: Ed25519PrivateKey) -> str:
    from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat
    return b64url_encode(key.public_key().public_bytes(Encoding.Raw, PublicFormat.Raw))


def _seed(key: Ed25519PrivateKey) -> str:
    from cryptography.hazmat.primitives.serialization import Encoding, NoEncryption, PrivateFormat
    return b64url_encode(key.private_bytes(Encoding.Raw, PrivateFormat.Raw, NoEncryption()))


@pytest.fixture
def keys(tmp_path: Path) -> dict[str, Any]:
    ident, staff, svc = (Ed25519PrivateKey.generate() for _ in range(3))
    (tmp_path / "identity.json").write_text(json.dumps(
        {"principal_keys": {"id1": _pub(ident)}, "delegation_keys": {"id1": _pub(ident)}}))
    (tmp_path / "staff.json").write_text(json.dumps({"principal_keys": {"st1": _pub(staff)}}))
    (tmp_path / "service.json").write_text(json.dumps(
        {"keys": {"cp1": {"iss": "control-api", "aud": "core-bridge", "key": _pub(svc)}}}))
    for name, kid, key in (("bridge-identity", "id1", ident), ("bridge-staff", "st1", staff),
                           ("bridge-callback", "cb1", Ed25519PrivateKey.generate()),
                           ("bridge-executor", "ex1", Ed25519PrivateKey.generate())):
        (tmp_path / f"{name}.json").write_text(json.dumps({"kid": kid, "key": _seed(key)}))
    return {"dir": tmp_path, "svc": svc}


def _env(pg: PgDbs, keys: dict[str, Any], **extra: str) -> dict[str, str]:
    d = keys["dir"]
    return {"AGENTCORE_REGISTRY_DSN": pg.runtime, "AGENTCORE_EVAL_DSN": pg.eval,
            "AGENTCORE_KEYS_FINGERPRINT": "k1:" + b64url_encode(b"f" * 32).replace("-", "A").replace("_", "B") + "=",
            "AGENTCORE_KEYS_TOKEN_MAP": "k1:" + b64url_encode(b"m" * 32).replace("-", "A").replace("_", "B") + "=",
            "PULSO_IDENTITY_KEYS": str(d / "identity.json"), "PULSO_STAFF_KEYS": str(d / "staff.json"),
            "PULSO_SERVICE_KEYS": str(d / "service.json"), "PULSO_PORT": "0",
            "PULSO_BRIDGE_IDENTITY_SIGNER": str(d / "bridge-identity.json"),
            "PULSO_BRIDGE_STAFF_SIGNER": str(d / "bridge-staff.json"),
            "PULSO_BRIDGE_CALLBACK_SIGNER": str(d / "bridge-callback.json"),
            "PULSO_BRIDGE_EXECUTOR_SIGNER": str(d / "bridge-executor.json"),
            "PULSO_LAB_BROKER_URL": "http://127.0.0.1:9", "PULSO_CONTROL_API_URL": "http://127.0.0.1:9", "PULSO_TENANT_ID": "t1",
            "PULSO_SHA": "abc1234", "PULSO_IMAGE_DIGEST": "sha256:" + "a" * 64, **extra}


def _compose(env: dict[str, str]) -> tuple[int, Any, str]:
    import io

    captured: list[Any] = []
    err = io.StringIO()
    code = runtime_main.run([], env=env, stderr=err, serve=lambda app, **kw: captured.append(app))
    return code, (captured[0] if captured else None), err.getvalue()


def _token(svc: Ed25519PrivateKey, purpose: str = "version_probe") -> str:
    now = int(time.time())
    return sign_service_jwt(svc, kid="cp1", claims={
        "iss": "control-api", "aud": "core-bridge", "sub": "worker:1", "tenant_id": "t", "purpose": purpose,
        "iat": now, "exp": now + 60, "jti": uuid.uuid4().hex})


def test_composed_app_ready_and_version(pg: PgDbs, keys: dict[str, Any]) -> None:
    ensure_schema(pg.runtime)
    code, app, err = _compose(_env(pg, keys))
    assert code == 0, err
    c = TestClient(app)
    assert c.get("/healthz").status_code == 200
    assert c.get("/readyz").json() == {"status": "ready"}
    r = c.get("/internal/v1/version", headers={"Authorization": f"Bearer {_token(keys['svc'])}"})
    body = r.json()
    assert r.status_code == 200
    assert body["agent_core_sha"] == "86a767474042a566a0dbd6ed23588959f27ebdb3"
    assert body["contracts_version"] == "1.3.0" and body["pulso_sha"] == "abc1234"
    assert body["image_digest"] == "sha256:" + "a" * 64
    assert body["runtime_profile"] == "agent_core_real" and body["doubles"]
    # Core's own problem+json is untouched outside /internal/v1; ours uses the envelope inside.
    assert "code" in c.get("/internal/v1/version").json()


def test_readyz_503_names_bridge_schema_when_missing(pg: PgDbs, keys: dict[str, Any]) -> None:
    code, app, _ = _compose(_env(pg, keys))  # startup migrates the bridge schema itself
    assert code == 0
    with psycopg.connect(pg.runtime, autocommit=True) as conn:
        conn.execute("DROP SCHEMA pulso_bridge CASCADE")
    r = TestClient(app).get("/readyz")
    assert r.status_code == 503 and r.json()["failed"] == ["bridge_schema"]


def test_readyz_503_names_eval_db_when_down(pg: PgDbs, keys: dict[str, Any]) -> None:
    ensure_schema(pg.runtime)
    code, app, _ = _compose(_env(pg, keys))
    assert code == 0
    with psycopg.connect(pg.admin, autocommit=True) as admin:
        admin.execute(f'DROP DATABASE "{pg.eval.rsplit("/", 1)[1]}" WITH (FORCE)')
    r = TestClient(app).get("/readyz")
    assert r.status_code == 503 and r.json()["failed"] == ["eval_db"]
    assert pg.eval not in r.text  # no DSN, no detail


def test_readyz_503_names_postgres_when_runtime_db_down(pg: PgDbs, keys: dict[str, Any]) -> None:
    ensure_schema(pg.runtime)
    code, app, _ = _compose(_env(pg, keys))
    assert code == 0
    with psycopg.connect(pg.admin, autocommit=True) as admin:
        admin.execute(f'DROP DATABASE "{pg.runtime.rsplit("/", 1)[1]}" WITH (FORCE)')
    failed = TestClient(app).get("/readyz").json()["failed"]
    assert "postgres" in failed and "bridge_schema" in failed


def test_key_files_check_fails_when_file_missing(pg: PgDbs, keys: dict[str, Any]) -> None:
    ensure_schema(pg.runtime)
    code, app, _ = _compose(_env(pg, keys))
    assert code == 0
    (keys["dir"] / "staff.json").write_text("")
    assert TestClient(app).get("/readyz").json()["failed"] == ["key_files"]


def test_bad_config_exit_2_names_piece_not_value(pg: PgDbs, keys: dict[str, Any]) -> None:
    env = _env(pg, keys, AGENTCORE_EVAL_DSN=pg.runtime)  # eval DSN == runtime DSN
    code, app, err = _compose(env)
    assert code == 2 and app is None
    assert "pulso:runtime_config_invalid" in err and "--eval-dsn" in err and pg.runtime not in err


def test_composed_without_service_keys_exit_2(pg: PgDbs, keys: dict[str, Any]) -> None:
    code, _, err = _compose(_env(pg, keys, PULSO_SERVICE_KEYS=str(keys["dir"] / "absent.json")))
    assert code == 2 and "service keys" in err


def test_jti_replay_table_survives_restart_and_races(pg: PgDbs) -> None:
    ensure_schema(pg.runtime)
    ensure_schema(pg.runtime)  # idempotent
    assert schema_ready(pg.runtime)
    from datetime import UTC, datetime

    exp = datetime.now(UTC) + timedelta(minutes=5)
    a, b = PgJtiStore(pg.runtime), PgJtiStore(pg.runtime)  # two "processes"
    assert a.consume("control-api", "j1", exp) is True
    assert b.consume("control-api", "j1", exp) is False
    assert b.consume("core-bridge", "j1", exp) is True  # replay scope is (iss, jti)
    # expired rows are purged
    assert a.consume("control-api", "old", datetime.now(UTC) - timedelta(seconds=1)) is True
    assert a.consume("control-api", "old", exp) is True


# ---- PinnedRegistryPort against the real PG registry --------------------------------------------------

def _seed_two_releases(dsn: str) -> tuple[Any, str, str, Any, Any]:
    from agent_core.registry import PgRegistryStore, PostgresRegistry
    from agent_core.registry.service import RegistryService

    try:
        from agent_core.registry.models import Origin
        from testing.fakes.clock import FakeClock
        from testing.fakes.ids import FakeIds
        from tests.registry.helpers import AGENT, REGISTRY_DEMO, admin, human, prompt_draft, suite_draft  # noqa: F401
        from tests.registry.service_world import SUITE, FakeEvaluator
    except ImportError:
        pytest.skip("pin checkout test helpers not importable")
    store = PgRegistryStore(lambda: psycopg.connect(dsn, autocommit=False))
    service = RegistryService(store, FakeEvaluator(), FakeClock(), FakeIds())
    service.import_seed(admin(), REGISTRY_DEMO)
    ana = human()

    def publish(version: str, key: str) -> str:
        p = service.create_proposal(ana, AGENT, Origin.manual, "t")
        service.put_draft(ana, p.proposal_id, [prompt_draft(version=version, text=f"Texto {version}."), SUITE],
                          expected_rev=0)
        service.freeze(ana, p.proposal_id)
        service.evaluate(ana, p.proposal_id, "disputas-suite")
        h = service.get_proposal(p.proposal_id).proposal.candidate_hash or ""
        service.approve(ana, p.proposal_id, h)
        return service.publish(ana, p.proposal_id, key).release_id

    r1, r2 = publish("1.1.0", "k1"), publish("1.2.0", "k2")
    clock = FakeClock()
    reg = PostgresRegistry(store, clock)
    return reg, r1, r2, (service, clock), (AGENT, admin)


def _principal(**attrs: str) -> Any:
    from testing.builders import principal
    return principal(type="builder", id="b", roles=["constructor"], attrs=attrs)


def test_pin_returns_pinned_release_even_after_alias_moves(pg: PgDbs) -> None:
    from agent_core.domain import AgentSelector

    reg, r1, r2, (service, clock), (agent, admin) = _seed_two_releases(pg.runtime)
    port = PinnedRegistryPort(reg)
    sel = AgentSelector.parse(f"{agent}@staging")
    assert port.resolve_release(sel, _principal()).id == r2  # no attr: falls through to the alias (moved to r2)
    pinned = port.resolve_release(sel, _principal(pin_release_id=r1))
    assert pinned.id == r1 and pinned.status == "active"


def test_pin_revoked_between_precheck_and_start(pg: PgDbs) -> None:
    from agent_core.domain import AgentSelector

    reg, r1, r2, (service, clock), (agent, admin) = _seed_two_releases(pg.runtime)
    port = PinnedRegistryPort(reg)
    sel = AgentSelector.parse(f"{agent}@staging")
    assert port.resolve_release(sel, _principal(pin_release_id=r1)).id == r1
    service.revoke(admin(), r1, "bad")
    clock.advance(timedelta(seconds=6))  # status TTL
    with pytest.raises(KeyError, match=PIN_UNAVAILABLE):
        port.resolve_release(sel, _principal(pin_release_id=r1))


def test_pin_unknown_or_mismatching_agent_or_version(pg: PgDbs) -> None:
    from agent_core.domain import AgentSelector

    reg, r1, r2, _, (agent, _admin) = _seed_two_releases(pg.runtime)
    port = PinnedRegistryPort(reg)
    with pytest.raises(KeyError, match=PIN_UNAVAILABLE):
        port.resolve_release(AgentSelector.parse(f"{agent}@staging"), _principal(pin_release_id="nope"))
    with pytest.raises(KeyError, match=PIN_UNAVAILABLE):
        port.resolve_release(AgentSelector.parse("otro-agente@staging"), _principal(pin_release_id=r1))
    with pytest.raises(KeyError, match=PIN_UNAVAILABLE):
        port.resolve_release(AgentSelector.parse(f"{agent}@9.9.9"), _principal(pin_release_id=r1))

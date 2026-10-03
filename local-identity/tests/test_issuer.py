"""First RED: the human issuer routes, service-JWT guard, replay tables and Principal JWS shape."""

from __future__ import annotations

import json
import uuid
from datetime import timedelta
from pathlib import Path
from typing import Any

import pytest
from conftest import ADMIN, HASH, SUPERVISOR, TENANT, Env, env_for, proposal_target, release_target
from fastapi.testclient import TestClient

from local_identity.app import build_app
from local_identity.client import IssuerError, ServiceSigner
from local_identity.config import load_config
from local_identity.keys import b64url_decode

CMD_URL = "/internal/v1/human/command-authorizations/issue"
SESSION_URL = "/internal/v1/human/session-assertions/issue"


def _decode(jws: str) -> tuple[dict[str, Any], dict[str, Any]]:
    h, p, _ = jws.split(".")
    return json.loads(b64url_decode(h)), json.loads(b64url_decode(p))


def _cmd(**over: object) -> dict[str, Any]:
    args: dict[str, Any] = {
        "tenant_id": TENANT,
        "actor_ref": SUPERVISOR,
        "command_ref": "cmd-1",
        "operation": "approve",
        "target": proposal_target(),
        "challenge_ref": "chal-1",
        "nonce": uuid.uuid4().hex,
    }
    args.update(over)
    return args


def test_command_authorization_is_exact_core_principal_jws(env: Env) -> None:
    out = env.client.command_authorization(**_cmd())
    header, body = _decode(out.authorization_jws)
    assert header == {"alg": "EdDSA", "kid": out.kid, "typ": "principal+jws"}
    assert out.kid.startswith("local-sim-human-")
    assert body["type"] == "builder" and body["id"] == SUPERVISOR
    assert body["roles"] == ["constructor", "aprobador"] and body["scopes"] == []
    assert body["auth"] == {"level": "step_up", "at": "2026-10-03T12:00:00.000Z", "simulated": True}
    assert body["exp"] == "2026-10-03T12:01:00.000Z"
    assert out.exp == int(env.clock.now.timestamp()) + 60
    attrs = body["attrs"]
    assert attrs["actor"] == "human" and attrs["tenant"] == TENANT
    assert (attrs["operation"], attrs["proposal_id"], attrs["candidate_hash"], attrs["expected_revision"]) == (
        "approve",
        "prop-1",
        HASH,
        "3",
    )
    assert (attrs["command_ref"], attrs["challenge_ref"]) == ("cmd-1", "chal-1")
    assert set(body) == {"type", "id", "roles", "scopes", "attrs", "auth", "exp"}
    assert out.metadata["auth_simulated"] is True


def test_release_target_binding_has_no_candidate_hash(env: Env) -> None:
    out = env.client.command_authorization(**_cmd(operation="promote", target=release_target()))
    attrs = _decode(out.authorization_jws)[1]["attrs"]
    assert attrs["target_kind"] == "release" and attrs["release_id"] == "rel-1"
    assert (attrs["agent_id"], attrs["alias"]) == ("agent-1", "prod")
    assert "candidate_hash" not in attrs and "proposal_id" not in attrs


def test_binding_digest_changes_with_operation_and_hash(env: Env) -> None:
    a = env.client.command_authorization(**_cmd()).metadata["binding_digest"]
    b = env.client.command_authorization(**_cmd(operation="publish")).metadata["binding_digest"]
    c = env.client.command_authorization(**_cmd(target=proposal_target(candidate_hash="b" * 64))).metadata[
        "binding_digest"
    ]
    assert len({a, b, c}) == 3


def test_revoke_needs_admin_and_carries_admin_role(env: Env) -> None:
    with pytest.raises(IssuerError) as err:
        env.client.command_authorization(**_cmd(operation="revoke", target=release_target()))
    assert err.value.code == "pulso:role_not_allowed" and err.value.status == 403
    out = env.client.command_authorization(**_cmd(actor_ref=ADMIN, operation="revoke", target=release_target()))
    assert "admin" in _decode(out.authorization_jws)[1]["roles"]
    plain = env.client.command_authorization(**_cmd(actor_ref=ADMIN))
    assert "admin" not in _decode(plain.authorization_jws)[1]["roles"]  # least authority per operation


def test_unknown_actor_and_bot_shaped_actor_denied(env: Env) -> None:
    for actor in ("pulso-constructor:tenant-a", "nobody"):
        with pytest.raises(IssuerError) as err:
            env.client.command_authorization(**_cmd(actor_ref=actor))
        assert err.value.code == "pulso:actor_not_allowed" and err.value.status == 403


def test_builder_human_without_aprobador_cannot_approve(env: Env) -> None:
    with pytest.raises(IssuerError) as err:
        env.client.command_authorization(**_cmd(actor_ref="local-builder-human"))
    assert err.value.code == "pulso:role_not_allowed"


def test_cross_tenant_denied_by_claim_and_by_identity(env: Env) -> None:
    with pytest.raises(IssuerError) as err:
        env.make_client(tenant_override="tenant-b").command_authorization(**_cmd())  # claim != body
    assert err.value.code == "pulso:tenant_mismatch"
    with pytest.raises(IssuerError) as err2:
        env.make_client(tenant_override="tenant-b").command_authorization(**_cmd(tenant_id="tenant-b"))
    assert err2.value.code == "pulso:tenant_mismatch"  # identity belongs to tenant-a


def test_operation_target_mismatch_and_bad_shapes(env: Env) -> None:
    for over in (
        {"operation": "approve", "target": release_target()},
        {"operation": "promote", "target": proposal_target()},
        {"operation": "delete"},
        {"target": proposal_target(candidate_hash="XYZ")},
        {"target": proposal_target(expected_revision=-1)},
        {"target": {**proposal_target(), "extra": 1}},
    ):
        with pytest.raises(IssuerError) as err:
            env.client.command_authorization(**_cmd(**over))
        assert err.value.status == 422, over


def test_unknown_request_fields_rejected(env: Env) -> None:
    token = env.client.mint("command_authorization")
    r = env.http.post(CMD_URL, headers={"Authorization": f"Bearer {token}"}, json={**_cmd(), "role": "admin"})
    assert r.status_code == 422 and r.json()["code"] == "pulso:invalid_request"


def test_nonce_replay_rejected(env: Env) -> None:
    args = _cmd()
    env.client.command_authorization(**args)
    with pytest.raises(IssuerError) as err:
        env.client.command_authorization(**args)  # fresh jti, same nonce
    assert err.value.code == "pulso:nonce_replayed" and err.value.status == 409


def test_service_token_replay_rejected_and_no_issue_on_replay(env: Env) -> None:
    token = env.client.mint("command_authorization")
    first = env.http.post(CMD_URL, headers={"Authorization": f"Bearer {token}"}, json=_cmd())
    assert first.status_code == 200
    again = env.http.post(CMD_URL, headers={"Authorization": f"Bearer {token}"}, json=_cmd())
    assert again.status_code == 401 and again.json()["code"] == "pulso:service_token_replayed"
    assert "authorization_jws" not in again.text


def test_session_assertion_claims(env: Env) -> None:
    out = env.client.session_assertion(
        tenant_id=TENANT, actor_ref=SUPERVISOR, session_intent_ref="intent-1", nonce="n" * 16
    )
    header, claims = _decode(out.assertion)
    assert header == {"alg": "EdDSA", "kid": out.kid, "typ": "JWT"} and out.kid.startswith("local-sim-session-")
    assert claims["iss"] == "human-issuer" and claims["aud"] == "control-api"
    assert claims["sub"] == SUPERVISOR and claims["tenant_id"] == TENANT
    assert claims["nonce"] == "n" * 16 and claims["session_intent_ref"] == "intent-1"
    assert claims["auth_level"] == "session" and claims["auth_at"] == claims["iat"]
    assert claims["exp"] - claims["iat"] <= 60 and isinstance(claims["jti"], str)
    assert out.exp == claims["exp"]
    with pytest.raises(IssuerError) as err:
        env.client.session_assertion(
            tenant_id=TENANT, actor_ref=SUPERVISOR, session_intent_ref="intent-1", nonce="n" * 16
        )
    assert err.value.code == "pulso:nonce_replayed"


def test_session_assertion_unknown_actor(env: Env) -> None:
    with pytest.raises(IssuerError) as err:
        env.client.session_assertion(tenant_id=TENANT, actor_ref="nobody", session_intent_ref="i", nonce="n" * 16)
    assert err.value.code == "pulso:actor_not_allowed"


def test_wrong_purpose_per_route(env: Env) -> None:
    tok = env.client.mint("session_assertion")
    r = env.http.post(CMD_URL, headers={"Authorization": f"Bearer {tok}"}, json=_cmd())
    assert r.status_code == 403 and r.json()["details"]["reason"] == "purpose_denied"
    tok2 = env.client.mint("command_authorization")
    r2 = env.http.post(
        SESSION_URL,
        headers={"Authorization": f"Bearer {tok2}"},
        json={"tenant_id": TENANT, "actor_ref": SUPERVISOR, "session_intent_ref": "i", "nonce": "n" * 16},
    )
    assert r2.status_code == 403


def test_missing_and_garbage_bearer(env: Env) -> None:
    assert env.http.post(CMD_URL, json=_cmd()).status_code == 401
    assert env.http.post(CMD_URL, headers={"Authorization": "Bearer nope"}, json=_cmd()).status_code == 401


def test_bot_style_principal_jws_cannot_call_issuer(env: Env) -> None:
    out = env.client.command_authorization(**_cmd())
    r = env.http.post(CMD_URL, headers={"Authorization": f"Bearer {out.authorization_jws}"}, json=_cmd())
    assert r.status_code == 401 and r.json()["details"]["reason"] == "bad_header"  # typ=principal+jws is no JWT


def test_other_service_keys_denied(env: Env) -> None:
    rogue = ServiceSigner.generate("rogue-1", issuer="control-api", audience="human-issuer")
    with pytest.raises(IssuerError) as err:
        env.make_client(signer=rogue).command_authorization(**_cmd())
    assert err.value.details["reason"] == "unknown_kid"
    real = ServiceSigner.from_file(env.dir / "control-api-signer.json")
    for iss, aud in (("core-bridge", "human-issuer"), ("control-api", "core-bridge")):
        bad = ServiceSigner(real.kid, real.private_key, issuer=iss, audience=aud)
        with pytest.raises(IssuerError) as err2:
            env.make_client(signer=bad).command_authorization(**_cmd())
        assert err2.value.status == 401, (iss, aud)


def test_ttl_and_skew_boundaries(env: Env) -> None:
    now_ts = int(env.clock.now.timestamp())
    sign = env.client.signer.sign_claims

    def post(**claims: object) -> int:
        base = {
            "iss": "control-api",
            "aud": "human-issuer",
            "sub": "worker",
            "tenant_id": TENANT,
            "purpose": "command_authorization",
            "iat": now_ts,
            "exp": now_ts + 60,
            "jti": uuid.uuid4().hex,
        }
        base.update(claims)
        return env.http.post(CMD_URL, headers={"Authorization": f"Bearer {sign(base)}"}, json=_cmd()).status_code

    assert post() == 200
    assert post(exp=now_ts + 61) == 401  # TTL > 60 s
    assert post(iat=now_ts - 30, exp=now_ts + 30) == 200
    assert post(iat=now_ts - 100, exp=now_ts - 59) == 200  # now < exp + skew(60)
    assert post(iat=now_ts - 120, exp=now_ts - 60) == 401  # now >= exp + skew
    assert post(iat=now_ts + 60, exp=now_ts + 100) == 200  # iat <= now + skew
    assert post(iat=now_ts + 61, exp=now_ts + 100) == 401


def test_replay_store_is_durable_across_restarts(tmp_path: Path, env: Env) -> None:
    db = tmp_path / "replay.sqlite3"
    app1 = build_app(load_config(env_for(env.dir, LOCAL_IDENTITY_REPLAY_DB=str(db))), now=env.clock)
    token = env.client.mint("command_authorization")
    h = {"Authorization": f"Bearer {token}"}
    assert TestClient(app1).post(CMD_URL, headers=h, json=_cmd()).status_code == 200
    app2 = build_app(load_config(env_for(env.dir, LOCAL_IDENTITY_REPLAY_DB=str(db))), now=env.clock)
    assert TestClient(app2).post(CMD_URL, headers=h, json=_cmd()).status_code == 401


def test_replay_rows_pruned_after_expiry_plus_skew(env: Env) -> None:
    token = env.client.mint("command_authorization")
    h = {"Authorization": f"Bearer {token}"}
    assert env.http.post(CMD_URL, headers=h, json=_cmd()).status_code == 200
    env.clock.now += timedelta(seconds=121)
    # expired beyond skew: rejected as expired (not as replay), and the row is pruned
    assert env.http.post(CMD_URL, headers=h, json=_cmd()).json()["details"]["reason"] == "expired"


def test_error_envelope_and_health(env: Env) -> None:
    r = env.http.post(CMD_URL, json={})
    assert set(r.json()) == {"schema_version", "code", "retryable", "trace_id", "details"}
    assert env.http.get("/healthz").status_code == 200 and env.http.get("/readyz").json() == {"status": "ready"}
    assert env.http.get("/openapi.json").status_code == 404 and env.http.get("/docs").status_code == 404


def test_no_secret_in_logs_or_repr(env: Env, caplog: pytest.LogCaptureFixture) -> None:
    out = env.client.command_authorization(**_cmd())
    assert out.authorization_jws not in repr(out) and out.authorization_jws not in caplog.text
    seed = json.loads((env.dir / "human-staff-signer.json").read_text())["key"]
    assert seed not in repr(env.config) and seed not in repr(env.client)


def test_assert_bound_accepts_exact_binding_and_rejects_any_other(env: Env) -> None:
    from local_identity.client import BindingMismatch, assert_bound

    args = _cmd()
    out = env.client.command_authorization(**args)
    bound = {k: v for k, v in args.items() if k != "nonce"}
    assert_bound(out.authorization_jws, **bound)
    for over in (
        {"operation": "publish"},
        {"command_ref": "cmd-2"},
        {"challenge_ref": "chal-2"},
        {"actor_ref": ADMIN},
        {"tenant_id": "tenant-b"},
        {"target": proposal_target(candidate_hash="b" * 64)},
        {"target": proposal_target(expected_revision=4)},
        {"operation": "promote", "target": release_target()},
    ):
        with pytest.raises(BindingMismatch):
            assert_bound(out.authorization_jws, **{**bound, **over})

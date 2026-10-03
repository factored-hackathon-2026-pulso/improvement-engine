"""FIRST RED of e2e-core (codex-standin): the Python stand-in of the Rust engine must sign A03 service JWTs the
runtime accepts, compute the annex-D idempotency key / request digest exactly like core-bridge, and host the
control-api + lab-broker doubles with exactly-once binding, tenant isolation and jti replay protection."""

from __future__ import annotations

import hashlib
import time

import pytest
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from fastapi.testclient import TestClient

from codex_standin.dto import idempotency_key, request_digest
from codex_standin.fixtures_app import World, create_app
from codex_standin.jwtsvc import Denied, KeyRing, Verifier, b64u, sign


def pub(key: Ed25519PrivateKey) -> str:
    from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat
    return b64u(key.public_key().public_bytes(Encoding.Raw, PublicFormat.Raw))


def test_idempotency_key_and_request_digest_follow_annex_d() -> None:
    assert idempotency_key("t1", "j1", "scout", 1, "k") == hashlib.sha256(b"t1|j1|scout|1|k").hexdigest()
    a = {"b": 1, "a": [1, 2], "request_digest": "x", "credentials": {"z": 1}, "trace": {"t": 1}}
    b = {"a": [1, 2], "b": 1}
    assert request_digest(a) == request_digest(b)  # excluded keys never enter the digest; JCS key order
    assert request_digest({"a": 1}) != request_digest({"a": 2})


def test_service_jwt_roundtrip_and_closed_rejections() -> None:
    key = Ed25519PrivateKey.generate()
    ring = KeyRing({"k1": ("core-bridge", "control-api", pub(key))})
    verifier = Verifier(ring)
    now = int(time.time())
    claims = {"iss": "core-bridge", "aud": "control-api", "sub": "bridge:1", "tenant_id": "t1", "scope": "binding",
              "iat": now, "exp": now + 60, "jti": "j-1"}
    tok = sign(key, "k1", claims)
    assert verifier.verify(tok, aud="control-api", scope="binding")["tenant_id"] == "t1"
    with pytest.raises(Denied) as replay:
        verifier.verify(tok, aud="control-api", scope="binding")
    assert replay.value.reason == "jti_replayed"
    with pytest.raises(Denied) as aud:
        verifier.verify(sign(key, "k1", {**claims, "jti": "j-2"}), aud="lab-broker", scope="binding")
    assert aud.value.reason == "wrong_audience"
    with pytest.raises(Denied) as scope:
        verifier.verify(sign(key, "k1", {**claims, "jti": "j-3"}), aud="control-api", scope="lab")
    assert scope.value.reason == "scope_denied"
    with pytest.raises(Denied) as exp:
        verifier.verify(sign(key, "k1", {**claims, "jti": "j-4", "exp": now - 5}), aud="control-api", scope="binding")
    assert exp.value.reason == "expired"
    with pytest.raises(Denied) as sig:
        verifier.verify(sign(Ed25519PrivateKey.generate(), "k1", {**claims, "jti": "j-5"}), aud="control-api",
                        scope="binding")
    assert sig.value.reason == "bad_signature"


def _world() -> tuple[TestClient, World, Ed25519PrivateKey, Ed25519PrivateKey]:
    cb, ex = Ed25519PrivateKey.generate(), Ed25519PrivateKey.generate()
    world = World(KeyRing({"cb": ("core-bridge", "control-api", pub(cb)), "ex": ("core-bridge", "lab-broker", pub(ex))}))
    return TestClient(create_app(world)), world, cb, ex


def _tok(key: Ed25519PrivateKey, kid: str, aud: str, scope: str, tenant: str, **extra: object) -> dict[str, str]:
    now = int(time.time())
    claims = {"iss": "core-bridge", "aud": aud, "sub": "bridge:1", "tenant_id": tenant, "scope": scope,
              "purpose": extra.pop("purpose", scope), "iat": now, "exp": now + 60,
              "jti": __import__("uuid").uuid4().hex, **extra}
    return {"Authorization": "Bearer " + sign(key, kid, claims)}


def _bind(tenant: str = "t1", job: str = "j1", cmd: str = "c1", digest: str = "d" * 64) -> dict[str, object]:
    return {"schema_version": "1", "tenant_id": tenant, "job_id": job, "command_key": cmd, "request_digest": digest,
            "attempt": 1, "core_run_id": "run-1", "bridge_instance_id": "b1", "task_binding_ref": f"bind-{cmd}"}


def test_binding_is_exactly_once_and_conflicts_are_409() -> None:
    client, world, cb, _ = _world()
    url = "/internal/v1/core-task-bindings"
    h = lambda t="t1": {**_tok(cb, "cb", "control-api", "binding", t, purpose="core_task_binding"), "Idempotency-Key": "c1"}  # noqa: E731
    assert client.post(url, json=_bind(), headers=h()).status_code == 200
    assert client.post(url, json=_bind(), headers=h()).status_code == 200  # replay: same effect
    assert world.binding_effects[("t1", "j1")] == 1
    other = {**_tok(cb, "cb", "control-api", "binding", "t1", purpose="core_task_binding"), "Idempotency-Key": "c1"}
    assert client.post(url, json=_bind(digest="e" * 64), headers=other).json()["code"] == "digest_mismatch"
    again = {**_tok(cb, "cb", "control-api", "binding", "t1", purpose="core_task_binding"), "Idempotency-Key": "c2"}
    assert client.post(url, json=_bind(cmd="c2"), headers=again).json()["code"] == "binding_conflict"
    assert world.binding_effects[("t1", "j1")] == 1


def test_tenant_claim_must_match_the_body_and_artifacts_never_cross_tenants() -> None:
    client, world, cb, ex = _world()
    wrong = {**_tok(cb, "cb", "control-api", "binding", "t2", purpose="core_task_binding"), "Idempotency-Key": "c1"}
    assert client.post("/internal/v1/core-task-bindings", json=_bind(), headers=wrong).status_code == 403
    world.put_artifact("t1", "art-1", {"x": 1})
    h = lambda t: _tok(ex, "ex", "lab-broker", "artifact_read", t, purpose="artifact_read")  # noqa: E731
    assert client.get("/internal/v1/broker/artifacts/art-1", headers=h("t1")).status_code == 200
    assert client.get("/internal/v1/broker/artifacts/art-1", headers=h("t2")).status_code == 404
    assert world.cross_tenant_denials == 1


def test_authorization_check_is_binding_scoped_and_denials_are_configurable() -> None:
    client, world, cb, ex = _world()
    client.post("/internal/v1/core-task-bindings", json=_bind(), headers={
        **_tok(cb, "cb", "control-api", "binding", "t1", purpose="core_task_binding"), "Idempotency-Key": "c1"})
    body = {"binding_ref": "bind-c1", "operation": "registry/freeze", "resource_refs": [], "payload_digest": "a" * 64}
    ok = client.post("/internal/v1/broker/authorizations/check", json=body, headers=_tok(
        ex, "ex", "lab-broker", "authorization_check", "t1", purpose="authorization_check", binding_ref="bind-c1"))
    assert ok.json()["allowed"] is True
    unknown = client.post("/internal/v1/broker/authorizations/check", json={**body, "binding_ref": "nope"},
                          headers=_tok(ex, "ex", "lab-broker", "authorization_check", "t1",
                                       purpose="authorization_check"))
    assert unknown.json()["allowed"] is False
    world.deny_operations.add("registry/freeze")
    denied = client.post("/internal/v1/broker/authorizations/check", json=body, headers=_tok(
        ex, "ex", "lab-broker", "authorization_check", "t1", purpose="authorization_check"))
    assert denied.json()["allowed"] is False and denied.json()["reason_code"] == "revoked"

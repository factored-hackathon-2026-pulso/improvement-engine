"""A03 token gaps: executor->lab-broker JWTs use the separate executor keypair (not the callback key) and every
older-tools broker call carries tenant_id + purpose + singular scope with a fresh jti per attempt."""

from __future__ import annotations

import json
from datetime import UTC, datetime, timedelta
from pathlib import Path
from typing import Any

import httpx
import pytest
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey

from pulso_core_runtime.internal.auth import b64url_decode, b64url_encode
from pulso_core_runtime.invoke.wiring import build_l3, install_tools, lab_broker_minter
from pulso_core_runtime.tools.context import InvocationContext


def _write(path: Path, kid: str) -> Ed25519PrivateKey:
    key = Ed25519PrivateKey.generate()
    path.write_text(json.dumps({"kid": kid, "key": b64url_encode(key.private_bytes_raw())}))
    return key


def _env(tmp: Path) -> tuple[dict[str, str], dict[str, Ed25519PrivateKey]]:
    keys = {n: _write(tmp / f"{n}.json", f"{n}-kid") for n in ("i", "s", "c", "e")}
    return ({"PULSO_BRIDGE_IDENTITY_SIGNER": str(tmp / "i.json"), "PULSO_BRIDGE_STAFF_SIGNER": str(tmp / "s.json"),
             "PULSO_BRIDGE_CALLBACK_SIGNER": str(tmp / "c.json"), "PULSO_BRIDGE_EXECUTOR_SIGNER": str(tmp / "e.json"),
             "PULSO_CONTROL_API_URL": "http://control.test", "PULSO_LAB_BROKER_URL": "http://broker.test"}, keys)


def _l3(env: dict[str, str]) -> Any:
    return build_l3(env, dsn="postgresql://x:y@127.0.0.1:1/none", registry=object(), app_getter=lambda: None,
                    migrate=False)


def _decode(jws: str) -> tuple[dict[str, Any], dict[str, Any], bytes, bytes]:
    h, b, s = jws.split(".")
    return (json.loads(b64url_decode(h)), json.loads(b64url_decode(b)), f"{h}.{b}".encode(), b64url_decode(s))


def _verifies(jws: str, key: Ed25519PrivateKey) -> bool:
    from cryptography.exceptions import InvalidSignature
    _, _, signed, sig = _decode(jws)
    try:
        key.public_key().verify(sig, signed)
        return True
    except InvalidSignature:
        return False


def test_missing_executor_signer_fails_closed_naming_the_file(tmp_path: Path) -> None:
    env, _ = _env(tmp_path)
    (tmp_path / "e.json").unlink()
    with pytest.raises(ValueError, match="e.json"):
        _l3(env)


def test_lab_broker_minter_uses_executor_key_not_callback_key(tmp_path: Path) -> None:
    env, keys = _env(tmp_path)
    l3 = _l3(env)
    jws = lab_broker_minter(l3, env)({"scope": "sandbox", "purpose": "sandbox_open", "tenant_id": "t1",
                                      "iss": "evil", "aud": "control-api", "jti": "fixed", "exp": 9999999999})
    head, body, _, _ = _decode(jws)
    assert head["kid"] == "e-kid" and _verifies(jws, keys["e"]) and not _verifies(jws, keys["c"])
    assert body["aud"] == "lab-broker" and body["iss"] == "core-bridge" and body["jti"] != "fixed"
    assert body["exp"] - body["iat"] <= 60


def test_executor_key_must_differ_from_callback_key(tmp_path: Path) -> None:
    env, keys = _env(tmp_path)
    (tmp_path / "e.json").write_text((tmp_path / "c.json").read_text())
    with pytest.raises(ValueError, match="distinct"):
        _l3(env)


def _ic(tenant: str = "t1") -> InvocationContext:
    return InvocationContext(
        tenant_id=tenant, job_id="job-1", stage="scout", attempt=1, binding_ref="bind-1", command_key="c",
        request_digest="r" * 64, bridge_instance_id="b", expires_at=datetime.now(UTC) + timedelta(minutes=5),
        memory_snapshot_ref="mem-1", extract_manifest_ref="ext-1")


def test_tools_broker_calls_carry_tenant_purpose_singular_scope_and_fresh_jti(tmp_path: Path) -> None:
    env, keys = _env(tmp_path)
    l3 = _l3(env)
    l3.registry.register(_ic())
    seen: list[tuple[str, dict[str, Any], bool]] = []

    def handle(req: httpx.Request) -> httpx.Response:
        jws = req.headers["authorization"].removeprefix("Bearer ")
        seen.append((req.url.path, _decode(jws)[1], _verifies(jws, keys["e"])))
        if req.url.path.endswith("/authorizations/check"):
            return httpx.Response(200, json={"allowed": True, "authorization_revision": 1})
        return httpx.Response(200, json={"session_ref": "s1"})

    runtime = install_tools(l3, env)
    runtime.broker._http = httpx.Client(transport=httpx.MockTransport(handle))
    runtime.broker.authorization_check("bind-1", "lab_query", [], None)
    runtime.broker.authorization_check("bind-1", "lab_query", [], None)
    runtime.broker.lab_open_session("bind-1", "ext-1")
    runtime.broker.artifact_get("bind-1", "art-1")
    assert [p for p, _, _ in seen][-2:] == ["/internal/v1/broker/lab/sessions", "/internal/v1/broker/artifacts/art-1"]
    for _, claims, ok in seen:
        assert ok and claims["aud"] == "lab-broker" and claims["iss"] == "core-bridge"
        assert claims["tenant_id"] == "t1" and claims["job_id"] == "job-1" and claims["binding_ref"] == "bind-1"
        assert isinstance(claims["scope"], str) and " " not in claims["scope"] and claims["purpose"]
        assert claims["exp"] - claims["iat"] <= 60
    assert len({c["jti"] for _, c, _ in seen}) == 4
    assert [(c["scope"], c["purpose"]) for _, c, _ in seen] == [
        ("authorization_check", "authorization_check")] * 2 + [("lab", "lab_session"), ("artifact_read", "artifact_read")]


def test_unknown_binding_is_a_denial_not_an_unscoped_token(tmp_path: Path) -> None:
    from pulso_core_runtime.tools.broker import BrokerError
    env, _ = _env(tmp_path)
    runtime = install_tools(_l3(env), env)
    runtime.broker._http = httpx.Client(transport=httpx.MockTransport(lambda r: pytest.fail("no request expected")))
    with pytest.raises(BrokerError):
        runtime.broker.lab_open_session("ghost", "ext-1")


def test_control_api_binding_token_uses_callback_key_and_body_tenant(tmp_path: Path) -> None:
    env, keys = _env(tmp_path)
    runtime = install_tools(_l3(env), env)
    seen: list[tuple[dict[str, Any], bool, bool]] = []

    def handle(req: httpx.Request) -> httpx.Response:
        jws = req.headers["authorization"].removeprefix("Bearer ")
        seen.append((_decode(jws)[1], _verifies(jws, keys["c"]), _verifies(jws, keys["e"])))
        return httpx.Response(200, json={})

    runtime.control._http = httpx.Client(transport=httpx.MockTransport(handle))
    runtime.control.bind({"tenant_id": "t1", "binding_ref": "b"}, "idem-1")
    claims, by_cb, by_ex = seen[0]
    assert by_cb and not by_ex and claims["aud"] == "control-api" and claims["scope"] == "binding"
    assert claims["tenant_id"] == "t1" and claims["purpose"] == "core_task_binding"

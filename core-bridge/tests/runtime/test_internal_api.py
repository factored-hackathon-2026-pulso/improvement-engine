"""/internal/v1 sub-app: service-JWT auth (per-route audience/purpose, jti replay), error envelope, version."""

from __future__ import annotations

import time
import uuid
from typing import Any

import pytest
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from fastapi.testclient import TestClient

from pulso_core_runtime.compat import PIN_SYMBOLS, PinDrift, assert_compat
from pulso_core_runtime.internal.app import build_internal_app
from pulso_core_runtime.internal.auth import (
    InMemoryJtiStore,
    ServiceJwtVerifier,
    ServiceKey,
    b64url_encode,
    sign_service_jwt,
)

pytestmark = pytest.mark.runtime

CP = Ed25519PrivateKey.generate()  # control-api -> core-bridge
LB = Ed25519PrivateKey.generate()  # core-bridge -> lab-broker (wrong audience for our routes)
INFO = {"agent_core_sha": "x", "contracts_version": "1.3.0", "pulso_sha": "p", "image_digest": "d",
        "runtime_profile": "agent_core_real", "doubles": ["tools: stand-in"]}


def _client() -> TestClient:
    keys = {"cp1": ServiceKey("control-api", "core-bridge", CP.public_key()),
            "lb1": ServiceKey("core-bridge", "lab-broker", LB.public_key())}
    app = build_internal_app(ServiceJwtVerifier(keys, InMemoryJtiStore()), version_info=lambda: INFO)
    return TestClient(app, raise_server_exceptions=False)


def _token(purpose: str = "version_probe", *, key: Any = CP, kid: str = "cp1", aud: str = "core-bridge",
           iss: str = "control-api", exp_in: int = 60, jti: str | None = None, **extra: Any) -> str:
    now = int(time.time())
    claims = {"iss": iss, "aud": aud, "sub": "worker:1", "tenant_id": "t1", "purpose": purpose, "job_id": "j1",
              "iat": now, "exp": now + exp_in, "jti": jti or uuid.uuid4().hex, **extra}
    return sign_service_jwt(key, kid=kid, claims=claims)


def _get(c: TestClient, token: str | None, path: str = "/version") -> Any:
    return c.get(path, headers={} if token is None else {"Authorization": f"Bearer {token}"})


def test_version_authenticated_and_shape() -> None:
    r = _get(_client(), _token())
    assert r.status_code == 200 and r.json() == INFO


def test_envelope_on_unauthenticated() -> None:
    r = _get(_client(), None)
    body = r.json()
    assert r.status_code == 401 and set(body) == {"schema_version", "code", "retryable", "trace_id", "details"}
    assert body["code"] == "pulso:auth_invalid"


def test_unknown_route_uses_envelope_not_problem_json() -> None:
    r = _get(_client(), _token(), "/nope")
    assert r.status_code == 404 and r.json()["code"] == "pulso:not_found"
    assert "problem+json" not in r.headers["content-type"]


def test_replayed_jti_rejected() -> None:
    c, t = _client(), _token(jti="same")
    assert _get(c, t).status_code == 200
    r = _get(c, t)
    assert r.status_code == 401 and r.json()["details"]["reason"] == "jti_replayed"


def test_wrong_audience_key_rejected() -> None:
    # A lab-broker token (own key, own aud) never opens a core-bridge route.
    t = _token(key=LB, kid="lb1", aud="lab-broker", iss="core-bridge")
    assert _get(_client(), t).json()["details"]["reason"] == "wrong_audience"


def test_kid_bound_to_issuer_and_audience() -> None:
    t = _token(iss="core-bridge")  # signed with cp1 but claims another issuer
    assert _get(_client(), t).json()["details"]["reason"] == "key_binding"


def test_wrong_purpose_forbidden() -> None:
    r = _get(_client(), _token("core_task_read"))
    assert r.status_code == 403 and r.json()["code"] == "pulso:auth_denied"


def test_expired_and_overlong_ttl() -> None:
    c = _client()
    assert _get(c, _token(exp_in=-5)).json()["details"]["reason"] == "expired"
    assert _get(c, _token(exp_in=900)).json()["details"]["reason"] == "ttl_too_long"


def test_bad_signature_header_and_alg() -> None:
    c = _client()
    good = _token()
    h, p, s = good.split(".")
    assert _get(c, f"{h}.{p}.{b64url_encode(b'x' * 64)}").json()["details"]["reason"] == "bad_signature"
    none_alg = b64url_encode(b'{"alg":"none","kid":"cp1","typ":"JWT"}')
    assert _get(c, f"{none_alg}.{p}.{s}").json()["details"]["reason"] == "bad_header"
    core_jws = b64url_encode(b'{"alg":"EdDSA","kid":"cp1","typ":"principal+jws"}')
    assert _get(c, f"{core_jws}.{p}.{s}").json()["details"]["reason"] == "bad_header"


def test_rejected_token_does_not_burn_jti() -> None:
    c = _client()
    assert _get(c, _token("core_task_read", jti="j")).status_code == 403
    assert _get(c, _token("version_probe", jti="j")).status_code == 200


def test_unimplemented_route_is_authenticated_501() -> None:
    c = _client()
    assert c.post("/core-tasks/invoke").status_code == 401
    r = c.post("/core-tasks/invoke", headers={"Authorization": f"Bearer {_token('core_task_invoke')}"})
    assert r.status_code == 501 and r.json()["code"] == "pulso:not_implemented"


def test_compat_passes_on_pin_and_detects_drift() -> None:
    assert_compat()
    drifted = (*PIN_SYMBOLS, ("agent_core.composition.serve_ports", "resolve_ports", "func", ("no_such_param",)))
    with pytest.raises(PinDrift):
        assert_compat(drifted)
    with pytest.raises(PinDrift):
        assert_compat((("agent_core.api.app", "does_not_exist", "func", ()),))

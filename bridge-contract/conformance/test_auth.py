"""Auth conformance (A02/A03 class i): fixed EdDSA `typ=JWT`, exact header, kid bound to (iss, aud), per-route purpose,
tenant claim, exp <= 5 min, receiver-owned jti replay (consumed last), tenant set, error envelope on every denial."""

from __future__ import annotations

import json
import re
import uuid

import pytest
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey

from conformance.conftest import assert_error, assert_valid
from conformance.kit import CONTRACT, DROP, Signer, b64u
from conformance.worlds import World

ROUTES = [r for r in CONTRACT["routes"] if "x-status" not in r]
AUTH = CONTRACT["auth"]
REASONS = set(AUTH["reasons"])


def route_path(r: dict) -> str:
    return re.sub(r"{\w+}", "probe", r["path"])


def ids(r: dict) -> str:
    return f"{r['method']} {r['path']}"


def send(world: World, r: dict, *, token: str | None, headers: dict | None = None):  # type: ignore[no-untyped-def]
    h = dict(headers or {})
    if token is not None:
        h["Authorization"] = f"Bearer {token}"
    body = {} if r["method"] == "POST" else None
    return world.api.call(r["method"], route_path(r), body=body, headers=h)


def purpose_of(r: dict) -> str:
    return r["purposes"][0]


def tok(world: World, r: dict, **kw):  # type: ignore[no-untyped-def]
    tenant = kw.pop("tenant", world.tenant)
    return world.signer.token(purpose_of(r), tenant, **kw)


@pytest.mark.parametrize("r", ROUTES, ids=ids)
def test_missing_token_is_401_auth_invalid(world: World, r: dict) -> None:
    resp = send(world, r, token=None)
    body = assert_error(resp, 401, "pulso:auth_invalid", retryable=False)
    assert body["details"]["reason"] == "missing_token"


@pytest.mark.parametrize("bad", ["abc", "a.b", "a.b.c.d", "!!!.???.***"])
def test_malformed_token_is_401_with_a_closed_reason(world: World, bad: str) -> None:
    resp = world.api.call("GET", "/version", token=bad)
    body = assert_error(resp, 401, "pulso:auth_invalid")
    assert body["details"]["reason"] in REASONS


def _signed(world: World, header: dict, claims: dict | None = None, key: Ed25519PrivateKey | None = None) -> str:
    claims = claims or world.signer.claims("version_probe", world.tenant)
    return world.signer.sign(claims, header=header, key=key)


@pytest.mark.parametrize("header", [
    {"alg": "none", "kid": "KID", "typ": "JWT"}, {"alg": "HS256", "kid": "KID", "typ": "JWT"},
    {"alg": "EdDSA", "kid": "KID", "typ": "principal+jws"}, {"alg": "EdDSA", "kid": "KID", "typ": "JWT", "x": 1},
    {"alg": "EdDSA", "kid": "KID"}], ids=["alg-none", "alg-hs256", "typ-principal", "extra-header-key", "no-typ"])
def test_header_must_be_exactly_alg_kid_typ_with_fixed_algorithm(world: World, header: dict) -> None:
    header = {k: (world.signer.kid if v == "KID" else v) for k, v in header.items()}
    resp = world.api.call("GET", "/version", token=_signed(world, header))
    body = assert_error(resp, 401, "pulso:auth_invalid")
    assert body["details"]["reason"] == "bad_header"


def test_unknown_kid_is_refused(world: World) -> None:
    resp = world.api.call("GET", "/version", token=_signed(world, {"alg": "EdDSA", "kid": "no-such-kid", "typ": "JWT"}))
    assert assert_error(resp, 401, "pulso:auth_invalid")["details"]["reason"] == "unknown_kid"


def test_signature_from_another_key_is_refused(world: World) -> None:
    header = {"alg": "EdDSA", "kid": world.signer.kid, "typ": "JWT"}
    resp = world.api.call("GET", "/version", token=_signed(world, header, key=Ed25519PrivateKey.generate()))
    assert assert_error(resp, 401, "pulso:auth_invalid")["details"]["reason"] == "bad_signature"


@pytest.mark.parametrize("over", [{"aud": "lab-broker"}, {"aud": "control-api"}, {"iss": "core-bridge"}],
                         ids=["aud-lab-broker", "aud-control-api", "iss-core-bridge"])
def test_kid_is_bound_to_one_issuer_and_audience(world: World, over: dict) -> None:
    resp = world.api.call("GET", "/version", token=world.signer.token("version_probe", world.tenant, **over))
    body = assert_error(resp, 401, "pulso:auth_invalid")
    assert body["details"]["reason"] in {"key_binding", "wrong_audience"}


@pytest.mark.parametrize("over,reason", [
    ({"ttl": -5}, "expired"), ({"ttl": AUTH["token"]["max_ttl_seconds"] + 1}, "ttl_too_long"),
    ({"jti": DROP}, "missing_claims"), ({"exp": DROP}, "missing_claims"), ({"iat": DROP}, "missing_claims"),
    ({"sub": DROP}, "missing_claims"), ({"jti": ""}, "missing_claims")],
    ids=["expired", "ttl-301s", "no-jti", "no-exp", "no-iat", "no-sub", "empty-jti"])
def test_time_and_required_claims(world: World, over: dict, reason: str) -> None:
    resp = world.api.call("GET", "/version", token=world.signer.token("version_probe", world.tenant, **over))
    assert assert_error(resp, 401, "pulso:auth_invalid")["details"]["reason"] == reason


def test_nan_timestamps_cannot_defeat_the_comparisons(world: World) -> None:
    claims = world.signer.claims("version_probe", world.tenant)
    head = b64u(json.dumps({"alg": "EdDSA", "kid": world.signer.kid, "typ": "JWT"}, separators=(",", ":")).encode())
    body = b64u(json.dumps({**claims, "exp": float("nan"), "iat": float("nan")}, separators=(",", ":")).encode())
    sig = b64u(world.signer._key.sign(f"{head}.{body}".encode()))
    resp = world.api.call("GET", "/version", token=f"{head}.{body}.{sig}")
    assert assert_error(resp, 401, "pulso:auth_invalid")["details"]["reason"] == "missing_claims"


@pytest.mark.parametrize("r", ROUTES, ids=ids)
def test_wrong_purpose_is_403_auth_denied(world: World, r: dict) -> None:
    other = next(p for rr in ROUTES for p in rr["purposes"] if p not in r["purposes"])
    resp = send(world, r, token=world.signer.token(other, world.tenant))
    body = assert_error(resp, 403, "pulso:auth_denied", retryable=False)
    assert body["details"]["reason"] == "purpose_denied"


@pytest.mark.parametrize("r", [r for r in ROUTES if r["tenant_required"]], ids=ids)
def test_tenant_claim_is_required_on_every_tenant_route(world: World, r: dict) -> None:
    resp = send(world, r, token=tok(world, r, tenant=None))
    assert assert_error(resp, 403, "pulso:auth_denied")["details"]["reason"] == "tenant_required"


@pytest.mark.parametrize("r", [r for r in ROUTES if r["tenant_required"]], ids=ids)
def test_tenant_outside_the_deployment_set_is_403_tenant_mismatch(world: World, r: dict) -> None:
    resp = send(world, r, token=tok(world, r, tenant=world.unknown_tenant))
    assert_error(resp, 403, "pulso:tenant_mismatch", retryable=False)


@pytest.mark.parametrize("r", [r for r in ROUTES if not r["tenant_required"]], ids=ids)
def test_tenant_exempt_routes_accept_a_token_without_tenant(world: World, r: dict) -> None:
    resp = send(world, r, token=tok(world, r, tenant=None))
    assert resp.status_code == 200, resp.text


def test_jti_replay_is_refused_and_a_fresh_jti_is_accepted(world: World) -> None:
    token = world.signer.token("version_probe", world.tenant)
    assert world.api.call("GET", "/version", token=token).status_code == 200
    again = world.api.call("GET", "/version", token=token)
    assert assert_error(again, 401, "pulso:auth_invalid")["details"]["reason"] == "jti_replayed"
    assert world.api.call("GET", "/version", token=world.signer.token("version_probe", world.tenant)).status_code == 200


def test_a_rejected_token_does_not_burn_its_jti(world: World) -> None:
    """Consume last: a token refused for its purpose is still unused for the route it WAS minted for."""
    jti = uuid.uuid4().hex
    token = world.signer.token("version_probe", world.tenant, jti=jti)
    wrong = world.api.call("GET", "/core-tasks/probe", token=token)  # purpose version_probe on a core_task_read route
    assert wrong.status_code == 403
    assert world.api.call("GET", "/version", token=token).status_code == 200


def test_every_denial_is_a_valid_error_envelope_not_problem_json(world: World) -> None:
    resp = world.api.call("GET", "/version")
    assert resp.headers["content-type"].startswith("application/json")
    assert_valid("ErrorEnvelope", resp.json())


def test_traceparent_trace_id_is_echoed_in_error_envelopes(world: World) -> None:
    trace = uuid.uuid4().hex
    resp = world.api.call("GET", "/version", headers={"traceparent": f"00-{trace}-{uuid.uuid4().hex[:16]}-01"})
    assert assert_error(resp, 401, "pulso:auth_invalid")["trace_id"] == trace


def test_unknown_path_is_404_not_found_envelope(world: World) -> None:
    resp = world.api.call("GET", "/no-such-route", purpose="version_probe")
    assert_error(resp, 404, "pulso:not_found")


def test_wrong_method_is_an_envelope_not_core_problem_json(world: World) -> None:
    resp = world.api.call("DELETE", "/version", purpose="version_probe")
    assert resp.status_code == 405
    assert_valid("ErrorEnvelope", resp.json())
    assert resp.json()["code"] == "pulso:http_error"


def test_signer_claim_shape_matches_the_published_schema(world: World) -> None:
    claims = world.signer.claims("core_task_invoke", world.tenant)
    claims = {k: v for k, v in claims.items() if k != "purpose"} | {"purpose": "core_task_invoke"}
    assert_valid("ServiceJwtClaims", claims)
    assert isinstance(world.signer, Signer)

"""Credential issuer (bot JWS TTL <= 15 min, two independent signers) and the binding callback (CAP-27)."""

from __future__ import annotations

import json
from datetime import UTC, datetime, timedelta
from typing import Any

import httpx
import pytest
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey

from pulso_core_runtime.credentials.issuer import CredentialIssuer, PrincipalSigner, load_signer
from pulso_core_runtime.internal.auth import b64url_encode
from pulso_core_runtime.invoke.binding import BindingService
from pulso_core_runtime.invoke.context import InvocationContext, InvocationRegistry
from pulso_core_runtime.invoke.errors import BridgeError
from pulso_core_runtime.store.migrations import apply_l3
from pulso_core_runtime.store.receipts import ReceiptStore

from .conftest import PgDbs

pytestmark = [pytest.mark.l3a]
NOW = datetime(2026, 10, 3, 12, 0, tzinfo=UTC)


def _issuer() -> tuple[CredentialIssuer, Ed25519PrivateKey, Ed25519PrivateKey]:
    staff, ident = Ed25519PrivateKey.generate(), Ed25519PrivateKey.generate()
    return CredentialIssuer({"staff": PrincipalSigner("st1", staff), "identity": PrincipalSigner("id1", ident)},
                            now=lambda: NOW), staff, ident


def _verifier(key: Ed25519PrivateKey, kid: str) -> Any:
    from agent_core.adapters.jws_identity import JwsIdentityVerifier
    return JwsIdentityVerifier({kid: key.public_key()}, {}, lambda ref, now: False)


def test_issue_registry_bot_is_valid_only_in_the_staff_verifier() -> None:
    issuer, staff, ident = _issuer()
    out = issuer.issue(claims_tenant="t1", tenant_id="t1", role="constructor", purpose="registry_write")
    assert out["kid"] == "st1" and out["exp"] == int((NOW + timedelta(minutes=15)).timestamp())
    p = _verifier(staff, "st1").verify(out["jws"])
    assert p.type.value == "builder" and p.roles == ["constructor"] and p.id == "pulso-constructor:t1"
    assert p.auth.level.value == "session" and "actor" not in p.attrs and p.exp - p.auth.at == timedelta(minutes=15)
    from agent_core.domain import CredentialsInvalid
    with pytest.raises(CredentialsInvalid):
        _verifier(ident, "id1").verify(out["jws"])  # not valid in the identity verifier


def test_issue_task_principal_uses_identity_key() -> None:
    issuer, staff, ident = _issuer()
    out = issuer.issue(claims_tenant="t1", tenant_id="t1", role="constructor", purpose="core_task")
    assert out["kid"] == "id1" and _verifier(ident, "id1").verify(out["jws"]).roles == ["constructor"]


@pytest.mark.parametrize("role,purpose", [("aprobador", "registry_write"), ("admin", "core_task"),
                                          ("constructor", "approve"), ("human", "registry_write")])
def test_no_human_or_approver_credentials(role: str, purpose: str) -> None:
    issuer, *_ = _issuer()
    with pytest.raises(BridgeError) as exc:
        issuer.issue(claims_tenant="t1", tenant_id="t1", role=role, purpose=purpose)
    assert exc.value.code == "pulso:credential_not_issuable" and exc.value.status == 403


def test_tenant_claim_must_match_body_and_ttl_is_capped() -> None:
    issuer, *_ = _issuer()
    with pytest.raises(BridgeError) as exc:
        issuer.issue(claims_tenant="t2", tenant_id="t1", role="constructor", purpose="core_task")
    assert exc.value.code == "pulso:tenant_mismatch"
    capped = CredentialIssuer({"identity": PrincipalSigner("id1", Ed25519PrivateKey.generate())}, now=lambda: NOW,
                              ttl=timedelta(hours=3))
    assert capped.issue(claims_tenant="t1", tenant_id="t1", role="constructor", purpose="core_task")["exp"] == \
        int((NOW + timedelta(minutes=15)).timestamp())
    with pytest.raises(BridgeError) as exc2:
        capped.issue(claims_tenant="t1", tenant_id="t1", role="constructor", purpose="registry_write")
    assert exc2.value.code == "pulso:credential_signing_unavailable"


def test_load_signer_errors_name_file_not_value(tmp_path: Any) -> None:
    f = tmp_path / "bridge-principal.json"
    f.write_text(json.dumps({"kid": "k", "key": b64url_encode(b"short")}))
    with pytest.raises(ValueError) as exc:
        load_signer(f)
    assert "bridge-principal.json" in str(exc.value) and "short" not in str(exc.value)
    seed = Ed25519PrivateKey.generate().private_bytes_raw()
    f.write_text(json.dumps({"kid": "k", "key": b64url_encode(seed)}))
    assert load_signer(f).kid == "k"


# ---- binding callback ----------------------------------------------------------------------------------

@pytest.fixture
def bound(pg: PgDbs) -> Any:
    from pulso_core_runtime.internal.store import ensure_schema
    ensure_schema(pg.runtime)
    apply_l3(pg.runtime)
    store = ReceiptStore(pg.runtime)
    reg = InvocationRegistry()
    store.begin(tenant_id="t1", key="cmd1", digest="d", stage="writer", job_id="j1", attempt=1, release_id="r",
                task_binding_ref="ref1", principal_id="p")
    store.transition("t1", "cmd1", "sent")
    reg.register(InvocationContext("t1", "j1", "writer", 1, "ref1", "cmd1", "d", "b1", NOW + timedelta(days=9999)))
    attrs = {"tenant": "t1", "job": "j1", "task_binding_ref": "ref1"}
    return store, reg, attrs


def _service(bound: Any, handler: Any) -> BindingService:
    store, reg, _ = bound
    return BindingService(store=store, registry=reg, control_api_url="http://control.test/", bridge_instance_id="b1",
                          signing_key=Ed25519PrivateKey.generate(), kid="cb1",
                          transport=httpx.MockTransport(handler))


@pytest.mark.pg
def test_binding_200_confirms_receipt_and_returns_stage_facts(bound: Any) -> None:
    store, _, attrs = bound
    seen: list[httpx.Request] = []

    def handler(req: httpx.Request) -> httpx.Response:
        seen.append(req)
        return httpx.Response(200, json={"ok": True})

    res = _service(bound, handler).bind(run_id="run-9", principal_attrs=attrs)
    assert res.ok and res.facts == {"is_scout": False, "is_verifier": False, "is_builder": False, "is_writer": True}
    req = seen[0]
    assert req.url.path == "/internal/v1/core-task-bindings" and req.headers["idempotency-key"] == "cmd1"
    sent = json.loads(req.content)
    assert sent["core_run_id"] == "run-9" and sent["task_binding_ref"] == "ref1" and sent["tenant_id"] == "t1"
    assert req.headers["authorization"].startswith("Bearer ")
    row = store.get("t1", "cmd1")
    assert row is not None and row.state == "binding_confirmed" and row.core_run_id == "run-9"


@pytest.mark.pg
@pytest.mark.parametrize("status,unproven", [(409, False), (404, False), (503, True)])
def test_binding_denials(bound: Any, status: int, unproven: bool) -> None:
    store, _, attrs = bound
    res = _service(bound, lambda req: httpx.Response(status, json={})).bind(run_id="run-9", principal_attrs=attrs)
    assert not res.ok and res.reason == "pulso:binding_failed" and res.effect_unproven is unproven
    row = store.get("t1", "cmd1")
    assert row is not None and row.state == ("manual_reconcile" if unproven else "sent")


@pytest.mark.pg
def test_binding_timeout_retries_once_then_manual_reconcile(bound: Any) -> None:
    store, _, attrs = bound
    calls: list[int] = []

    def handler(req: httpx.Request) -> httpx.Response:
        calls.append(1)
        raise httpx.ConnectTimeout("t")

    res = _service(bound, handler).bind(run_id="run-9", principal_attrs=attrs)
    assert not res.ok and res.effect_unproven and len(calls) == 2
    row = store.get("t1", "cmd1")
    assert row is not None and row.state == "manual_reconcile" and row.reason == "binding_unproven"


@pytest.mark.pg
def test_binding_without_or_with_crossed_context_is_denied_without_callback(bound: Any) -> None:
    calls: list[int] = []

    def handler(req: httpx.Request) -> httpx.Response:
        calls.append(1)
        return httpx.Response(200)

    svc = _service(bound, handler)
    assert svc.bind(run_id="r", principal_attrs={}).reason == "pulso:context_missing"
    assert svc.bind(run_id="r", principal_attrs={"tenant": "t2", "job": "j1", "task_binding_ref": "ref1"}).reason == \
        "pulso:context_mismatch"
    assert calls == []

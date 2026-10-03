"""BrokerArtifactPort + BrokerSandboxClient against the loopback broker/bank (real HTTP, route-table doubles).

Real: HTTP transport, per-attempt service JWTs (Ed25519), the ArmRunner eight steps, PG16 (arm rows, registry).
DOUBLES (declared): Loopback lab-broker artifact route + stateful bank (`BankBackend`), FakeBroker authorisation."""

from __future__ import annotations

import base64
import hashlib
import json
import time
import uuid
from collections.abc import Iterator
from typing import Any

import httpx
import pytest
from agent_core.domain.json import canonical_bytes
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey

from integration.loopback import BankBackend, Loopback
from pulso_core_runtime.evaluation.broker_clients import BrokerArtifactPort, BrokerSandboxClient
from pulso_core_runtime.evaluation.sandbox_port import SandboxTimeout, SandboxUnavailable
from pulso_core_runtime.internal.auth import sign_service_jwt

pytestmark = pytest.mark.runtime
KEY = Ed25519PrivateKey.generate()
CTX = {"tenant_id": "t1", "job_id": "arm-1", "campaign_ref": "camp-1", "case_ref": "case-1", "arm": "baseline",
       "repetition": 0}


def mint(claims: dict[str, Any]) -> str:
    now = int(time.time())
    return sign_service_jwt(KEY, kid="cb1", claims={
        "iss": "core-bridge", "aud": "lab-broker", "sub": "bridge:1", "iat": now, "exp": now + 60,
        "jti": uuid.uuid4().hex, **claims})


def claims_of(request: httpx.Request) -> dict[str, Any]:
    token = request.headers["authorization"].removeprefix("Bearer ")
    body = token.split(".")[1]
    return json.loads(base64.urlsafe_b64decode(body + "=" * (-len(body) % 4)))  # type: ignore[no-any-return]


@pytest.fixture
def bank() -> Iterator[tuple[Loopback, BankBackend]]:
    backend = BankBackend()
    loop = Loopback(backend)
    try:
        yield loop, backend
    finally:
        loop.close()


def manifest_artifact(content: Any) -> dict[str, Any]:
    return {"schema_version": "1", "encoding": "json", "content": content, "byte_length": 10,
            "artifact": {"id": "art-1", "media_type": "application/json",
                         "digest": "sha256:" + hashlib.sha256(canonical_bytes(content)).hexdigest()}}


def test_artifact_port_takes_binding_ref_and_sends_a_per_attempt_artifact_read_jwt(bank) -> None:  # type: ignore[no-untyped-def]
    loop, backend = bank
    sealed = {"scenarios": [{"id": "s1"}], "entries": {"s1": {}}}
    backend.artifacts["art-1"] = manifest_artifact(sealed)
    port = BrokerArtifactPort(loop.url, mint)
    assert port.artifact_get("art-1", "t1", "bind-1") == sealed
    assert port.artifact_get("art-1", "t1", "bind-1") == sealed
    first, second = (claims_of(r) for r in backend.requests)
    assert first["aud"] == "lab-broker" and first["scope"] == "artifact_read" and first["purpose"] == "artifact_read"
    assert first["tenant_id"] == "t1" and first["binding_ref"] == "bind-1" and first["jti"] != second["jti"]
    assert backend.requests[0].headers["x-pulso-binding-ref"] == "bind-1"


def test_artifact_port_fails_closed(bank) -> None:  # type: ignore[no-untyped-def]
    loop, backend = bank
    port = BrokerArtifactPort(loop.url, mint)
    with pytest.raises(LookupError):  # unknown artifact
        port.artifact_get("nope", "t1", "bind-1")
    tampered = manifest_artifact({"scenarios": []})
    tampered["artifact"]["digest"] = "sha256:" + "0" * 64
    backend.artifacts["bad"] = tampered
    with pytest.raises(LookupError):  # digest mismatch
        port.artifact_get("bad", "t1", "bind-1")
    locked = manifest_artifact({"scenarios": []})
    locked["artifact"]["final_locked"] = True
    backend.artifacts["locked"] = locked
    with pytest.raises(LookupError):  # final-locked (oracle/gold) artifacts are never handed to the arm runner
        port.artifact_get("locked", "t1", "bind-1")
    backend.artifacts["text"] = {**manifest_artifact({"a": 1}), "encoding": "utf8", "content": "x"}
    with pytest.raises(LookupError):  # not a JSON object manifest
        port.artifact_get("text", "t1", "bind-1")
    with pytest.raises(LookupError):  # transport down
        BrokerArtifactPort("http://127.0.0.1:1", mint).artifact_get("art-1", "t1", "bind-1")


def test_sandbox_client_full_lifecycle_with_sandbox_scope_and_purposes(bank) -> None:  # type: ignore[no-untyped-def]
    loop, backend = bank
    client = BrokerSandboxClient(loop.url, mint)
    session = client.open("bind-1", "seed-1", CTX)
    assert session.revision == 0 and session.initial_state_digest.startswith("sha256:")
    opened = backend.bodies[-1]
    assert opened == {"campaign_ref": "camp-1", "case_ref": "case-1", "arm": "baseline", "repetition": 0,
                      "seed_manifest_ref": "seed-1"}  # nothing else leaves the bridge
    done = client.act(session, "key-1", 0, {"type": "case.open", "args": {"customer_ref": "c1"}})
    assert done.revision == 1 and done.effect_receipt == "rcpt-key-1" and done.result["type"] == "case.open"  # type: ignore[index]
    assert client.act(session, "key-1", 1, {"type": "case.open", "args": {"customer_ref": "c1"}}) == done  # replay
    assert client.readback(session, "key-1") == done and client.readback(session, "other") is None
    assert client.reset(session).revision == 0
    assert client.close(session, "arm_done") == f"final:{session.session_ref}"
    purposes = [(claims_of(r)["scope"], claims_of(r)["purpose"]) for r in backend.requests]
    assert purposes == [("sandbox", "sandbox_open"), ("sandbox", "sandbox_act"), ("sandbox", "sandbox_act"),
                        ("sandbox", "sandbox_read"), ("sandbox", "sandbox_read"), ("sandbox", "sandbox_open"),
                        ("sandbox", "sandbox_close")]
    assert {claims_of(r)["binding_ref"] for r in backend.requests} == {"bind-1"}
    assert {claims_of(r)["tenant_id"] for r in backend.requests} == {"t1"}
    assert len({claims_of(r)["jti"] for r in backend.requests}) == len(backend.requests)  # fresh jti per attempt
    assert backend.requests[1].headers["idempotency-key"] == "key-1"


def test_sandbox_client_failure_semantics(bank) -> None:  # type: ignore[no-untyped-def]
    loop, backend = bank
    client = BrokerSandboxClient(loop.url, mint)
    backend.open_mode = "503"
    with pytest.raises(SandboxUnavailable):  # nothing was sent to the bank state: failed_infra
        client.open("bind-1", "seed-1", CTX)
    with pytest.raises(SandboxUnavailable):
        BrokerSandboxClient("http://127.0.0.1:1", mint).open("bind-1", "seed-1", CTX)
    backend.open_mode = "ok"
    session = client.open("bind-1", "seed-1", CTX)
    backend.act_mode = "lose_response"  # effect applied, answer lost -> uncertain, resolved by readback
    with pytest.raises(SandboxTimeout):
        client.act(session, "key-9", 0, {"type": "case.note", "args": {}})
    got = client.readback(session, "key-9")
    assert got is not None and got.revision == 1
    backend.act_mode = "refuse"  # a 4xx means no effect
    with pytest.raises(SandboxUnavailable):
        client.act(session, "key-10", 1, {"type": "case.note", "args": {}})
    with pytest.raises(SandboxUnavailable):  # other payload under a used key: bank 409, no effect
        client.act(session, "key-9", 1, {"type": "case.note", "args": {"x": 1}})
    dead = BrokerSandboxClient("http://127.0.0.1:1", mint)
    with pytest.raises(SandboxUnavailable):  # a session this client never opened: no identity, fail closed
        dead.readback(session, "key-9")
    dead._ctx.update(client._ctx)
    with pytest.raises(SandboxTimeout):  # transport down at readback: unknown, never "absent"
        dead.readback(session, "key-9")


def test_sandbox_client_requires_tenant_context_and_does_not_leak_it_into_the_body(bank) -> None:  # type: ignore[no-untyped-def]
    loop, backend = bank
    client = BrokerSandboxClient(loop.url, mint)
    with pytest.raises(SandboxUnavailable):  # no tenant -> no token can be minted -> fail closed, zero requests
        client.open("bind-1", "seed-1", {"campaign_ref": "c"})
    assert backend.requests == []

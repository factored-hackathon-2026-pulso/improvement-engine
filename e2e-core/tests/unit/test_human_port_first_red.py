"""First RED of the approval flow (demo steps 8-9): the stand-in `HumanAuthorizationPort`.

Contract under test (local-identity README, "HumanAuthorizationPort contract"): the PORT, not Core, enforces binding.
It (1) consumes a durable intention atomically (single use), (2) calls `assert_bound` with values read from that
intention, (3) hands back the exact JWS bytes for Core. The issuer here is the real local-identity app in process."""

from __future__ import annotations

import base64
import json
import logging
import sqlite3
import subprocess
import threading
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

import pytest

pytest.importorskip("local_identity")
from codex_standin.human_port import (
    HumanAuthorizationPort,
    IntentionConsumed,
    IntentionStore,
    IntentionUnknown,
    PodmanExecTransport,
)
from fastapi.testclient import TestClient
from local_identity.app import build_app
from local_identity.client import (
    BindingMismatch,
    IssuerError,
    LocalIdentityClient,
    ServiceSigner,
)
from local_identity.config import load_config
from local_identity.keys import generate_keyset

TENANT = "tenant-local"
HASH = "a" * 64


@pytest.fixture
def issuer(tmp_path: Path) -> LocalIdentityClient:
    keys = generate_keyset(tmp_path / "keys", tenant_id=TENANT)
    env = {"LOCAL_IDENTITY_PROFILE": "local",
           "LOCAL_IDENTITY_SERVICE_KEYS": str(keys.dir / "human-issuer-service-keys.json"),
           "LOCAL_IDENTITY_SESSION_SIGNER": str(keys.dir / "session-signer.json"),
           "LOCAL_IDENTITY_HUMAN_STAFF_SIGNER": str(keys.dir / "human-staff-signer.json"),
           "LOCAL_IDENTITY_IDENTITIES": str(keys.dir / "identities.json")}
    http = TestClient(build_app(load_config(env)), base_url="http://human-issuer:8083")
    return LocalIdentityClient("http://human-issuer:8083", ServiceSigner.from_file(keys.dir / "control-api-signer.json"),
                               tenant_id=TENANT, http_client=http)


def _port(issuer: Any, tmp_path: Path) -> HumanAuthorizationPort:
    return HumanAuthorizationPort(IntentionStore(tmp_path / "intentions.sqlite3"), issuer)


def _target(rev: int = 3) -> dict[str, Any]:
    return {"proposal_id": "prop-1", "candidate_hash": HASH, "expected_revision": rev}


def _claims(jws: str) -> dict[str, Any]:
    return json.loads(base64.urlsafe_b64decode(jws.split(".")[1] + "=="))  # type: ignore[no-any-return]


def test_an_intention_is_single_use(issuer: LocalIdentityClient, tmp_path: Path) -> None:
    port = _port(issuer, tmp_path)
    it = port.create_intention(tenant_id=TENANT, actor_ref="local-supervisor", operation="approve", target=_target())
    auth = port.authorize(it.intention_id)
    claims = _claims(auth.reveal())
    assert claims["attrs"]["operation"] == "approve" and claims["attrs"]["candidate_hash"] == HASH
    assert claims["attrs"]["command_ref"] == it.command_ref and claims["attrs"]["challenge_ref"] == it.challenge_ref
    with pytest.raises(IntentionConsumed):
        port.authorize(it.intention_id)


def test_single_use_holds_under_concurrency(issuer: LocalIdentityClient, tmp_path: Path) -> None:
    port = _port(issuer, tmp_path)
    it = port.create_intention(tenant_id=TENANT, actor_ref="local-supervisor", operation="approve", target=_target())
    results: list[str] = []

    def go() -> None:
        try:
            port.authorize(it.intention_id)
            results.append("ok")
        except IntentionConsumed:
            results.append("consumed")

    threads = [threading.Thread(target=go) for _ in range(8)]
    [t.start() for t in threads]
    [t.join() for t in threads]
    assert sorted(results) == ["consumed"] * 7 + ["ok"]


def test_unknown_intention_is_refused(issuer: LocalIdentityClient, tmp_path: Path) -> None:
    with pytest.raises(IntentionUnknown):
        _port(issuer, tmp_path).authorize("nope")


def test_the_intention_is_durable_across_port_instances(issuer: LocalIdentityClient, tmp_path: Path) -> None:
    first = _port(issuer, tmp_path)
    it = first.create_intention(tenant_id=TENANT, actor_ref="local-supervisor", operation="publish", target=_target(4))
    second = _port(issuer, tmp_path)  # a fresh process would reopen the same file
    assert second.authorize(it.intention_id).kid.startswith("local-sim-human-")
    with pytest.raises(IntentionConsumed):
        first.authorize(it.intention_id)


def test_binding_is_checked_against_the_intention_not_the_issuer_answer(issuer: LocalIdentityClient, tmp_path: Path) -> None:
    """An issuer (or a swapped credential) answering for ANOTHER target must be refused by assert_bound."""
    port = _port(issuer, tmp_path)
    it = port.create_intention(tenant_id=TENANT, actor_ref="local-supervisor", operation="approve", target=_target(3))

    class Swapped:
        def command_authorization(self, **kw: Any) -> Any:
            return issuer.command_authorization(**{**kw, "target": _target(9)})  # signed for another revision

    swapped = HumanAuthorizationPort(port.store, Swapped())  # type: ignore[arg-type]
    with pytest.raises(BindingMismatch):
        swapped.authorize(it.intention_id)
    with pytest.raises(IntentionConsumed):  # the failed attempt still burned the intention (no retry on stale bytes)
        port.authorize(it.intention_id)


def test_issuer_refusal_surfaces_and_burns_nothing_silently(issuer: LocalIdentityClient, tmp_path: Path) -> None:
    port = _port(issuer, tmp_path)
    it = port.create_intention(tenant_id=TENANT, actor_ref="local-supervisor", operation="revoke",
                               target={"release_id": "rel-1", "agent_id": "a", "alias": "staging", "expected_revision": 0})
    with pytest.raises(IssuerError) as err:  # a supervisor cannot obtain a revoke authorization
        port.authorize(it.intention_id)
    assert err.value.code == "pulso:role_not_allowed"
    assert port.store.state(it.intention_id) == "failed"


def test_the_jws_never_leaks_through_repr_str_or_logging(issuer: LocalIdentityClient, tmp_path: Path,
                                                         caplog: pytest.LogCaptureFixture) -> None:
    port = _port(issuer, tmp_path)
    it = port.create_intention(tenant_id=TENANT, actor_ref="local-supervisor", operation="approve", target=_target())
    with caplog.at_level(logging.DEBUG):
        auth = port.authorize(it.intention_id)
        logging.getLogger("t").info("authorization %s %r", auth, auth)
    jws = auth.reveal()
    assert jws.split(".")[2] not in caplog.text and jws not in repr(auth) and jws not in str(auth)
    db = sqlite3.connect(tmp_path / "intentions.sqlite3")
    dump = "\n".join(str(r) for t in ("intentions",) for r in db.execute(f"select * from {t}"))
    assert jws.split(".")[2] not in dump  # the store keeps the intention, never the credential


def test_exec_transport_keeps_the_service_token_out_of_argv(monkeypatch: pytest.MonkeyPatch) -> None:
    import httpx

    seen: dict[str, Any] = {}

    def fake_run(argv: list[str], **kw: Any) -> Any:
        seen["argv"], seen["stdin"] = argv, kw["input"]
        out = json.dumps({"status": 200, "body": base64.b64encode(b'{"ok":true}').decode()})
        return subprocess.CompletedProcess(argv, 0, stdout=out, stderr="")

    monkeypatch.setattr(subprocess, "run", fake_run)
    transport = PodmanExecTransport("pulso-dev", "ctr-1", podman="podman")
    with httpx.Client(transport=transport, base_url="http://human-issuer:8083") as client:
        r = client.post("/x", json={"a": 1}, headers={"Authorization": "Bearer SECRET-TOKEN"})
    assert r.status_code == 200 and r.json() == {"ok": True}
    assert "SECRET-TOKEN" not in " ".join(seen["argv"]) and "SECRET-TOKEN" in seen["stdin"]
    assert seen["argv"][:5] == ["podman", "--connection", "pulso-dev", "exec", "-i"] and "ctr-1" in seen["argv"]


def test_now_is_used_to_refuse_a_stale_credential(issuer: LocalIdentityClient, tmp_path: Path) -> None:
    far = datetime(2099, 1, 1, tzinfo=UTC)
    port = HumanAuthorizationPort(IntentionStore(tmp_path / "i.sqlite3"), issuer, now=lambda: far)
    it = port.create_intention(tenant_id=TENANT, actor_ref="local-supervisor", operation="approve", target=_target())
    with pytest.raises((BindingMismatch, IssuerError)):
        port.authorize(it.intention_id)

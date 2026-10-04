"""control-api black-box contract: ONE file, run against BOTH the Python double (e2e-core fixtures_app + platform-sim
ingest fixture) and the Rust server (seams/crates/control-api). Pure HTTP; no knowledge of either implementation.

    <venv>/python -m pytest contracts/control-api/test_blackbox.py -p no:cacheprovider
    CONTROL_API_TARGETS=rust   (or double)   CONTROL_API_BIN=<path to the control-api binary>

The file generates its own Ed25519 keys and starts the target itself (the double in-process via uvicorn, the Rust
server as a subprocess configured with the same E2E_VERIFY_KEYS / E2E_PORT env the double's container uses).

Surfaces: POST /internal/v1/core-task-bindings, POST /internal/v1/broker/authorizations/check,
GET|POST /internal/v1/broker/artifacts, admin /_e2e/config (artifact seeding, deny_operations, faults).
"""

from __future__ import annotations

import base64
import hashlib
import json
import os
import socket
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request
import uuid
from pathlib import Path
from typing import Any

import pytest

# Needs the e2e venv (cryptography, fastapi, uvicorn, rfc8785); skip with a clear reason instead of breaking collection.
pytest.importorskip("cryptography", reason="control-api black-box test needs the e2e venv (see module docstring)")
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey  # noqa: E402
from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat  # noqa: E402

ROOT = Path(__file__).resolve().parents[2]
BIN = os.environ.get("CONTROL_API_BIN", "D:/cargo-targets/claude-seams-capi/debug/control-api.exe")
TARGETS = os.environ.get("CONTROL_API_TARGETS", "double,rust").split(",")
BROKER = "/internal/v1/broker"
SUB = "exporter-binding-1"  # the ingest fixture pins `sub` of artifact uploads to one registered binding


def b64u(raw: bytes) -> str:
    return base64.urlsafe_b64encode(raw).rstrip(b"=").decode()


class Keys:
    def __init__(self) -> None:
        self.cb, self.ex = Ed25519PrivateKey.generate(), Ed25519PrivateKey.generate()

    @staticmethod
    def pub(k: Ed25519PrivateKey) -> str:
        return b64u(k.public_key().public_bytes(Encoding.Raw, PublicFormat.Raw))

    def verify_keys(self) -> dict[str, Any]:
        ring = {"cb": ["core-bridge", "control-api", self.pub(self.cb)],
                "ex": ["core-bridge", "lab-broker", self.pub(self.ex)]}
        ingest = {"keys": {"ex": {"iss": "core-bridge", "aud": "lab-broker", "key": self.pub(self.ex)}},
                  "binding_ref": SUB, "tenant_id": "t1"}
        return {"ring": ring, "ingest": ingest}

    def token(self, which: str, aud: str, scope: str, tenant: str | None, **over: Any) -> str:
        key, kid = (self.cb, "cb") if which == "cb" else (self.ex, "ex")
        now = int(time.time())
        claims: dict[str, Any] = {"iss": "core-bridge", "aud": aud, "sub": over.pop("sub", "bridge:1"), "scope": scope,
                                  "purpose": over.pop("purpose", scope), "iat": now, "exp": now + 60,
                                  "jti": uuid.uuid4().hex}
        if tenant is not None:
            claims["tenant_id"] = tenant
        claims.update(over)
        head = b64u(json.dumps({"alg": "EdDSA", "kid": kid, "typ": "JWT"}, separators=(",", ":")).encode())
        body = b64u(json.dumps(claims, separators=(",", ":")).encode())
        sig = b64u(key.sign(f"{head}.{body}".encode()))
        return f"{head}.{body}.{sig}"


def free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return int(s.getsockname()[1])


def wait_port(port: int, proc: subprocess.Popen[bytes] | None = None) -> None:
    deadline = time.time() + 20
    while time.time() < deadline:
        if proc is not None and proc.poll() is not None:
            raise AssertionError(f"server exited early with {proc.returncode}")
        try:
            socket.create_connection(("127.0.0.1", port), 0.2).close()
            return
        except OSError:
            time.sleep(0.05)
    raise AssertionError("server did not open its port")


class Http:
    def __init__(self, base: str) -> None:
        self.base = base

    def call(self, method: str, path: str, body: Any = None, token: str | None = None,
             headers: dict[str, str] | None = None) -> tuple[int, Any]:
        data = None if body is None else json.dumps(body).encode()
        req = urllib.request.Request(self.base + path, data=data, method=method)
        if data is not None:
            req.add_header("Content-Type", "application/json")
        if token is not None:
            req.add_header("Authorization", "Bearer " + token)
        for k, v in (headers or {}).items():
            req.add_header(k, v)
        try:
            with urllib.request.urlopen(req, timeout=10) as r:
                return r.status, json.loads(r.read() or b"null")
        except urllib.error.HTTPError as e:
            raw = e.read()
            return e.code, (json.loads(raw) if raw else None)


class Target:
    def __init__(self, name: str, http: Http, keys: Keys) -> None:
        self.name, self.http, self.keys = name, http, keys

    def admin(self, cfg: dict[str, Any]) -> None:
        assert self.http.call("POST", "/_e2e/config", cfg)[0] == 200


def _start_double(keys: Keys) -> tuple[str, Any]:
    sys.path[:0] = [str(ROOT / "e2e-core" / "src"), str(ROOT / "platform-sim")]
    os.environ["E2E_VERIFY_KEYS"] = json.dumps(keys.verify_keys())
    import uvicorn
    from codex_standin.serve import build

    port = free_port()
    server = uvicorn.Server(uvicorn.Config(build(), host="127.0.0.1", port=port, log_level="error"))
    threading.Thread(target=server.run, daemon=True).start()
    wait_port(port)
    return f"http://127.0.0.1:{port}", server


def _start_rust(keys: Keys) -> tuple[str, Any]:
    assert Path(BIN).exists(), f"control-api binary not built: {BIN}"
    port = free_port()
    env = {**os.environ, "E2E_VERIFY_KEYS": json.dumps(keys.verify_keys()), "E2E_PORT": str(port), "CONTROL_API_ADMIN": "1"}
    proc = subprocess.Popen([BIN], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    wait_port(port, proc)
    return f"http://127.0.0.1:{port}", proc


@pytest.fixture(params=TARGETS)
def t(request: pytest.FixtureRequest) -> Any:
    keys = Keys()
    base, handle = (_start_double if request.param == "double" else _start_rust)(keys)
    yield Target(request.param, Http(base), keys)
    if request.param == "double":
        handle.should_exit = True
    else:
        handle.kill()
        handle.wait()


def bind_body(tenant: str = "t1", job: str = "j1", cmd: str = "c1", digest: str = "d" * 64) -> dict[str, Any]:
    return {"schema_version": "1", "tenant_id": tenant, "job_id": job, "command_key": cmd, "request_digest": digest,
            "attempt": 1, "core_run_id": "run-1", "bridge_instance_id": "b1", "task_binding_ref": f"bind-{cmd}"}


def bind(t: Target, tenant: str = "t1", job: str = "j1", cmd: str = "c1", digest: str = "d" * 64,
         claim_tenant: str | None = None, key: str | None = None) -> tuple[int, Any]:
    tok = t.keys.token("cb", "control-api", "binding", claim_tenant or tenant, purpose="core_task_binding")
    return t.http.call("POST", "/internal/v1/core-task-bindings", bind_body(tenant, job, cmd, digest), tok,
                       {"Idempotency-Key": key or cmd})


def authz(t: Target, tenant: str, body: dict[str, Any]) -> tuple[int, Any]:
    tok = t.keys.token("ex", "lab-broker", "authorization_check", tenant, purpose="authorization_check")
    return t.http.call("POST", BROKER + "/authorizations/check", body, tok)


def read_artifact(t: Target, tenant: str, art_id: str) -> tuple[int, Any]:
    tok = t.keys.token("ex", "lab-broker", "artifact_read", tenant, purpose="artifact_read")
    return t.http.call("GET", f"{BROKER}/artifacts/{art_id}", None, tok)


def jcs_digest(content: Any) -> str:
    # canonical form for the plain-ASCII, integer-only contents used here (sorted keys, no whitespace) == RFC 8785
    return hashlib.sha256(json.dumps(content, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def upload_body(content: Any, digest: str | None = None) -> dict[str, Any]:
    return {"schema_version": "1", "binding_ref": SUB, "source_schema_ref": None, "classification": "treated",
            "information_partition": "p1", "artifact_kind": "schema", "media_type": "application/json",
            "encoding": "json", "content": content, "content_digest": digest or jcs_digest(content)}


def put_artifact(t: Target, body: dict[str, Any], tenant: str = "t1", key: str | None = None) -> tuple[int, Any]:
    tok = t.keys.token("ex", "lab-broker", "artifact_write", tenant, purpose="artifact_upload", sub=SUB)
    return t.http.call("POST", BROKER + "/artifacts", body, tok, {"Idempotency-Key": key or body["content_digest"]})


# ---- bindings (CAP-27) ------------------------------------------------------------------------------------------
def test_binding_is_exactly_once_and_conflicts_are_409(t: Target) -> None:
    assert bind(t)[0] == 200
    status, ok = bind(t)  # replay
    assert status == 200 and ok == {"schema_version": "1", "state": "confirmed", "tenant_id": "t1", "job_id": "j1"}
    status, body = bind(t, digest="e" * 64)
    assert (status, body["code"]) == (409, "digest_mismatch")
    status, body = bind(t, cmd="c2")  # another command for the same job
    assert (status, body["code"]) == (409, "binding_conflict")


def test_binding_tenant_claim_and_idempotency_key_must_match(t: Target) -> None:
    status, body = bind(t, claim_tenant="t2")
    assert (status, body["code"]) == (403, "tenant_mismatch")
    status, body = bind(t, key="other")
    assert (status, body["code"]) == (422, "idempotency_key_mismatch")


def test_binding_503_before_effect_then_retry_confirms(t: Target) -> None:
    t.admin({"faults": {"bind": ["503"]}})
    assert bind(t)[0] == 503
    assert bind(t)[0] == 200


# ---- service JWT rejections ---------------------------------------------------------------------------------------
def test_service_jwt_closed_rejections(t: Target) -> None:
    url = "/internal/v1/core-task-bindings"
    hdr = {"Idempotency-Key": "c1"}

    def call(tok: str | None) -> tuple[int, Any]:
        return t.http.call("POST", url, bind_body(), tok, hdr)

    def tk(**kw: Any) -> str:
        args = {"aud": "control-api", "scope": "binding", "tenant": "t1", "purpose": "core_task_binding", **kw}
        return t.keys.token("cb", args.pop("aud"), args.pop("scope"), args.pop("tenant"), **args)

    status, body = call(None)
    assert (status, body["code"]) == (401, "pulso:auth_missing_bearer")
    good = tk()
    assert call(good)[0] == 200
    status, body = call(good)  # same token again: receiver-owned jti replay set
    assert (status, body["code"]) == (401, "pulso:auth_jti_replayed")
    status, body = call(tk(scope="lab"))
    assert (status, body["code"]) == (403, "pulso:auth_scope_denied")
    status, body = call(tk(purpose="other"))
    assert (status, body["code"]) == (403, "pulso:auth_purpose_denied")
    status, body = call(tk(aud="lab-broker"))
    assert (status, body["code"]) == (401, "pulso:auth_wrong_audience")
    status, body = call(tk(tenant=None))
    assert (status, body["code"]) == (403, "pulso:auth_tenant_required")
    status, body = call(tk(exp=int(time.time()) - 5))
    assert (status, body["code"]) == (401, "pulso:auth_expired")
    status, body = call(tk(exp=int(time.time()) + 3600))
    assert (status, body["code"]) == (401, "pulso:auth_ttl_too_long")
    head, claims, _ = good.split(".")
    status, body = call(".".join([head, claims, b64u(b"\0" * 64)]))
    assert (status, body["code"]) == (401, "pulso:auth_bad_signature")
    status, body = call("not-a-jwt")
    assert (status, body["code"]) == (401, "pulso:auth_malformed")


# ---- authorization checks -----------------------------------------------------------------------------------------
def test_authorization_check_is_binding_scoped_and_revocable(t: Target) -> None:
    assert bind(t)[0] == 200
    body = {"binding_ref": "bind-c1", "operation": "registry/freeze", "resource_refs": [], "payload_digest": "a" * 64}
    status, ok = authz(t, "t1", body)
    assert status == 200 and ok["allowed"] is True and ok["reason_code"] is None and ok["authorization_revision"] == 1
    assert ok["valid_until"].endswith("Z")
    assert authz(t, "t1", {**body, "binding_ref": "nope"})[1]["allowed"] is False
    other = authz(t, "t2", body)[1]  # another tenant's binding: never allowed
    assert other["allowed"] is False and other["reason_code"] == "binding_unknown"
    t.admin({"deny_operations": ["registry/freeze"]})
    denied = authz(t, "t1", body)[1]
    assert denied["allowed"] is False and denied["reason_code"] == "revoked"


# ---- artifacts ----------------------------------------------------------------------------------------------------
def test_artifact_read_is_tenant_scoped(t: Target) -> None:
    t.admin({"artifacts": [{"tenant": "t1", "id": "art-1", "content": {"x": 1}}]})
    status, body = read_artifact(t, "t1", "art-1")
    assert status == 200 and body["content"] == {"x": 1} and body["encoding"] == "json"
    assert body["artifact"]["id"] == "art-1" and body["artifact"]["digest"].startswith("sha256:")
    assert body["byte_length"] == len(b'{"x":1}')
    status, body = read_artifact(t, "t2", "art-1")
    assert (status, body["code"]) == (404, "artifact_not_found")
    assert read_artifact(t, "t1", "missing")[0] == 404


def test_artifact_put_is_idempotent_by_digest(t: Target) -> None:
    body = upload_body({"b": 2, "a": [1, 2]})
    status, first = put_artifact(t, body)
    assert status == 201
    digest = body["content_digest"]
    assert first["artifact_ref"] == {"id": f"artifact:{digest}", "digest": digest, "media_type": "application/json"}
    assert first["receipt_ref"]["id"] == f"receipt:{digest}"
    status, again = put_artifact(t, body)
    assert (status, again) == (200, first)
    status, bad = put_artifact(t, upload_body({"a": 1}, digest="f" * 64))
    assert (status, bad) == (422, {"error": "digest_mismatch"})
    status, bad = put_artifact(t, body, key="0" * 64)  # Idempotency-Key must equal the digest
    assert status == 422
    status, bad = put_artifact(t, {**body, "extra": 1})
    assert (status, bad) == (422, {"error": "schema_invalid"})
    status, bad = put_artifact(t, {**upload_body({"a": 1}), "artifact_kind": "cut"})
    assert (status, bad) == (422, {"error": "source_schema_ref_required"})


def test_artifact_put_requires_write_scope_and_matching_tenant(t: Target) -> None:
    body = upload_body({"k": "v"})
    tok = t.keys.token("ex", "lab-broker", "artifact_read", "t1", purpose="artifact_upload", sub=SUB)
    assert t.http.call("POST", BROKER + "/artifacts", body, tok, {"Idempotency-Key": body["content_digest"]})[0] == 403
    assert put_artifact(t, body, tenant="t2")[0] == 403
    assert t.http.call("POST", BROKER + "/artifacts", body, None, {"Idempotency-Key": body["content_digest"]})[0] == 401


# ---- the roleplay scout: bind, authorise, read the evidence artifact -----------------------------------------------
def test_scout_flow_binding_then_authorization_then_artifact(t: Target) -> None:
    t.admin({"artifacts": [{"tenant": "t1", "id": "scout-evidence", "content": {"rows": [1, 2, 3]}}]})
    assert bind(t, job="scout-job", cmd="scout-cmd")[0] == 200
    allowed = authz(t, "t1", {"binding_ref": "bind-scout-cmd", "operation": "artifact/read",
                              "resource_refs": ["scout-evidence"], "payload_digest": "b" * 64})[1]
    assert allowed["allowed"] is True
    status, art = read_artifact(t, "t1", "scout-evidence")
    assert status == 200 and art["content"] == {"rows": [1, 2, 3]}
    assert read_artifact(t, "t2", "scout-evidence")[0] == 404

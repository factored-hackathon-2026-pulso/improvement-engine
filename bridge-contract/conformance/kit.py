"""Self-contained client kit of the conformance suite: service-JWT signer, JCS digest, schema validation, HTTP client.

Deliberately imports NOTHING from core-bridge: Codex can point the suite at any implementation (our runtime, the
platform-sim mock, their own double) with only httpx, jsonschema, referencing, cryptography and pytest installed.
Env var NAMES (values are never printed): see README section "Running the suite"."""

from __future__ import annotations

import base64
import hashlib
import json
import time
import uuid
from collections.abc import Callable
from pathlib import Path
from typing import Any

import httpx
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from jsonschema import Draft202012Validator
from referencing import Registry, Resource

ROOT = Path(__file__).resolve().parents[1]
SCHEMAS = ROOT / "schemas"
CONTRACT = json.loads((ROOT / "contract.json").read_text(encoding="utf-8"))
BASE = "/internal/v1"


def b64u(raw: bytes) -> str:
    return base64.urlsafe_b64encode(raw).rstrip(b"=").decode()


def b64u_decode(text: str) -> bytes:
    return base64.urlsafe_b64decode(text + "=" * (-len(text) % 4))


def jcs(value: Any) -> bytes:
    """RFC 8785 subset: sorted keys, compact, UTF-8, integers only (decimals travel as strings)."""
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode("utf-8")


def sha256_hex(data: bytes | str) -> str:
    return hashlib.sha256(data.encode() if isinstance(data, str) else data).hexdigest()


def request_digest(body: dict[str, Any]) -> str:
    excluded = json.loads((SCHEMAS / "CoreTaskInvocation.schema.json").read_text(encoding="utf-8"))["x-digest-excluded"]
    return sha256_hex(jcs({k: v for k, v in body.items() if k not in excluded}))


def idempotency_key(tenant: str, job: str, stage: str, attempt: int, logical_key: str) -> str:
    return sha256_hex(f"{tenant}|{job}|{stage}|{attempt}|{logical_key}")


def evaluation_context_ref(tenant: str, job: str, binding: str, proposal_id: str, candidate_hash: str,
                           attempt: int) -> str:
    """contract.json idempotency.admissions.derivation: the ref the bridge derives (annex D.4)."""
    return "evc-" + sha256_hex(f"{tenant}|{job}|{binding}|{proposal_id}|{candidate_hash}|{attempt}")[:40]


def binding_ref(tenant: str, key: str) -> str:
    return sha256_hex(f"{tenant}|{key}")


def _registry() -> Registry:
    reg: Registry = Registry()
    for path in sorted(SCHEMAS.glob("*.schema.json")):
        doc = json.loads(path.read_text(encoding="utf-8"))
        reg = reg.with_resource(path.name, Resource.from_contents(doc))
    return reg


REGISTRY = _registry()


def schema_errors(name: str, instance: Any) -> list[str]:
    schema = json.loads((SCHEMAS / f"{name}.schema.json").read_text(encoding="utf-8"))
    validator = Draft202012Validator(schema, registry=REGISTRY)
    return sorted({f"{'/'.join(str(p) for p in e.absolute_path) or '(root)'}: {e.message[:120]}"
                   for e in validator.iter_errors(instance)})


class Signer:
    """Mints Rust -> bridge service JWTs (A03 class i). `purpose_map` lets a profile translate purpose names."""

    def __init__(self, kid: str, seed: bytes, *, iss: str = "control-api", aud: str = "core-bridge",
                 now: Callable[[], float] = time.time, purpose_map: dict[str, str] | None = None) -> None:
        self.kid, self.iss, self.aud, self.now = kid, iss, aud, now
        self._key = Ed25519PrivateKey.from_private_bytes(seed)
        self.purpose_map = purpose_map or {}

    def sign(self, claims: dict[str, Any], *, header: dict[str, Any] | None = None, key: Ed25519PrivateKey | None = None,
             ) -> str:
        head = b64u(json.dumps(header or {"alg": "EdDSA", "kid": self.kid, "typ": "JWT"}, separators=(",", ":")).encode())
        body = b64u(json.dumps(claims, separators=(",", ":")).encode())
        sig = (key or self._key).sign(f"{head}.{body}".encode())
        return f"{head}.{body}.{b64u(sig)}"

    def claims(self, purpose: str, tenant: str | None, *, job_id: str | None = "job-contract", ttl: int = 60,
               **over: Any) -> dict[str, Any]:
        now = int(self.now())
        c: dict[str, Any] = {"iss": self.iss, "aud": self.aud, "sub": "worker:contract", "purpose":
                             self.purpose_map.get(purpose, purpose), "iat": now, "exp": now + ttl,
                             "jti": uuid.uuid4().hex}
        if tenant is not None:
            c["tenant_id"] = tenant
        if job_id is not None:
            c["job_id"] = job_id
        c.update(over)
        return {k: v for k, v in c.items() if v is not _DROP}

    def token(self, purpose: str, tenant: str | None, **kw: Any) -> str:
        return self.sign(self.claims(purpose, tenant, **kw))


_DROP = object()
DROP = _DROP  # pass `jti=DROP` to omit a claim


class Api:
    """Thin httpx wrapper: every call mints a fresh token (fresh jti) unless `token` is given."""

    def __init__(self, base_url: str, signer: Signer, tenant: str, timeout: float = 60.0) -> None:
        self.http = httpx.Client(base_url=base_url.rstrip("/"), timeout=timeout)
        self.signer, self.tenant = signer, tenant

    def call(self, method: str, path: str, *, purpose: str | None = None, tenant: str | None = ..., body: Any = None,  # type: ignore[assignment]
             headers: dict[str, str] | None = None, token: str | None = None, raw: bytes | None = None,
             job_id: str | None = "job-contract", claims: dict[str, Any] | None = None) -> httpx.Response:
        h = dict(headers or {})
        if token is None and purpose is not None:
            who = self.tenant if tenant is ... else tenant
            token = self.signer.token(purpose, who, job_id=job_id, **(claims or {}))
        if token is not None:
            h["Authorization"] = f"Bearer {token}"
        kw: dict[str, Any] = {"headers": h}
        if raw is not None:
            kw["content"] = raw
            h.setdefault("Content-Type", "application/json")
        elif body is not None:
            kw["json"] = body
        return self.http.request(method, BASE + path, **kw)

    def close(self) -> None:
        self.http.close()

"""Stand-in for Codex's `HumanAuthorizationPort` (demo steps 8-9, plan 17.3.2 / local-identity README).

Why the PORT enforces the binding: Core verifies only signature, `auth.level=step_up`, role and `exp` of the human
Principal and IGNORES every other attr, so Core alone cannot tell "approve prop-1@3" from "publish prop-2@9". The port:

1. keeps a DURABLE intention (what the human is about to authorise: operation + target + command/challenge refs) and
   consumes it atomically before anything is issued (single use: the Principal has no `jti`),
2. obtains the command-authorization JWS from the human issuer and calls `assert_bound` with values read from that
   intention (never from the request),
3. hands the exact JWS bytes back for Core. The credential is wrapped so it never reaches a repr, a log line or the store.

`PodmanExecTransport` lets the host-side stand-in reach the human-issuer, which is INTERNAL ONLY (no host port), by
executing a tiny HTTP relay inside that container; the service token travels on stdin, never in argv."""

from __future__ import annotations

import base64
import json
import sqlite3
import subprocess
import uuid
from collections.abc import Callable
from dataclasses import dataclass, field
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

import httpx
from local_identity.client import IssuerError, LocalIdentityClient, assert_bound

__all__ = ["HumanAuthorizationPort", "Intention", "IntentionConsumed", "IntentionError", "IntentionStore",
           "IntentionUnknown", "IssuerError", "PodmanExecTransport", "SensitiveJws"]


class IntentionError(Exception):
    """Base class: the intention cannot be used."""


class IntentionUnknown(IntentionError):
    """No such intention."""


class IntentionConsumed(IntentionError):
    """The intention was already used (or burned by a failed attempt): a new intention is required."""


@dataclass(frozen=True)
class Intention:
    intention_id: str
    tenant_id: str
    actor_ref: str
    command_ref: str
    operation: str
    target: dict[str, Any]
    challenge_ref: str


class SensitiveJws:
    """A bearer credential. Printing, formatting or logging it yields a redaction; `reveal()` is the only way out."""

    __slots__ = ("_jws", "exp", "kid")

    def __init__(self, jws: str, kid: str, exp: int) -> None:
        self._jws, self.kid, self.exp = jws, kid, exp

    def reveal(self) -> str:
        return self._jws

    def bearer(self) -> dict[str, str]:
        return {"Authorization": f"Bearer {self._jws}"}

    def __repr__(self) -> str:
        return f"SensitiveJws(kid={self.kid!r}, <redacted>)"

    __str__ = __repr__

    def __format__(self, spec: str) -> str:
        return repr(self)


class IntentionStore:
    """Durable intentions (SQLite file). States: open -> consumed | failed. The credential is never stored."""

    def __init__(self, path: Path | str) -> None:
        self._path = str(path)
        with self._conn() as c:
            c.execute("create table if not exists intentions (intention_id text primary key, tenant_id text not null,"
                      " actor_ref text not null, command_ref text not null, operation text not null,"
                      " target_json text not null, challenge_ref text not null, state text not null,"
                      " created_at text not null)")

    def _conn(self) -> sqlite3.Connection:
        conn = sqlite3.connect(self._path, timeout=30, isolation_level=None)
        conn.execute("pragma busy_timeout=30000")
        return conn

    def create(self, *, tenant_id: str, actor_ref: str, operation: str, target: dict[str, Any],
               command_ref: str | None = None, challenge_ref: str | None = None) -> Intention:
        it = Intention(uuid.uuid4().hex, tenant_id, actor_ref, command_ref or f"cmd-{uuid.uuid4().hex[:12]}",
                       operation, dict(target), challenge_ref or f"chal-{uuid.uuid4().hex[:12]}")
        with self._conn() as c:
            c.execute("insert into intentions values (?,?,?,?,?,?,?,?,?)",
                      (it.intention_id, it.tenant_id, it.actor_ref, it.command_ref, it.operation,
                       json.dumps(it.target, sort_keys=True), it.challenge_ref, "open",
                       datetime.now(UTC).isoformat()))
        return it

    def consume(self, intention_id: str) -> Intention:
        """Atomic open -> consumed. Exactly one caller wins; every other gets IntentionConsumed."""
        with self._conn() as c:
            row = c.execute("select tenant_id, actor_ref, command_ref, operation, target_json, challenge_ref"
                            " from intentions where intention_id=?", (intention_id,)).fetchone()
            if row is None:
                raise IntentionUnknown(intention_id)
            won = c.execute("update intentions set state='consumed' where intention_id=? and state='open'",
                            (intention_id,)).rowcount
        if won != 1:
            raise IntentionConsumed(intention_id)
        return Intention(intention_id, row[0], row[1], row[2], row[3], json.loads(row[4]), row[5])

    def mark_failed(self, intention_id: str) -> None:
        with self._conn() as c:
            c.execute("update intentions set state='failed' where intention_id=?", (intention_id,))

    def state(self, intention_id: str) -> str | None:
        with self._conn() as c:
            row = c.execute("select state from intentions where intention_id=?", (intention_id,)).fetchone()
        return None if row is None else str(row[0])


@dataclass
class HumanAuthorizationPort:
    store: IntentionStore
    issuer: LocalIdentityClient
    now: Callable[[], datetime] = field(default=lambda: datetime.now(UTC))

    def create_intention(self, *, tenant_id: str, actor_ref: str, operation: str, target: dict[str, Any],
                         command_ref: str | None = None, challenge_ref: str | None = None) -> Intention:
        return self.store.create(tenant_id=tenant_id, actor_ref=actor_ref, operation=operation, target=target,
                                 command_ref=command_ref, challenge_ref=challenge_ref)

    def authorize(self, intention_id: str) -> SensitiveJws:
        """Consume the intention, obtain the JWS, verify it is bound to THAT intention, return it for Core."""
        it = self.store.consume(intention_id)
        values: dict[str, Any] = {"tenant_id": it.tenant_id, "actor_ref": it.actor_ref, "command_ref": it.command_ref,
                                  "operation": it.operation, "target": it.target, "challenge_ref": it.challenge_ref}
        try:
            auth = self.issuer.command_authorization(**values, nonce=uuid.uuid4().hex)
            assert_bound(auth.authorization_jws, **values, now=self.now())
        except Exception:
            self.store.mark_failed(intention_id)
            raise
        return SensitiveJws(auth.authorization_jws, auth.kid, auth.exp)


_RELAY = """
import base64, json, sys, urllib.error, urllib.request
req = json.load(sys.stdin)
r = urllib.request.Request("http://127.0.0.1:8083" + req["path"], data=base64.b64decode(req["body"]) or None,
                           method=req["method"], headers=req["headers"])
try:
    resp = urllib.request.urlopen(r, timeout=10)
    status, body = resp.status, resp.read()
except urllib.error.HTTPError as exc:
    status, body = exc.code, exc.read()
print(json.dumps({"status": status, "body": base64.b64encode(body).decode()}))
"""


class PodmanExecTransport(httpx.BaseTransport):
    """httpx transport that performs each request INSIDE the internal-only human-issuer container
    (`podman --connection C exec -i NAME python -c <relay>`). Request (incl. the Authorization header) goes through
    stdin; the relay program travels base64-encoded so no quote/newline has to survive native argument passing."""

    def __init__(self, connection: str, container: str, *, podman: str, timeout: float = 60.0) -> None:
        self._argv = [podman, "--connection", connection, "exec", "-i", container, "python", "-c",
                      "exec(__import__('base64').b64decode('" + base64.b64encode(_RELAY.encode()).decode() + "'))"]
        self._timeout = timeout

    def handle_request(self, request: httpx.Request) -> httpx.Response:
        payload = json.dumps({"method": request.method, "path": request.url.raw_path.decode(),
                              "headers": {k: v for k, v in request.headers.items() if k.lower() != "host"},
                              "body": base64.b64encode(request.read()).decode()})
        proc = subprocess.run(self._argv, input=payload, capture_output=True, text=True, timeout=self._timeout, check=False)
        if proc.returncode != 0:
            raise httpx.ConnectError("human-issuer relay failed")
        out = json.loads(proc.stdout.strip().splitlines()[-1])
        return httpx.Response(out["status"], content=base64.b64decode(out["body"]),
                              headers={"content-type": "application/json"}, request=request)

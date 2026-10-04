"""HTTP client of the engine stand-in towards the Core bridge (`/internal/v1/*`, A03 class i: iss=control-api,
aud=core-bridge, singular scope + purpose, tenant_id claim, fresh jti per attempt, TTL 60 s)."""

from __future__ import annotations

import time
import uuid
from typing import Any

import httpx

from codex_standin import jwtsvc

PURPOSES = {
    "invoke": "core_task_invoke", "read": "core_task_read", "admit": "evaluation_admit",
    "arm_run": "evaluation_arm_run", "arm_read": "evaluation_arm_read", "version": "version_probe",
    "aliases": "alias_read", "dry_run": "authoring_dry_run", "credentials": "credential_issue"}


class Bridge:
    def __init__(self, base_url: str, key_seed: str, kid: str, *, iss: str = "control-api", sub: str = "worker:e2e",
                 timeout: float = 120.0) -> None:
        self._key = jwtsvc.private_from_seed(key_seed)
        self._kid, self._iss, self._sub = kid, iss, sub
        self.base = base_url.rstrip("/") + "/internal/v1"
        self._http = httpx.Client(timeout=timeout)

    def token(self, purpose: str, tenant: str | None, **extra: Any) -> str:
        now = int(time.time())
        claims: dict[str, Any] = {"iss": self._iss, "aud": "core-bridge", "sub": self._sub, "scope": purpose,
                                  "purpose": purpose, "iat": now, "exp": now + 60, "jti": uuid.uuid4().hex, **extra}
        if tenant is not None:
            claims["tenant_id"] = tenant
        return jwtsvc.sign(self._key, self._kid, claims)

    def call(self, method: str, path: str, op: str, tenant: str | None, *, json: Any = None,
             headers: dict[str, str] | None = None, token: str | None = None, **claims: Any) -> httpx.Response:
        h = {"Authorization": "Bearer " + (token or self.token(PURPOSES[op], tenant, **claims)), **(headers or {})}
        return self._http.request(method, self.base + path, json=json, headers=h)

    def version(self) -> dict[str, Any]:
        return self.call("GET", "/version", "version", None).json()  # type: ignore[no-any-return]

    def invoke(self, tenant: str, key: str, body: dict[str, Any]) -> httpx.Response:
        # Annex D auth alignment (A03 i): the signed `job_id` claim must equal the invoked body job.
        return self.call("POST", "/core-tasks/invoke", "invoke", tenant, json=body,
                         headers={"Idempotency-Key": key}, job_id=body.get("job_id"))

    def read_task(self, tenant: str, task_id: str) -> httpx.Response:
        return self.call("GET", f"/core-tasks/{task_id}", "read", tenant)

    def admit(self, tenant: str, job_id: str, body: dict[str, Any]) -> httpx.Response:
        return self.call("POST", "/evaluation/admissions", "admit", tenant, json=body, job_id=job_id)

    def arm_run(self, tenant: str, body: dict[str, Any]) -> httpx.Response:
        return self.call("POST", "/evaluation/arms/run", "arm_run", tenant, json=body,
                         headers={"Idempotency-Key": body["idempotency_key"]})

    def arm_by_key(self, tenant: str, key: str) -> httpx.Response:
        return self.call("GET", f"/evaluation/arms/by-key/{key}", "arm_read", tenant)

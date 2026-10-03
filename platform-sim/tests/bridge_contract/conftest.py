"""Bridge mock contract fixtures: a real HTTP process (uvicorn) per test session, reset through /_sim between tests."""

from __future__ import annotations

import contextlib
import json
import os
import subprocess
import sys
import time
from pathlib import Path

import httpx
import pytest
from jsonschema import Draft202012Validator
from referencing import Registry, Resource

from bridge_mock import service_jws
from parity.servers import free_port

PLATFORM_SIM = Path(__file__).resolve().parents[2]
SCHEMAS = PLATFORM_SIM / "bridge_mock" / "schemas"


def _registry() -> Registry:
    reg = Registry()
    for p in SCHEMAS.glob("*.schema.json"):
        reg = reg.with_resource(p.name, Resource.from_contents(json.loads(p.read_text(encoding="utf-8"))))
    return reg


REGISTRY = _registry()


def validate(name: str, instance) -> None:
    schema = json.loads((SCHEMAS / f"{name}.schema.json").read_text(encoding="utf-8"))
    Draft202012Validator(schema, registry=REGISTRY).validate(instance)


@pytest.fixture(scope="session")
def base_url():
    port = free_port()
    env = {**os.environ, "PYTHONPATH": str(PLATFORM_SIM), "PYTHONIOENCODING": "utf-8"}
    proc = subprocess.Popen([sys.executable, "-m", "bridge_mock.main", "--port", str(port)], env=env,
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    url = f"http://127.0.0.1:{port}"
    try:
        deadline = time.time() + 30
        while time.time() < deadline:
            if proc.poll() is not None:
                raise RuntimeError("bridge mock exited")
            with contextlib.suppress(httpx.TransportError):
                if httpx.get(f"{url}/_sim/info", timeout=1).status_code == 200:
                    break
            time.sleep(0.2)
        else:
            raise RuntimeError("bridge mock did not start")
        yield url
    finally:
        proc.terminate()
        with contextlib.suppress(subprocess.TimeoutExpired):
            proc.wait(5)
        if proc.stderr:
            proc.stderr.close()


@pytest.fixture()
def client(base_url):
    with httpx.Client(base_url=base_url, timeout=20) as c:
        c.post("/_sim/reset").raise_for_status()
        yield c


class Bridge:
    """Tiny helper around the HTTP client: mints service JWTs and builds valid invocations."""

    def __init__(self, client: httpx.Client) -> None:
        self.c = client

    def token(self, purpose: str = "task_invoke", tenant: str = "tenant-a", **kw) -> str:
        return service_jws.issue(purpose=purpose, tenant_id=tenant, **kw)

    def headers(self, purpose: str = "task_invoke", tenant: str = "tenant-a", key: str | None = None, **kw) -> dict:
        h = {"Authorization": f"Bearer {self.token(purpose, tenant, **kw)}"}
        if key is not None:
            h["Idempotency-Key"] = key
        return h

    def invocation(self, *, stage: str = "scout", tenant: str = "tenant-a", job: str = "job-1", attempt: int = 0,
                   **over) -> dict:
        body = {
            "schema_version": "1", "tenant_id": tenant, "job_id": job, "stage": stage, "attempt": attempt,
            "agent_id": f"pulso-{stage.replace('_', '-')}", "agent_version": "1.0.0", "release_id": "rel-mock-0001",
            "input": {"signal": "x"}, "input_refs": [], "config_ref": "cfg-1", "lab_grant_ref": "grant-1",
            "memory_snapshot_ref": None, "information_partition": "train", "cutoff": "2026-01-01T00:00:00Z",
            "deadline": "2026-01-01T01:00:00Z", "budget_ref": "bud-1",
        }
        body.update(over)
        body.pop("request_digest", None)
        body["request_digest"] = service_jws.request_digest(body)
        return body

    def invoke(self, body: dict | None = None, key: str = "k-1", *, tenant: str = "tenant-a", **kw):
        body = body or self.invocation(tenant=tenant)
        return self.c.post("/internal/v1/core-tasks/invoke", json=body,
                           headers=self.headers("task_invoke", tenant, key, **kw))

    def effects(self) -> dict:
        return self.c.get("/_sim/effects").json()

    def sim(self, path: str, **body):
        r = self.c.post(f"/_sim/{path}", json=body)
        r.raise_for_status()
        return r.json()


@pytest.fixture()
def bridge(client) -> Bridge:
    return Bridge(client)

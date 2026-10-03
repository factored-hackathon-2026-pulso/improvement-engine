"""Worlds: how a target is reached and seeded. A world is the ONLY target-specific piece of the suite."""

from __future__ import annotations

import time
from typing import Any, ClassVar

import httpx

from conformance.kit import CONTRACT, Api, Signer, idempotency_key, request_digest


class World:
    """Base world. `caps` gates the tests that need target-specific seeding:
    invoke (a seeded scout release), writer (frozen candidates), evaluation, control (fault hooks), credentials, arms."""

    target = "external"
    doubles: ClassVar[list[str]] = []
    caps: ClassVar[set[str]] = set()
    base_url = ""
    tenant = "t1"
    other_tenant = "t2"
    unknown_tenant = "t-not-deployed"
    scout_release = "rel-scout"
    writer_release = "rel-writer"
    agent_version = "1.0.0"
    control: Any = None
    arm_agent = "agent-under-test"
    arm_release = "rel-published"
    budget_ref = "bud-1"
    api: Api
    signer: Signer

    def now(self) -> float:
        return time.time()

    def before_scout(self) -> None:
        return None

    def new_candidate(self, tag: str) -> dict[str, Any]:
        raise NotImplementedError("target has no candidate seeding")

    def seal_manifest(self, ref: str) -> None:
        raise NotImplementedError

    def writer_plan(self, tag: str) -> dict[str, Any]:
        raise NotImplementedError("target has no writer seeding")

    def seal_artifact(self, name: str, content: Any) -> None:
        raise NotImplementedError

    def close(self) -> None:
        return None

    # -- DTO builders (real/annex-D shape; a profile may override) --------------------------------------------------
    def invocation(self, stage: str, job: str, logical: str, *, attempt: int = 1, **over: Any) -> dict[str, Any]:
        spec = CONTRACT["stages"][stage]
        release = self.writer_release if stage == "writer" else self.scout_release
        body: dict[str, Any] = {
            "schema_version": "1", "tenant_id": self.tenant, "pulso_run_ref": f"pr-{job}", "job_id": job,
            "stage": stage, "attempt": attempt, "agent_id": spec["agent_id"], "agent_version": self.agent_version,
            "release_id": release, "input": {"briefing_ref": "wiki/briefing.md"} if stage == "scout" else {},
            "input_artifact_refs": [], "lab_grant_ref": "grant-contract", "logical_key": logical}
        if stage == "scout":
            body.update(memory_snapshot_ref="mem-1", extract_manifest_ref="ex-1")
        body.update(over)
        return body

    def invoke(self, body: dict[str, Any], job: str | None = None, *, key: str | None = None,
               with_digest: bool = False, purpose: str = "core_task_invoke", **kw: Any) -> httpx.Response:
        job = job or body["job_id"]
        if with_digest:
            body = {**body, "request_digest": request_digest(body)}
        key = key or idempotency_key(body["tenant_id"], body["job_id"], body["stage"], body["attempt"],
                                     body["logical_key"])
        headers = {"Idempotency-Key": key, **kw.pop("headers", {})}
        return self.api.call("POST", "/core-tasks/invoke", purpose=purpose, body=body, headers=headers, job_id=job, **kw)

    def read_task(self, task_id: str, tenant: str | None = ...) -> httpx.Response:  # type: ignore[assignment]
        return self.api.call("GET", f"/core-tasks/{task_id}", purpose="core_task_read", tenant=tenant)

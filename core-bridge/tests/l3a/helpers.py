from __future__ import annotations

import hashlib
from typing import Any

from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey

from pulso_core_runtime.credentials.issuer import PrincipalSigner
from pulso_core_runtime.invoke.context import InvocationRegistry
from pulso_core_runtime.invoke.service import InvokeService, InvokeSettings
from pulso_core_runtime.store.receipts import ReceiptStore

from .fakes import FakeCore, FakeReleases


def idem_key(tenant: str, job: str, stage: str, attempt: int, logical: str) -> str:
    return hashlib.sha256(f"{tenant}|{job}|{stage}|{attempt}|{logical}".encode()).hexdigest()


def body(tenant: str = "t1", job: str = "j1", stage: str = "scout", attempt: int = 1, logical: str = "k",
         **over: Any) -> dict[str, Any]:
    b: dict[str, Any] = {
        "schema_version": "1", "tenant_id": tenant, "pulso_run_ref": "pr1", "job_id": job, "stage": stage,
        "attempt": attempt, "agent_id": "pulso-scout", "agent_version": "1.0.0", "release_id": "rel-1",
        "input": {"q": "x"}, "input_artifact_refs": [], "lab_grant_ref": "grant1", "logical_key": logical}
    b.update(over)
    return b


def build_service(dsn: str, core: FakeCore | None = None, **kw: Any) -> tuple[InvokeService, FakeCore]:
    core = core or FakeCore()
    settings = kw.pop("settings", InvokeSettings(max_inflight=8))
    svc = InvokeService(
        store=ReceiptStore(dsn), core=core, releases=kw.pop("releases", FakeReleases()),
        signer=PrincipalSigner("id1", Ed25519PrivateKey.generate()), runs=core,
        registry=kw.pop("registry", InvocationRegistry()), settings=settings, **kw)
    return svc, core

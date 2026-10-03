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
    store = ReceiptStore(dsn)
    from pulso_core_runtime.adapters import ReceiptBindingLookup
    from pulso_core_runtime.reconcile.reconciler import Reconciler
    kw.setdefault("reconciler", Reconciler(store=store, runs=core, bindings=ReceiptBindingLookup(store)))
    svc = InvokeService(
        store=store, core=core, releases=kw.pop("releases", FakeReleases()),
        signer=PrincipalSigner("id1", Ed25519PrivateKey.generate()), runs=core,
        registry=kw.pop("registry", InvocationRegistry()), settings=settings, **kw)
    attach_binder(core, svc._registry, svc._store)
    return svc, core


def attach_binder(core: FakeCore, registry: Any, store: ReceiptStore) -> None:
    """The fake Core stands in for the run in which `pulso/bind_context` confirms the binding."""
    from pulso_core_runtime.invoke.context import current_binding

    def bind(run_id: str) -> None:
        ref = current_binding()
        if ref is None:
            return
        ctx = registry.lookup(ref)
        registry.confirm(ref)
        store.transition(ctx.tenant_id, ctx.command_key, "binding_confirmed", core_run_id=run_id)

    core.binder = bind

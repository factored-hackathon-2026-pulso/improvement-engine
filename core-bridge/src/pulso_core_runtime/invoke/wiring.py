"""One-call composition of the L3a handlers for `main._compose` (L2 owns that file).

Needed wiring in `main._compose` (see the L3a report):

    holder: dict[str, Any] = {}
    l3 = build_l3(env, dsn=dsn, registry=ports.registry, app_getter=lambda: holder["app"], tool_registry=...)
    internal = build_internal_app(verifier, version_info=version_info(env), handlers=l3.handlers)
    ...
    app = create_app(deps); holder["app"] = app

`l3.binding` is what the `pulso/bind_context` handler (L3b `tools/bind.py`) calls; `l3.registry` is the
`InvocationRegistry` the dispatcher resolves contexts from."""

from __future__ import annotations

from collections.abc import Callable, Mapping
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from pulso_core_runtime.credentials.issuer import CredentialIssuer, load_signer
from pulso_core_runtime.invoke.binding import BindingService
from pulso_core_runtime.invoke.context import ConfirmingRegistry, InvocationRegistry
from pulso_core_runtime.invoke.core_client import AsgiCoreClient
from pulso_core_runtime.invoke.pin import RegistryReleaseChecker
from pulso_core_runtime.invoke.projection import CatalogProjector, FactProjector
from pulso_core_runtime.invoke.routes import Handler, make_handlers
from pulso_core_runtime.invoke.runs import PgRunReader
from pulso_core_runtime.invoke.service import InvokeService, InvokeSettings
from pulso_core_runtime.reconcile.reconciler import Reconciler
from pulso_core_runtime.store.migrations import apply_l3
from pulso_core_runtime.store.receipts import ReceiptStore

KEYS_DIR = "/run/pulso-keys"


@dataclass
class L3:
    handlers: dict[str, Handler]
    service: InvokeService
    binding: BindingService
    registry: InvocationRegistry
    store: ReceiptStore
    callback_signer: Any = None


def build_l3(env: Mapping[str, str], *, dsn: str, registry: Any, app_getter: Callable[[], Any],
             projector: FactProjector | None = None, reconciler: Reconciler | None = None,
             settings: InvokeSettings | None = None, migrate: bool = True) -> L3:
    """Signer files: `PULSO_BRIDGE_IDENTITY_SIGNER` (run principals, matches `--identity-keys`),
    `PULSO_BRIDGE_STAFF_SIGNER` (registry bot, matches `--staff-keys`), `PULSO_BRIDGE_CALLBACK_SIGNER`
    (control-api service JWTs). Each `{"kid", "key"}`; errors name the file only."""
    if migrate:
        apply_l3(dsn)
    identity = load_signer(Path(env.get("PULSO_BRIDGE_IDENTITY_SIGNER", f"{KEYS_DIR}/bridge-identity.json")))
    staff = load_signer(Path(env.get("PULSO_BRIDGE_STAFF_SIGNER", f"{KEYS_DIR}/bridge-staff.json")))
    callback = load_signer(Path(env.get("PULSO_BRIDGE_CALLBACK_SIGNER", f"{KEYS_DIR}/bridge-callback.json")))
    store = ReceiptStore(dsn)
    runs = PgRunReader(dsn)
    from pulso_core_runtime.stages.catalog import CATALOG

    inv_registry = ConfirmingRegistry(store)  # shared with the L3b dispatcher/tools (`ToolRuntime.contexts`)
    projector = projector or CatalogProjector(contexts=inv_registry)
    cfg = settings or InvokeSettings(
        max_inflight=int(env.get("PULSO_BRIDGE_MAX_INFLIGHT", "8")),
        bridge_instance_id=env.get("PULSO_BRIDGE_INSTANCE", "bridge-1"),
        stage_slots={name: frozenset(spec.input_slots) for name, spec in CATALOG.items()})
    service = InvokeService(
        store=store, core=AsgiCoreClient(app_getter), releases=RegistryReleaseChecker(registry), signer=identity,
        runs=runs, registry=inv_registry, settings=cfg, projector=projector,
        reconciler=reconciler or Reconciler(store=store, runs=runs, projector=projector))
    binding = BindingService(
        store=store, registry=inv_registry, control_api_url=env.get("PULSO_CONTROL_API_URL", ""),
        signing_key=callback._key, kid=callback.kid, bridge_instance_id=env.get("PULSO_BRIDGE_INSTANCE", "bridge-1"))
    issuer = CredentialIssuer({"identity": identity, "staff": staff})
    return L3(make_handlers(service, issuer), service, binding, inv_registry, store, callback)


def install_tools(l3: L3, env: Mapping[str, str], *, builder_factory: Any = None, leak_signal: Any = None) -> Any:
    """Installs the L3b `ToolRuntime` (dispatcher for `factories.tools`) sharing `l3.registry`. Service JWTs:
    `iss=core-bridge`, `aud=control-api` for scope `binding`, `aud=lab-broker` otherwise, TTL 60 s."""
    import time
    import uuid

    from pulso_core_runtime.internal.auth import sign_service_jwt
    from pulso_core_runtime.tools.broker import BrokerClient, ControlApiClient
    from pulso_core_runtime.tools.factory import ToolRuntime, configure

    signer = l3.callback_signer

    def tokens(scope: str) -> str:
        now = int(time.time())
        return sign_service_jwt(signer._key, kid=signer.kid, claims={
            "iss": "core-bridge", "aud": "control-api" if scope == "binding" else "lab-broker",
            "sub": f"bridge:{env.get('PULSO_BRIDGE_INSTANCE', 'bridge-1')}", "scope": scope,
            "purpose": "core_task_binding" if scope == "binding" else "broker", "iat": now, "exp": now + 60,
            "jti": uuid.uuid4().hex})

    runtime = ToolRuntime(l3.registry, BrokerClient(env.get("PULSO_LAB_BROKER_URL", ""), tokens),
                          ControlApiClient(env.get("PULSO_CONTROL_API_URL", ""), tokens), builder_factory, leak_signal)
    configure(runtime)
    return runtime

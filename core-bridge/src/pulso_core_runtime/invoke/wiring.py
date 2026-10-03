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

from pulso_core_runtime.adapters import (
    ReceiptBindingLookup,
    expected_write_keys,
    sealed_commitment_check,
)
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
    executor_signer: Any = None


def build_l3(env: Mapping[str, str], *, dsn: str, registry: Any, app_getter: Callable[[], Any],
             projector: FactProjector | None = None, reconciler: Reconciler | None = None, writes: Any = None,
             settings: InvokeSettings | None = None, migrate: bool = True) -> L3:
    """Signer files: `PULSO_BRIDGE_IDENTITY_SIGNER` (run principals, matches `--identity-keys`),
    `PULSO_BRIDGE_STAFF_SIGNER` (registry bot, matches `--staff-keys`), `PULSO_BRIDGE_CALLBACK_SIGNER`
    (control-api service JWTs). Each `{"kid", "key"}`; errors name the file only."""
    if migrate:
        apply_l3(dsn)
    identity = load_signer(Path(env.get("PULSO_BRIDGE_IDENTITY_SIGNER", f"{KEYS_DIR}/bridge-identity.json")))
    staff = load_signer(Path(env.get("PULSO_BRIDGE_STAFF_SIGNER", f"{KEYS_DIR}/bridge-staff.json")))
    callback = load_signer(Path(env.get("PULSO_BRIDGE_CALLBACK_SIGNER", f"{KEYS_DIR}/bridge-callback.json")))
    executor = load_signer(Path(env.get("PULSO_BRIDGE_EXECUTOR_SIGNER", f"{KEYS_DIR}/bridge-executor.json")))
    if _seed(executor) == _seed(callback) or executor.kid == callback.kid:
        raise ValueError("pulso:credential_signing_unavailable: executor and callback signers must be distinct "
                         "keypairs (A03 iii)")
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
        reconciler=reconciler or Reconciler(
            store=store, runs=runs, projector=projector, writes=writes, bindings=ReceiptBindingLookup(store),
            expected_writes=expected_write_keys(store),
            commitment_check=sealed_commitment_check(store, lambda key: writes.get_write(key) if writes else None)))
    binding = BindingService(
        store=store, registry=inv_registry, control_api_url=env.get("PULSO_CONTROL_API_URL", ""),
        signing_key=callback._key, kid=callback.kid, bridge_instance_id=env.get("PULSO_BRIDGE_INSTANCE", "bridge-1"))
    issuer = CredentialIssuer({"identity": identity, "staff": staff})
    return L3(make_handlers(service, issuer), service, binding, inv_registry, store, callback, executor)


def _seed(signer: Any) -> bytes:
    return bytes(signer._key.private_bytes_raw())


def lab_broker_minter(l3: L3, env: Mapping[str, str]) -> Any:
    """`mint(claims) -> JWS` for the arm-side broker clients (A03 class iii): `iss=core-bridge`, `aud=lab-broker`,
    TTL 60 s, a fresh `jti` per call; the caller supplies `scope, purpose, tenant_id, job_id, binding_ref`."""
    import time
    import uuid

    from pulso_core_runtime.internal.auth import sign_service_jwt

    signer = l3.executor_signer  # A03 iii: the separate executor keypair, never the callback key

    def mint(claims: dict[str, Any]) -> str:
        now = int(time.time())
        body = {k: v for k, v in claims.items() if v is not None}
        return sign_service_jwt(signer._key, kid=signer.kid, claims={
            **body, "iss": "core-bridge", "aud": "lab-broker",
            "sub": f"bridge:{env.get('PULSO_BRIDGE_INSTANCE', 'bridge-1')}",
            "iat": now, "exp": now + 60, "jti": uuid.uuid4().hex})

    return mint


def install_tools(l3: L3, env: Mapping[str, str], *, builder_factory: Any = None, leak_signal: Any = None) -> Any:
    """Installs the L3b `ToolRuntime` (dispatcher for `factories.tools`) sharing `l3.registry`. Service JWTs:
    `iss=core-bridge`, `aud=control-api` for scope `binding`, `aud=lab-broker` otherwise, TTL 60 s."""
    import time
    import uuid

    from pulso_core_runtime.internal.auth import sign_service_jwt
    from pulso_core_runtime.tools.broker import BrokerClient, ControlApiClient
    from pulso_core_runtime.tools.factory import ToolRuntime, configure

    callback, executor = l3.callback_signer, l3.executor_signer
    sub = f"bridge:{env.get('PULSO_BRIDGE_INSTANCE', 'bridge-1')}"

    def tokens(scope: str, claims: dict[str, Any]) -> str:
        """One fresh token per HTTP attempt. `binding` (class ii) is signed by the callback key for
        `aud=control-api`; every lab-broker scope (class iii) by the separate executor key."""
        now = int(time.time())
        signer = callback if scope == "binding" else executor
        return sign_service_jwt(signer._key, kid=signer.kid, claims={
            **claims, "iss": "core-bridge", "aud": "control-api" if scope == "binding" else "lab-broker",
            "sub": sub, "scope": scope, "iat": now, "exp": now + 60, "jti": uuid.uuid4().hex})

    def identity(binding_ref: str) -> tuple[str, str] | None:
        try:
            ic = l3.registry.lookup(binding_ref)
        except Exception:
            return None
        return ic.tenant_id, ic.job_id

    runtime = ToolRuntime(l3.registry, BrokerClient(env.get("PULSO_LAB_BROKER_URL", ""), tokens, identity=identity),
                          ControlApiClient(env.get("PULSO_CONTROL_API_URL", ""), tokens), builder_factory, leak_signal)
    configure(runtime)
    return runtime

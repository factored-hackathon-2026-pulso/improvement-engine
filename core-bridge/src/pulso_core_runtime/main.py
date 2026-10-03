"""Composed runtime entrypoint (plan 17.3.2 `main` sequence). Exit 0 normal, 2 configuration."""

from __future__ import annotations

import argparse
import dataclasses
import os
import sys
from collections.abc import Callable, Mapping, Sequence
from pathlib import Path
from typing import Any, TextIO

from pulso_core_runtime import CONTRACTS_VERSION, PIN_SHA
from pulso_core_runtime.compat import assert_compat
from pulso_core_runtime.errors import (
    EXIT_CONFIG,
    EXIT_OK,
    AdapterMissing,
    DemoDoubleInRealMode,
    RuntimeConfigError,
)
from pulso_core_runtime.factories import DEFAULT_PATHS, FACTORIES, FACTORY_NAMES, STAND_INS

DEMO_ENV = "AGENTCORE_ALLOW_DEMO"
KEYS_DIR = "/run/pulso-keys"


def preflight(env: Mapping[str, str]) -> None:
    """Step 0, before `resolve_ports` (upstream would silently substitute `testing.*` doubles)."""
    if DEMO_ENV in env:
        raise DemoDoubleInRealMode(DEMO_ENV, "demo doubles are never allowed in the real runtime")
    assert_compat()


def factory_paths(env: Mapping[str, str]) -> dict[str, str]:
    """Closed set of seven. An override may repoint a piece (never to `testing`); empty means omitted."""
    paths: dict[str, str] = {}
    for _, name in FACTORIES:
        var = "PULSO_FACTORY_" + name.upper().replace("-", "_")
        path = env.get(var, DEFAULT_PATHS[name])
        if not path.strip():
            raise AdapterMissing(f"factory `{name}`", f"{var} is empty")
        module = path.partition(":")[0]
        if module == "testing" or module.startswith("testing."):
            raise RuntimeConfigError(f"factory `{name}`", "test doubles are rejected")
        paths[name] = path
    return paths


def synthesise_args(env: Mapping[str, str], paths: Mapping[str, str]) -> argparse.Namespace:
    ns: dict[str, Any] = {attr: paths[name] for attr, name in FACTORIES}
    ns.update(
        host=env.get("PULSO_HOST", "0.0.0.0"), port=int(env.get("PULSO_PORT", "8000")), dsn=None,
        agents=None, registry_api=True, eval_dsn=None,
        identity_keys=Path(env.get("PULSO_IDENTITY_KEYS", f"{KEYS_DIR}/identity.json")),
        staff_keys=Path(env.get("PULSO_STAFF_KEYS", f"{KEYS_DIR}/staff.json")))
    return argparse.Namespace(**ns)


def version_info(env: Mapping[str, str]) -> Callable[[], dict[str, Any]]:
    def info() -> dict[str, Any]:
        return {"agent_core_sha": PIN_SHA, "contracts_version": CONTRACTS_VERSION,
                "pulso_sha": env.get("PULSO_SHA", "unknown"), "image_digest": env.get("PULSO_IMAGE_DIGEST", "unknown"),
                "runtime_profile": "agent_core_real",
                "doubles": [f"{k}: {v}" for k, v in sorted(STAND_INS.items())]}
    return info


def run(argv: Sequence[str] | None = None, *, env: Mapping[str, str] | None = None, stderr: TextIO | None = None,
        serve: Callable[..., None] | None = None, resolve: Callable[..., Any] | None = None) -> int:
    env = dict(os.environ if env is None else env)
    err = stderr or sys.stderr
    try:
        preflight(env)
        paths = factory_paths(env)
    except RuntimeConfigError as exc:
        print(f"pulso-core-runtime cannot start: {exc}", file=err)
        return EXIT_CONFIG
    return _compose(env, err, paths, serve, resolve)


def _compose(env: dict[str, str], err: TextIO, paths: dict[str, str], serve: Callable[..., None] | None,
             resolve: Callable[..., Any] | None) -> int:
    from agent_core.composition.observability import ObservabilityConfigError, setup_observability
    from agent_core.composition.serve import GATEWAY_TRACER, build_api_deps
    from agent_core.composition.serve_ports import ServeConfigError, resolve_ports
    from agent_core.composition.serve_registry import build_registry_service_for_serve
    from agent_core.composition.telemetry import OtelTurnTelemetry
    from agent_core.adapters.system_clock import SystemClock
    from agent_core.api.app import create_app
    from agent_core.api.limits import RateLimitConfig

    from pulso_core_runtime.ids import PulsoIds
    from pulso_core_runtime.internal.app import build_internal_app
    from pulso_core_runtime.internal.auth import ServiceJwtVerifier, load_service_keys
    from pulso_core_runtime.internal.store import PgJtiStore
    from pulso_core_runtime.pin import PinnedRegistryPort
    from pulso_core_runtime.readiness import bridge_schema_check, factories_ok_check, key_files_check

    try:
        observability = setup_observability(env)
    except ObservabilityConfigError as exc:
        print("pulso-core-runtime cannot start:", file=err)
        for problem in exc.problems:
            print(f"  - pulso:runtime_config_invalid: {problem}", file=err)
        return EXIT_CONFIG
    try:
        args = synthesise_args(env, paths)
        try:
            ports = (resolve or resolve_ports)(args, env, SystemClock(), PulsoIds(),
                                               tracer=observability.tracer(GATEWAY_TRACER))
        except ServeConfigError as exc:
            print("pulso-core-runtime cannot start:", file=err)
            for problem in exc.problems:
                print(f"  - pulso:runtime_config_invalid: {problem}", file=err)
            return EXIT_CONFIG
        dsn = env.get("AGENTCORE_REGISTRY_DSN", "")
        key_paths = [args.identity_keys, args.staff_keys, Path(env.get("PULSO_SERVICE_KEYS", f"{KEYS_DIR}/service.json"))]
        try:
            service_keys = load_service_keys(key_paths[2])
        except ValueError as exc:
            print(f"pulso-core-runtime cannot start: pulso:runtime_config_invalid: {exc}", file=err)
            return EXIT_CONFIG
        ports = dataclasses.replace(
            ports, registry=PinnedRegistryPort(ports.registry),
            readiness=(*ports.readiness,))
        service = build_registry_service_for_serve(ports)  # L5 swaps in Quotas/PulsoScenarioHarness
        deps = build_api_deps(ports, registry_service=service, telemetry=OtelTurnTelemetry())
        verifier = ServiceJwtVerifier(service_keys, PgJtiStore(dsn))
        internal = build_internal_app(verifier, version_info=version_info(env))

        def internal_extension(app: Any, authenticate: Any) -> None:
            app.mount("/internal/v1", internal)

        extra = (
            ("eval_db", _eval_ping(env)), bridge_schema_check(dsn),
            ("key_files", key_files_check(key_paths)), factories_ok_check(set(FACTORY_NAMES), FACTORY_NAMES),
        )
        deps = dataclasses.replace(
            deps, limits=RateLimitConfig(), extensions=(*deps.extensions, internal_extension),
            readiness=(*deps.readiness, *extra))
        app = create_app(deps)
        if serve is None:
            import uvicorn
            serve = uvicorn.run
        serve(app, host=args.host, port=args.port, log_config=None, access_log=False)
        return EXIT_OK
    finally:
        observability.shutdown()


def _eval_ping(env: Mapping[str, str]) -> Callable[[], bool]:
    from agent_core.adapters.postgres_uow import PostgresStore
    store = PostgresStore(env.get("AGENTCORE_EVAL_DSN", ""))
    return store.ping


def cli() -> None:
    raise SystemExit(run())


if __name__ == "__main__":
    cli()

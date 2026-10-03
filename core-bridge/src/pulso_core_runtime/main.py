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
from pulso_core_runtime.factories import (
    DEFAULT_PATHS,
    FACTORIES,
    FACTORY_NAMES,
    stand_ins,
)

DEMO_ENV = "AGENTCORE_ALLOW_DEMO"
# Literal: `agent_core.composition.serve.GATEWAY_TRACER` was removed upstream (PR #26); the value is unchanged.
LLM_GATEWAY_TRACER = "agent_core.adapters.llm"
# N-09: Core re-reads the key files at most every N seconds (its own default is 5, applied silently when the arg is
# absent). Explicit here so the value is a decision, not an accident; 0 disables.
KEYS_RELOAD_ENV = "PULSO_KEYS_RELOAD_SECONDS"
KEYS_RELOAD_DEFAULT = 5.0
# N-08: Core mounts `/v1/export/*` whenever the store can export and a staff verifier exists (both true here). The
# bridge's own PG exporter is the ingest path (outbox + exact event JSON), so the HTTP export stays OFF unless asked.
EXPORT_ENV = "PULSO_CORE_EXPORT_ENABLED"
KEYS_DIR = "/run/pulso-keys"
# (env var, default file stem under KEYS_DIR): the bridge's own private signers (see `invoke.wiring.build_l3`).
SIGNER_FILES: tuple[tuple[str, str], ...] = (
    ("PULSO_BRIDGE_IDENTITY_SIGNER", "bridge-identity"), ("PULSO_BRIDGE_STAFF_SIGNER", "bridge-staff"),
    ("PULSO_BRIDGE_CALLBACK_SIGNER", "bridge-callback"),
    ("PULSO_BRIDGE_EXECUTOR_SIGNER", "bridge-executor"))


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


def keys_reload_seconds(env: Mapping[str, str]) -> float:
    raw = env.get(KEYS_RELOAD_ENV, "").strip()
    if not raw:
        return KEYS_RELOAD_DEFAULT
    try:
        value = float(raw)
    except ValueError:
        raise RuntimeConfigError(KEYS_RELOAD_ENV, "must be a number of seconds") from None
    if not 0 <= value < 86400:
        raise RuntimeConfigError(KEYS_RELOAD_ENV, "must be between 0 (off) and 86400 seconds")
    return value


def export_enabled(env: Mapping[str, str]) -> bool:
    return env.get(EXPORT_ENV, "").strip().lower() in {"1", "true", "yes"}


def synthesise_args(env: Mapping[str, str], paths: Mapping[str, str]) -> argparse.Namespace:
    ns: dict[str, Any] = {attr: paths[name] for attr, name in FACTORIES}
    ns.update(
        host=env.get("PULSO_HOST", "0.0.0.0"), port=int(env.get("PULSO_PORT", "8000")), dsn=None,
        agents=None, registry_api=True, eval_dsn=None,
        identity_keys=Path(env.get("PULSO_IDENTITY_KEYS", f"{KEYS_DIR}/identity.json")),
        staff_keys=Path(env.get("PULSO_STAFF_KEYS", f"{KEYS_DIR}/staff.json")),
        keys_reload_seconds=keys_reload_seconds(env))
    return argparse.Namespace(**ns)


def version_info(env: Mapping[str, str], doubles: list[str] | None = None,
                 ports: Any = None) -> Callable[[], dict[str, Any]]:
    def info() -> dict[str, Any]:
        # Exception TYPE of the last failed key reload (N-09), never a message; None when healthy or reload is off.
        reload_error = getattr(getattr(ports, "verifier", None), "last_reload_error", None)
        return {"schema_version": "1", "bridge_instance_id": env.get("PULSO_BRIDGE_INSTANCE", "bridge-1"),
                "keys_reload_error": reload_error, "agent_core_sha": PIN_SHA, "contracts_version": CONTRACTS_VERSION,
                "pulso_sha": env.get("PULSO_SHA", "unknown"), "image_digest": env.get("PULSO_IMAGE_DIGEST", "unknown"),
                "runtime_profile": "agent_core_real",
                "doubles": list(doubles if doubles is not None
                                else [f"{k}: {v}" for k, v in sorted(stand_ins(env).items())])}
    return info


def run(argv: Sequence[str] | None = None, *, env: Mapping[str, str] | None = None, stderr: TextIO | None = None,
        serve: Callable[..., None] | None = None, resolve: Callable[..., Any] | None = None,
        llm_probe_client: Any = None) -> int:
    env = dict(os.environ if env is None else env)
    err = stderr or sys.stderr
    try:
        preflight(env)
        paths = factory_paths(env)
    except RuntimeConfigError as exc:
        print(f"pulso-core-runtime cannot start: {exc}", file=err)
        return EXIT_CONFIG
    return _compose(env, err, paths, serve, resolve, llm_probe_client)


class _Lazy:
    """Attribute proxy resolved at first use: breaks the `resolve_ports` -> factories -> L3 -> registry cycle
    (the tool runtime must exist before `resolve_ports`, the registry only exists after it)."""

    def __init__(self, holder: dict[str, Any], key: str) -> None:
        self._holder, self._key = holder, key

    def __getattr__(self, name: str) -> Any:
        try:
            target = self._holder[self._key]
        except KeyError:
            raise RuntimeError(f"runtime piece `{self._key}` is not composed yet") from None
        return getattr(target, name)


def _required_urls(env: Mapping[str, str]) -> list[str]:
    return [f"{var} is empty" for var in ("PULSO_LAB_BROKER_URL", "PULSO_CONTROL_API_URL")
            if not env.get(var, "").strip()]


def _constructor_principal(ic: Any) -> Any:
    """Bot constructor principal of the writer (never the run principal): `pulso-constructor:<tenant>`."""
    from datetime import UTC, datetime, timedelta

    from agent_core.domain.identity import AuthInfo, AuthLevel, Principal, PrincipalType

    now = datetime.now(UTC)
    return Principal(type=PrincipalType.builder, id=f"pulso-constructor:{ic.tenant_id}", roles=["constructor"],
                     attrs={"tenant": ic.tenant_id}, auth=AuthInfo(level=AuthLevel.session, at=now),
                     exp=now + timedelta(minutes=15))


def limits_from_env(env: Mapping[str, str]) -> Any:
    """Core's per-principal limits from AGENTCORE_RATE_* / AGENTCORE_DAILY_BUDGET_USD (agent-core >= 894fa65).
    Raises ValueError on a malformed value. At 789d6c8 (no `rate_limits_from_env`) the defaults apply."""
    from agent_core.api.limits import RateLimitConfig
    from agent_core.composition import serve
    reader = getattr(serve, "rate_limits_from_env", None)
    return reader(env) if reader is not None else RateLimitConfig()


def _path(value: str | None) -> Path | None:
    return Path(value) if value else None


def _wiring_stand_ins(budgets: Any) -> dict[str, str]:
    """Stand-ins decided by the composition itself (not by a factory)."""
    out: dict[str, str] = {}
    out["eval-budgets"] = ("static file resolver (control-api budget contract not defined)" if budgets.configured
                           else "no PULSO_EVAL_BUDGETS file or PULSO_EVAL_BUDGETS_JSON: every budget_ref resolves to None (fails closed)")
    return out


def _fail(err: TextIO, *problems: str) -> int:
    print("pulso-core-runtime cannot start:", file=err)
    for problem in problems:
        print(f"  - pulso:runtime_config_invalid: {problem}", file=err)
    return EXIT_CONFIG


def _compose(env: dict[str, str], err: TextIO, paths: dict[str, str], serve: Callable[..., None] | None,
             resolve: Callable[..., Any] | None, llm_probe_client: Any = None) -> int:
    import psycopg
    from agent_core.adapters.system_clock import SystemClock
    from agent_core.api.app import create_app
    from agent_core.composition.observability import (
        ObservabilityConfigError,
        setup_observability,
    )
    from agent_core.composition.serve import build_api_deps
    from agent_core.composition.serve_ports import ServeConfigError, resolve_ports
    from agent_core.composition.telemetry import OtelTurnTelemetry

    from pulso_core_runtime.adapters import (
        BrokerAuthPort,
        EvalTranscript,
        ServiceWriteProbe,
        SpendMeteringGateway,
        StaticBudgetResolver,
    )
    from pulso_core_runtime.authoring.routes import AuthoringDeps
    from pulso_core_runtime.authoring.routes import register as register_authoring
    from pulso_core_runtime.authoring.service import AuthoringService
    from pulso_core_runtime.evaluation.arms import ArmRunner
    from pulso_core_runtime.evaluation.broker_clients import (
        BrokerArtifactPort,
        BrokerSandboxClient,
    )
    from pulso_core_runtime.evaluation.native import EvaluationGate
    from pulso_core_runtime.evaluation.report import PgArmStore, ensure_eval_schema
    from pulso_core_runtime.evaluation.routes import EvaluationDeps
    from pulso_core_runtime.evaluation.routes import register as register_evaluation
    from pulso_core_runtime.evaluation.targets import TargetLoader
    from pulso_core_runtime.ids import PulsoIds
    from pulso_core_runtime.internal.app import build_internal_app
    from pulso_core_runtime.internal.auth import ServiceJwtVerifier, load_service_keys
    from pulso_core_runtime.internal.store import PgJtiStore, ensure_schema
    from pulso_core_runtime.invoke.wiring import (
        build_l3,
        install_tools,
        lab_broker_minter,
    )
    from pulso_core_runtime.llm.config import llm_doubles, parse_llm_config
    from pulso_core_runtime.llm.probe import llm_gateway_check
    from pulso_core_runtime.pin import PinnedRegistryPort
    from pulso_core_runtime.readiness import (
        bridge_schema_check,
        factories_ok_check,
        key_files_check,
    )
    from pulso_core_runtime.registry_service import (
        FlowEvaluationGate,
        build_evaluation_runtime,
    )
    from pulso_core_runtime.tools.factory import protected_builder_factory
    from pulso_core_runtime.tools.guard import BindingGuardGateway, BindingGuardProvider

    try:
        observability = setup_observability(env)
    except ObservabilityConfigError as exc:
        return _fail(err, *exc.problems)
    try:
        try:
            args = synthesise_args(env, paths)
        except RuntimeConfigError as exc:
            print(f"pulso-core-runtime cannot start: {exc}", file=err)
            return EXIT_CONFIG
        # Fail closed: the runtime has task stages, so no gateway configuration is a startup error (exit 2).
        llm_cfg, llm_problems = parse_llm_config(env)
        if llm_cfg is None:
            return _fail(err, *llm_problems)
        core_sha = env.get("PULSO_CORE_SHA", "").strip()
        if core_sha and core_sha != PIN_SHA:  # Core's `/version` build sha must be the pinned one
            return _fail(err, "PULSO_CORE_SHA must equal the pinned agent-core sha (or be unset)")
        try:
            limits = limits_from_env(env)
        except ValueError as exc:
            return _fail(err, str(exc))
        dsn = env.get("AGENTCORE_REGISTRY_DSN", "")
        eval_dsn = env.get("AGENTCORE_EVAL_DSN", "")
        service_path = Path(env.get("PULSO_SERVICE_KEYS", f"{KEYS_DIR}/service.json"))
        signer_paths = [Path(env.get(var, f"{KEYS_DIR}/{name}.json")) for var, name in SIGNER_FILES]
        key_paths = [args.identity_keys, args.staff_keys, service_path, *signer_paths]
        try:
            service_keys = load_service_keys(service_path)
        except ValueError as exc:
            print(f"pulso-core-runtime cannot start: pulso:runtime_config_invalid: {exc}", file=err)
            return EXIT_CONFIG
        missing = _required_urls(env)
        if missing:
            return _fail(err, *missing)

        holder: dict[str, Any] = {}
        # Bridge schemas + migrations (idempotent, advisory-locked) before anything can take traffic.
        try:
            ensure_schema(dsn)
            ensure_eval_schema(dsn)
            l3 = build_l3(env, dsn=dsn, registry=_Lazy(holder, "registry"), app_getter=lambda: holder["app"],
                          migrate=True, writes=ServiceWriteProbe(lambda: holder["service"]))
        except psycopg.Error as exc:
            return _fail(err, f"bridge database unavailable ({type(exc).__name__})")
        except ValueError as exc:  # signer file unreadable: names the file, never the value
            print(f"pulso-core-runtime cannot start: {exc}", file=err)
            return EXIT_CONFIG
        # The tool runtime must exist BEFORE `resolve_ports` calls `factories.tools`; its builder factory is
        # late-bound (the registry service needs the resolved ports).
        tool_runtime = install_tools(l3, env, builder_factory=lambda ic: holder["builder_factory"](ic))

        try:
            ports = (resolve or resolve_ports)(args, env, SystemClock(), PulsoIds(),
                                               tracer=observability.tracer(LLM_GATEWAY_TRACER))
        except ServeConfigError as exc:
            return _fail(err, *exc.problems)
        pinned = PinnedRegistryPort(ports.registry)
        holder["registry"] = pinned
        ports = dataclasses.replace(ports, registry=pinned)

        # Evaluation composes from the UNGUARDED gateway/providers (synthetic principals, own budget meter);
        # only the live path below is wrapped by the binding guards.
        broker = BrokerAuthPort(tool_runtime.broker)
        try:
            budgets = StaticBudgetResolver(_path(env.get("PULSO_EVAL_BUDGETS")), env.get("PULSO_EVAL_BUDGETS_JSON"))
        except ValueError as exc:
            return _fail(err, str(exc))
        gate = EvaluationGate(permits=int(env.get("PULSO_EVAL_PERMITS", "1")))
        try:
            evaluation = build_evaluation_runtime(dataclasses.replace(ports, transcript=EvalTranscript()),
                                                  runtime_dsn=dsn, eval_dsn=eval_dsn, broker=broker,
                                                  budgets=budgets, gate=gate, ledger=l3.store)
        except ValueError as exc:
            return _fail(err, str(exc))
        holder["service"] = evaluation.service
        mint = lab_broker_minter(l3, env)
        lab_url = env.get("PULSO_LAB_BROKER_URL", "")
        arms = ArmRunner(store=PgArmStore(dsn), broker=broker, artifacts=BrokerArtifactPort(lab_url, mint), budgets=budgets,
                         loader=TargetLoader(ports.registry_api.store), composition=evaluation.composition,
                         gate=evaluation.evaluation_gate, sandbox=BrokerSandboxClient(lab_url, mint),
                         ledger=l3.store)
        holder["builder_factory"] = protected_builder_factory(
            evaluation.service, _constructor_principal, ports.ids, l3.registry, tool_runtime.broker,
            gate=FlowEvaluationGate(evaluation, _constructor_principal), admissions=evaluation.admissions)

        live = dataclasses.replace(
            ports, gateway=BindingGuardGateway(
                SpendMeteringGateway(ports.gateway, l3.registry, l3.store, policy=llm_cfg.policy,
                                     profiles=_profile_lookup(pinned)), l3.registry),
            providers={name: BindingGuardProvider(p, l3.registry) for name, p in ports.providers.items()})
        if not export_enabled(env):
            live = dataclasses.replace(live, run_export=None)  # N-08: no `/v1/export/*` unless explicitly enabled
        # N-04: Core's own `/version` reports this build sha (ours at `/internal/v1/version` also carries digest+doubles).
        deps = build_api_deps(live, registry_service=evaluation.service, telemetry=OtelTurnTelemetry(),
                              build_sha=env.get("PULSO_CORE_SHA") or PIN_SHA)
        handlers: dict[str, Any] = {}
        register_evaluation(handlers, EvaluationDeps(evaluation, arms, broker, budgets))
        # CAP-08 alias reads and CAP-16 L2 dry-run: pure reads over the same store/service Core uses (no quota).
        register_authoring(handlers, AuthoringDeps(AuthoringService(
            ports.registry_api.store, evaluation.service, runtime_profile="agent_core_real")))
        doubles = [f"{k}: {v}" for k, v in sorted({**stand_ins(env), **_wiring_stand_ins(budgets)}.items())]
        doubles += llm_doubles(llm_cfg)
        doubles += [f"core:{name}" for name in ports.doubles]
        verifier = ServiceJwtVerifier(service_keys, PgJtiStore(dsn))
        internal = build_internal_app(verifier, version_info=version_info(env, doubles, ports), handlers=handlers, l3=l3)

        def internal_extension(app: Any, authenticate: Any) -> None:
            app.mount("/internal/v1", internal)

        extra = (
            ("eval_db", _eval_ping(env)), bridge_schema_check(dsn),
            ("key_files", key_files_check(key_paths, verifiers=lambda: [
                getattr(ports, "verifier", None), getattr(getattr(ports, "registry_api", None), "staff_verifier", None)])), factories_ok_check(set(FACTORY_NAMES), FACTORY_NAMES),
            *((llm_gateway_check(llm_cfg.url or "", llm_cfg.token or "", client=llm_probe_client),)
              if llm_cfg.mode == "gateway" else ()),
        )
        deps = dataclasses.replace(
            deps, limits=limits, extensions=(*deps.extensions, internal_extension),
            readiness=(*deps.readiness, *extra))
        app = create_app(deps)
        holder["app"] = app
        app.state.pulso_limits = limits
        app.state.pulso_arms = arms  # introspection handle for composition tests (no secrets, in-process only)
        if serve is None:
            import uvicorn
            serve = uvicorn.run
        serve(app, host=args.host, port=args.port, log_config=None, access_log=False)
        return EXIT_OK
    finally:
        observability.shutdown()


def _profile_lookup(registry: Any) -> Callable[[Any], Any]:
    """`prompt ref -> ModelProfile` the registry will send to the gateway (same resolution as `HttpLLMGateway`)."""
    from agent_core.domain import ModelProfile, Prompt

    def lookup(prompt: Any) -> Any:
        profile_ref = registry.get(prompt, Prompt).model_profile.require_exact()
        return registry.get(profile_ref, ModelProfile)
    return lookup


def _eval_ping(env: Mapping[str, str]) -> Callable[[], bool]:
    from agent_core.adapters.postgres_uow import PostgresStore
    store = PostgresStore(env.get("AGENTCORE_EVAL_DSN", ""))
    return store.ping


def cli() -> None:
    raise SystemExit(run())


if __name__ == "__main__":
    cli()

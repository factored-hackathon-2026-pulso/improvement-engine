"""Closed list of pin symbols the runtime depends on (plan 17.3.2). Drives the contract-drift job.

`assert_compat()` imports every symbol and checks the signature/field names we call with; any drift raises
`PinDrift` (exit 2 `pulso:pin_symbol_drift`) before the process composes anything."""

from __future__ import annotations

import dataclasses
import importlib
import inspect
from typing import Any

from pulso_core_runtime.errors import PinDrift

# (module, name, kind, required names). kind: "func" -> parameter names, "dataclass" -> field names, "class".
PIN_SYMBOLS: tuple[tuple[str, str, str, tuple[str, ...]], ...] = (
    ("agent_core.composition.serve_ports", "resolve_ports", "func", ("args", "env", "clock", "ids", "tracer")),
    ("agent_core.composition.serve_ports", "ServePorts", "dataclass",
     ("clock", "ids", "registry", "releases", "doubles", "readiness", "registry_api", "directory")),
    ("agent_core.composition.serve_ports", "DemoContext", "dataclass", ("clock", "ids", "registry")),
    ("agent_core.composition.serve", "build_api_deps", "func", ("ports", "registry_service", "telemetry")),
    ("agent_core.api.app", "ApiDeps", "dataclass", ("limits", "extensions", "readiness")),
    ("agent_core.api.app", "create_app", "func", ("deps",)),
    ("agent_core.registry.http", "registry_extension", "func", ("service", "verifier", "clock")),
    ("agent_core.registry", "RegistryService", "class", ()),
    ("agent_core.registry", "ScenarioEvaluator", "class", ()),
    ("agent_core.registry", "LocalSandbox", "class", ()),
    ("agent_core.composition.evaluation", "EngineScenarioHarness", "class", ()),
    ("agent_core.composition.evaluation", "EvalStorage", "class", ()),
    ("agent_core.composition.registry", "UowRunReleases", "class", ()),
    ("agent_core.composition.telemetry", "OtelTurnTelemetry", "class", ()),
    ("agent_core.composition.observability", "setup_observability", "func", ("env",)),
    ("agent_core.adapters.postgres_uow", "PostgresStore", "class", ()),
    ("agent_core.registry", "PostgresRegistry", "class", ()),
    ("agent_core.registry", "PgRegistryStore", "class", ()),
    ("agent_core.registry.candidate", "build_candidate", "func",
     ("agent_id", "base", "base_entities", "drafts", "published_hash")),
    ("agent_core.composition.builder_tools", "BuilderToolExecutor", "class", ()),
    ("agent_core.adapters.identity_keys", "load_identity_verifier", "func",
     ("path", "grant_active", "delegation")),
)


def _check(module: str, name: str, kind: str, required: tuple[str, ...]) -> str | None:
    try:
        obj: Any = getattr(importlib.import_module(module), name)
    except (ImportError, AttributeError):
        return f"{module}:{name} missing"
    if kind == "func":
        have = set(inspect.signature(obj).parameters)
    elif kind == "dataclass":
        if not dataclasses.is_dataclass(obj):
            return f"{module}:{name} is not a dataclass"
        have = {f.name for f in dataclasses.fields(obj)}
    else:
        have = set()
        if not inspect.isclass(obj):
            return f"{module}:{name} is not a class"
    missing = sorted(set(required) - have)
    return f"{module}:{name} lacks {', '.join(missing)}" if missing else None


def assert_compat(symbols: tuple[tuple[str, str, str, tuple[str, ...]], ...] = PIN_SYMBOLS) -> None:
    problems = [p for s in symbols if (p := _check(*s))]
    if problems:
        raise PinDrift("pin symbols", "; ".join(problems))

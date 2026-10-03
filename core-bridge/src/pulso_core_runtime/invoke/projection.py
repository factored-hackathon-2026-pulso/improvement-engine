"""`ReadRunResult` projection (CAP-28): only whitelisted facts leave the process. Slots, token_map, actions,
principal, decisions and pages are never read here. The stage whitelist/schemas are owned by L3b
(`facts/whitelist.py`); this module only enforces envelope, caps, digest and number rules."""

from __future__ import annotations

import hashlib
from collections.abc import Mapping
from dataclasses import dataclass
from decimal import Decimal
from typing import Any, Protocol

from agent_core.domain import canonical_bytes

from pulso_core_runtime.invoke.errors import BridgeError

FACT_CAP = 128 * 1024
RESULT_CAP = 256 * 1024


@dataclass(frozen=True)
class FactSpec:
    required: bool = True


class FactProjector(Protocol):
    def project(self, stage: str, core_run_id: str, run: Any | None, *, status: str, outcome: str,
                binding_ref: str | None = None) -> dict[str, Any]:
        ...


def _has_noninteger_number(value: Any) -> bool:
    if isinstance(value, float):
        return True
    if isinstance(value, Decimal):
        exponent = value.as_tuple().exponent
        return isinstance(exponent, int) and exponent < 0  # any fractional literal; decimals travel as strings
    if isinstance(value, Mapping):
        return any(_has_noninteger_number(v) for v in value.values())
    if isinstance(value, list):
        return any(_has_noninteger_number(v) for v in value)
    return False


def _fact_parts(fact: Any) -> tuple[Any, str]:
    if isinstance(fact, Mapping):
        return fact.get("value"), str(fact.get("source_kind", "tool"))
    source = getattr(fact, "source", None)
    return getattr(fact, "value", None), str(getattr(source, "kind", "agent"))


class WhitelistProjector:
    def __init__(self, whitelist: Mapping[str, Mapping[str, FactSpec]]) -> None:
        self._whitelist = whitelist

    def project(self, stage: str, core_run_id: str, run: Any | None, *, status: str, outcome: str,
                binding_ref: str | None = None) -> dict[str, Any]:
        if run is None:
            raise BridgeError("pulso:run_not_found", 404)
        specs = self._whitelist.get(stage, {})
        facts: dict[str, Any] = {}
        total = 0
        for name, spec in specs.items():
            fact = run.facts.get(name)
            if fact is None:
                if spec.required and outcome == "completed":
                    raise BridgeError("pulso:output_missing", 422, details={"fact": name})
                continue
            value, kind = _fact_parts(fact)
            if _has_noninteger_number(value):
                raise BridgeError("pulso:fact_schema_violation", 422, details={"fact": name})
            raw = canonical_bytes(value)
            if len(raw) > FACT_CAP:
                raise BridgeError("pulso:output_too_large", 413, details={"fact": name})
            total += len(raw)
            facts[name] = {"value": value, "source_kind": kind, "digest": hashlib.sha256(raw).hexdigest()}
        if total > RESULT_CAP:
            raise BridgeError("pulso:output_too_large", 413)
        digest = hashlib.sha256(canonical_bytes({k: v["digest"] for k, v in sorted(facts.items())})).hexdigest()
        return {"schema_version": "1", "core_run_id": core_run_id, "status": status, "outcome": outcome,
                "facts": facts, "output_digest": digest}


_FACT_STATUS = {"pulso:output_missing": 422, "pulso:output_too_large": 413, "pulso:fact_schema_violation": 422}


class CatalogProjector:
    """Adapter over L3b `facts.whitelist` (stage whitelist, strict schemas, canaries, writer composition)."""

    def __init__(self, canaries: Any = (), contexts: Any = None) -> None:
        self._canaries, self._contexts = tuple(canaries), contexts

    def project(self, stage: str, core_run_id: str, run: Any | None, *, status: str, outcome: str,
                binding_ref: str | None = None) -> dict[str, Any]:
        from pulso_core_runtime.facts.whitelist import FactError, compose_writer_receipts, project_result
        from pulso_core_runtime.stages.catalog import CATALOG

        if run is None:
            raise BridgeError("pulso:run_not_found", 404)
        try:
            receipts = None
            call_log = None
            if self._contexts is not None and binding_ref:
                try:
                    call_log = self._contexts.seen_artifacts(binding_ref)  # ArtifactRefs in the call log
                except Exception:
                    call_log = None  # context already removed: evidence refs cannot be proven
            if CATALOG[stage].projection and outcome == "completed":
                actions = [a.model_dump(mode="json") if hasattr(a, "model_dump") else a for a in run.actions]
                receipts = compose_writer_receipts(run.facts, actions)
            return project_result(stage, core_run_id=core_run_id, status=status, outcome=outcome,
                                  run_facts=run.facts, canaries=self._canaries, call_log_refs=call_log,
                                  writer_receipts=receipts)
        except FactError as exc:
            raise BridgeError(exc.code, _FACT_STATUS.get(exc.code, 422)) from None

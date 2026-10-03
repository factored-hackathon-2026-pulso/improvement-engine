"""Arms (plan 17.3.5): `POST /internal/v1/evaluation/arms/run`, `GET .../arms/{execution_id}`,
`GET .../arms/by-key/{key}`. Eight steps, status mapping `completed | candidate_failed | failed_infra | unknown`.

Assumption (Pulso proposal, annex D): `ArmRequest`/`ArmReport` field names below follow plan 17.3.5 steps 1-8;
the concrete producer semantics still need Codex confirmation (A05)."""

from __future__ import annotations

import hashlib
import threading
from collections.abc import Callable
from dataclasses import dataclass
from typing import Any, Literal, Protocol

import pydantic
from agent_core.domain import SchemaError
from agent_core.registry import HarnessUnavailable, LocalSandbox, Scenario
from pydantic import BaseModel, ConfigDict, Field

from pulso_core_runtime.evaluation.admission import AdmissionDenied, BrokerPort, valid_context_ref
from pulso_core_runtime.evaluation.budget import BudgetLimits, EvalBudgetMeter
from pulso_core_runtime.evaluation.native import EvalComposition, EvaluationGate
from pulso_core_runtime.evaluation.report import ArmRow, ArmStore, digest_of
from pulso_core_runtime.evaluation.sandbox_port import (
    EvaluationSandboxPort,
    SandboxPortAdapter,
    SandboxTimeout,
    SandboxUnavailable,
)
from pulso_core_runtime.evaluation.targets import TargetError, TargetLoader
from pulso_core_runtime.harness import ManifestEntry

ArmStatus = Literal["completed", "candidate_failed", "failed_infra", "unknown"]


class ArmRequest(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True)

    schema_version: Literal["1"] = "1"
    idempotency_key: str = Field(min_length=1, max_length=200)
    binding_ref: str = Field(min_length=1, max_length=200)
    case_ref: str = Field(min_length=1, max_length=200)
    arm: str = Field(min_length=1, max_length=64)
    repetition: int = Field(ge=0)
    seed: int | str
    mode: Literal["native", "task_builder", "stateful_attention"]
    agent_id: str
    target: dict[str, Any]
    target_commitment: str | None = None
    scenario_manifest_ref: str
    seed_manifest_ref: str | None = None
    oracle_ref: str | None = None  # opaque, echoed, never resolved here
    budget_ref: str
    supersedes_execution_id: str | None = None


class ArtifactPort(Protocol):
    def artifact_get(self, ref: str, tenant_id: str) -> dict[str, Any]:
        """Sealed scenario manifest: {"scenarios": [Scenario JSON...], "entries": {scenario_id: {...}}}."""
        ...


class BudgetPort(Protocol):
    def resolve(self, budget_ref: str, tenant_id: str) -> BudgetLimits | None: ...


def scoped_key(key: str, tenant_id: str) -> str:
    """Single-flight key stored per tenant: `|` is outside the idempotency-key charset, so no collisions."""
    return f"{tenant_id}|{key}"


def execution_id_for(key: str, tenant_id: str) -> str:
    return "arm-" + hashlib.sha256(scoped_key(key, tenant_id).encode()).hexdigest()[:32]


class ArmDenied(Exception):
    def __init__(self, code: str, status: int) -> None:
        super().__init__(code)
        self.code, self.status = code, status


@dataclass
class ArmRunner:
    store: ArmStore
    broker: BrokerPort
    artifacts: ArtifactPort
    budgets: BudgetPort
    loader: TargetLoader
    composition: EvalComposition
    gate: EvaluationGate
    sandbox: EvaluationSandboxPort | None
    inflight: set[str] | None = None

    def __post_init__(self) -> None:
        self.inflight = set()
        self._lock = threading.Lock()

    # -- reads ------------------------------------------------------------------------------------------
    def _settle(self, row: ArmRow) -> ArmRow:
        """A `running` row nobody in this process is executing was interrupted: report `unknown`."""
        assert self.inflight is not None
        if row.status == "running" and row.execution_id not in self.inflight:
            report = {"execution_id": row.execution_id, "status": "unknown", "reason": "interrupted"}
            self.store.finish(row.execution_id, "unknown", report)
            fresh = self.store.get(row.execution_id)
            assert fresh is not None
            return fresh
        return row

    @staticmethod
    def _owned(row: ArmRow | None, tenant_id: str) -> ArmRow | None:
        """A row of another tenant looks exactly like a missing one (no cross-tenant read, no existence oracle)."""
        return row if row is not None and row.idempotency_key.startswith(f"{tenant_id}|") else None

    def read(self, execution_id: str, *, tenant_id: str) -> ArmRow | None:
        row = self._owned(self.store.get(execution_id), tenant_id)
        return None if row is None else self._settle(row)

    def read_by_key(self, key: str, *, tenant_id: str) -> ArmRow | None:
        row = self._owned(self.store.get_by_key(scoped_key(key, tenant_id)), tenant_id)
        return None if row is None else self._settle(row)

    # -- the eight steps ----------------------------------------------------------------------------------
    def run(self, raw: dict[str, Any], *, tenant_id: str) -> ArmRow:
        assert self.inflight is not None
        try:
            req = ArmRequest.model_validate(raw)  # step 1: extra=forbid (no gold/oracle fields can ride along)
        except pydantic.ValidationError as exc:
            raise ArmDenied("invalid_request", 422) from exc
        if req.mode == "native" and req.seed_manifest_ref is not None:  # even "": any bank pointer is a mix
            raise ArmDenied("mixed_world_rejected", 409)  # native never receives a bank client
        if req.mode != "native" and (self.sandbox is None or not req.seed_manifest_ref):
            raise ArmDenied("sandbox_required", 409)
        if not valid_context_ref(req.idempotency_key):
            raise ArmDenied("idempotency_key_invalid", 422)
        if req.supersedes_execution_id is not None:
            old = self.read(req.supersedes_execution_id, tenant_id=tenant_id)
            if old is None or old.status != "unknown":
                raise ArmDenied("supersedes_invalid", 409)  # only a reconciled `unknown` arm can be re-run
        digest = digest_of(req.model_dump(mode="json"))
        execution_id = execution_id_for(req.idempotency_key, tenant_id)  # allocated BEFORE any effect
        row, created = self.store.begin(execution_id, scoped_key(req.idempotency_key, tenant_id), digest)
        if not created:
            if row.request_digest != digest:
                raise ArmDenied("idempotency_conflict", 409)
            return self._settle(row)  # same key + digest: the stored report (or `unknown`), never a re-run
        self.inflight.add(execution_id)
        try:
            report = self._execute(req, execution_id, tenant_id)
        except ArmDenied as exc:  # denied before any effect: terminal, zero spend
            self.store.finish(execution_id, "failed_infra", {"execution_id": execution_id,
                                                             "status": "failed_infra", "reason": exc.code})
            raise
        except BaseException:
            report = {"execution_id": execution_id, "status": "unknown", "reason": "runner_error"}
            self.store.finish(execution_id, "unknown", report)
            raise
        finally:
            self.inflight.discard(execution_id)
        self.store.finish(execution_id, report["status"], report)
        out = self.store.get(execution_id)
        assert out is not None
        return out

    def _execute(self, req: ArmRequest, execution_id: str, tenant_id: str) -> dict[str, Any]:
        base: dict[str, Any] = {
            "case_ref": req.case_ref, "arm": req.arm, "repetition": req.repetition, "seed": req.seed,
            "execution_id": execution_id, "oracle_ref": req.oracle_ref, "event_refs": [],
            "effect_receipts": [], "final_state_ref": None, "initial_state_digest": None, "usage": None,
            "cost_known": False, "trace_id": execution_id, "target_commitment": req.target_commitment}

        def done(status: ArmStatus, **extra: Any) -> dict[str, Any]:
            return {**base, "status": status, **extra}

        try:  # step 2
            allowed = self.broker.check(tenant_id=tenant_id, binding_ref=req.binding_ref, scope="evaluation_arm",
                                        payload_digest=digest_of(req.model_dump(mode="json")))
        except Exception:
            allowed = False
        if not allowed:
            raise ArmDenied("broker_denied", 403)  # zero work; the single-flight row is closed as failed_infra
        try:  # step 3
            loaded = self.loader.load(req.target)
            if req.target_commitment is not None and req.target_commitment != loaded.commitment:
                raise TargetError("commitment_mismatch")
            base["target_commitment"] = loaded.commitment
        except TargetError as exc:
            return done("failed_infra", reason=exc.code, detail=exc.reason)
        except Exception:  # store/transport failure while loading: infrastructure, never a 500
            return done("failed_infra", reason="target_load_failed")
        try:  # step 4: scenarios for the harness only (no gold, no oracle)
            sealed = self.artifacts.artifact_get(req.scenario_manifest_ref, tenant_id)
            scenarios = [Scenario.model_validate(s) for s in sealed["scenarios"]]
            entries = {sid: ManifestEntry(input=e.get("input"), attrs=e.get("attrs", {}), lang=e.get("lang"))
                       for sid, e in sealed.get("entries", {}).items()}
        except Exception:
            return done("failed_infra", reason="manifest_missing")
        if not scenarios:
            return done("failed_infra", reason="manifest_empty")  # zero scenarios run is no evidence at all
        budget = self.budgets.resolve(req.budget_ref, tenant_id)
        if budget is None:
            return done("failed_infra", reason="budget_unknown")
        meter = EvalBudgetMeter(budget)
        adapter: SandboxPortAdapter | None = None
        sandbox: Any = LocalSandbox(self.composition.ids)
        if req.mode != "native":  # step 5
            assert self.sandbox is not None and req.seed_manifest_ref is not None
            adapter = SandboxPortAdapter(self.sandbox, req.binding_ref, req.seed_manifest_ref, execution_id,
                                         self.composition.ids)
            sandbox = adapter
        extra: dict[str, Any] = {}
        if req.mode == "stateful_attention":
            extra["evaluation_binding_ref"] = req.binding_ref
        if req.mode != "native":
            extra["manifest"] = entries
        if (req.mode == "native") != (adapter is None) or (req.mode == "native" and type(sandbox) is not LocalSandbox):
            return done("failed_infra", reason="mixed_world_rejected")  # defence in depth: native has no bank client
        status: ArmStatus = "completed"
        reason: str | None = None
        harness: Any = None
        try:  # step 6: under the evaluation semaphore
            harness = self.composition.harness(req.mode, execution_id=execution_id, arm=req.arm, meter=meter,
                                               tenant_id=tenant_id, **extra)
            with self.gate.slot():
                for scenario in scenarios:
                    handle = sandbox.provision(scenario.seed, loaded.eval_target)
                    try:
                        harness.run(loaded.eval_target, req.agent_id, scenario, sandbox.tools(handle))
                    finally:
                        sandbox.teardown(handle)
        except SandboxTimeout:
            status, reason = "unknown", "sandbox_timeout_after_send"
        except (SandboxUnavailable, HarnessUnavailable) as exc:
            status, reason = "failed_infra", str(exc)[:80] or type(exc).__name__
        except (SchemaError, pydantic.ValidationError):
            status, reason = "candidate_failed", "schema_violation"  # target hash verified: candidate-side fault
        except Exception as exc:  # never a 500; an effect on the bank may have happened -> `unknown`
            effect = adapter is not None and bool(adapter.receipts or adapter.unresolved)
            status, reason = ("unknown" if effect else "failed_infra"), f"unexpected:{type(exc).__name__}"
        if adapter is not None and adapter.unresolved and status == "completed":
            status, reason = "unknown", "action_without_readback"
        runs = harness.runs if harness is not None else []
        early = [f"audit:{r.run_id}" for r in runs if r.closed_early]
        out: dict[str, Any] = {  # step 7
            "closed_early": bool(early), "closed_early_runs": early,  # explicit evidence, never a silent pass
            "event_refs": [f"audit:{r.run_id}" for r in runs],
            "effect_receipts": list(adapter.receipts) if adapter else [],
            "final_state_ref": adapter.final_state_refs[-1] if adapter and adapter.final_state_refs else None,
            "initial_state_digest": adapter.initial_state_digests[0] if adapter and adapter.initial_state_digests
            else None,
            "usage": meter.usage(), "cost_known": meter.cost_known}
        return done(status, reason=reason, **out)  # step 8 persists in `run`


def _unused(_: Callable[..., Any], __: AdmissionDenied) -> None:  # pragma: no cover
    return None

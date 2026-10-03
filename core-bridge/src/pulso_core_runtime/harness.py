"""`PulsoScenarioHarness` (plan 17.3.5, DR-02): one class, three modes chosen by constructor argument.

Differences from the pinned `EngineScenarioHarness` (private upstream; probes copied, not imported):
  * the run `idempotency_key` is unique per job (`eval-<sha256(execution_id|scenario|arm|repetition|nonce)>`)
    instead of the fixed `eval-{scenario.id}`, so a persistent eval DB never replays a stored run;
  * `RunInput.input` and principal attrs come from a sealed manifest in the task modes;
  * budget meter in front of the gateway; every non-candidate failure is mapped to `HarnessUnavailable`;
  * refuses a registry that is not a `SnapshotRegistry` (`AgentSelector(alias="prod")` would otherwise
    resolve the live `prod` alias);
  * events come from the job's own `audit.read(run_id)`; the candidate release id is the constant
    `"candidate"`, so correlation is by `execution_id`/run id, never by release.

Concurrency: all per-evaluation context is passed explicitly at construction (no ContextVar), because
`ScenarioEvaluator` runs jobs on a ThreadPoolExecutor, which does not propagate ContextVars (test_contextvars)."""

from __future__ import annotations

import hashlib
import secrets
import threading
from collections.abc import Callable, Mapping
from dataclasses import dataclass, field
from datetime import timedelta
from typing import Any, Literal

import psycopg
from agent_core.composition import EvalStorage
from agent_core.composition.engine import EngineConfig, EngineDeps, build_turn_engine
from agent_core.decision import DecisionProvider, ProviderError, ProviderTimeout, RawPrediction
from agent_core.domain import (
    AgentSelector,
    ConfirmAnswer,
    EngineError,
    EngineEvent,
    EntityRef,
    GatewayError,
    JsonValue,
    Locale,
    Principal,
    ProblemCode,
    ProviderSpec,
    RunInput,
    TurnInput,
)
from agent_core.ports import GenerationResult
from agent_core.registry import EvalTarget, HarnessUnavailable, Scenario, SnapshotRegistry

from pulso_core_runtime.evaluation.budget import EvalBudgetMeter

Mode = Literal["native", "task_builder", "stateful_attention"]
MODES: tuple[str, ...] = ("native", "task_builder", "stateful_attention")
BUILDER_ROLES = ["constructor"]


@dataclass(frozen=True)
class ManifestEntry:
    """What the sealed manifest holds per scenario id (no gold, no oracle)."""

    input: dict[str, JsonValue] | None = None
    attrs: Mapping[str, str] = field(default_factory=dict)
    lang: str | None = None


@dataclass(frozen=True)
class RunRecord:
    """Evidence of one harness job (feeds `ArmReport.event_refs` and the eval report)."""

    execution_id: str
    mode: str
    label: str
    arm: str
    scenario_id: str
    repetition: int
    key: str
    run_id: str


class _ProbingGateway:
    def __init__(self, inner: Any) -> None:
        self._inner, self.failed = inner, False

    def generate(self, prompt: EntityRef, inputs_model_view: dict[str, JsonValue], locale: Locale,
                 schema: dict[str, JsonValue] | None = None) -> GenerationResult:
        try:
            result: GenerationResult = self._inner.generate(prompt, inputs_model_view, locale, schema)
            return result
        except GatewayError:
            self.failed = True
            raise


class _ProbingProvider:
    def __init__(self, inner: DecisionProvider, failures: list[str]) -> None:
        self._inner, self._failures = inner, failures
        self.name = inner.name

    def predict(self, spec: ProviderSpec, inputs_model_view: dict[str, JsonValue],
                schema: dict[str, JsonValue], locale: Locale) -> RawPrediction:
        try:
            return self._inner.predict(spec, inputs_model_view, schema, locale)
        except (ProviderError, ProviderTimeout):
            self._failures.append(self.name)
            raise


def run_key(execution_id: str, scenario_id: str, arm: str, repetition: int, nonce: str) -> str:
    raw = "|".join((execution_id, scenario_id, arm, str(repetition), nonce))
    return "eval-" + hashlib.sha256(raw.encode()).hexdigest()


class PulsoScenarioHarness:
    def __init__(self, *, mode: Mode, clock: Any, ids: Any, keys: Any, gateway: Any,
                 providers: Callable[[str], Mapping[str, DecisionProvider]], calibrations: Any,
                 authz: Any, storage: Callable[[], EvalStorage], classifier: Any = None,
                 config: EngineConfig | None = None, execution_id: str = "native", arm: str | None = None,
                 tenant_id: str = "pulso", manifest: Mapping[str, ManifestEntry] | None = None,
                 evaluation_binding_ref: str | None = None, meter: EvalBudgetMeter | None = None,
                 nonce: str | None = None) -> None:
        if mode not in MODES:
            raise ValueError(f"unknown harness mode {mode!r}")
        if mode == "stateful_attention" and not evaluation_binding_ref:
            raise ValueError("stateful_attention needs evaluation_binding_ref")
        if mode != "native" and manifest is None:
            raise ValueError(f"{mode} needs the sealed manifest")
        self._mode: Mode = mode
        self._clock, self._ids, self._keys = clock, ids, keys
        self._meter = meter
        self._gateway = meter.wrap(gateway) if meter is not None else gateway
        self._providers, self._calibrations, self._authz = providers, calibrations, authz
        self._storage, self._classifier = storage, classifier
        self._config = config or EngineConfig()
        self._execution_id, self._arm, self._tenant = execution_id, arm, tenant_id
        self._manifest = dict(manifest or {})
        self._binding_ref = evaluation_binding_ref
        self._nonce = nonce or secrets.token_hex(16)  # per instance: two instances never share keys
        self._lock = threading.Lock()
        self._counters: dict[tuple[str, str], int] = {}
        self.runs: list[RunRecord] = []

    @property
    def mode(self) -> Mode:
        return self._mode

    def _principal(self, scenario: Scenario, level: str, entry: ManifestEntry | None) -> Principal:
        now = self._clock.now()
        auth = {"level": level, "at": now}
        if self._mode == "task_builder":
            attrs = dict(entry.attrs) if entry else {}
            return Principal.model_validate({
                "type": "builder", "id": f"builder:pulso-constructor:{self._tenant}", "roles": BUILDER_ROLES,
                "attrs": attrs, "auth": {"level": "step_up", "at": now}, "exp": now + timedelta(hours=1)})
        attrs = {**scenario.principal.attrs, **(dict(entry.attrs) if entry else {})}
        if self._mode == "stateful_attention":
            attrs["evaluation_binding_ref"] = self._binding_ref or ""
        return Principal.model_validate({
            "type": "customer", "id": scenario.principal.id, "attrs": attrs, "auth": auth,
            "exp": now + timedelta(hours=1)})

    def _next_repetition(self, label: str, scenario_id: str) -> int:
        with self._lock:
            n = self._counters.get((label, scenario_id), 0)
            self._counters[(label, scenario_id)] = n + 1
            return n

    def run(self, target: EvalTarget, agent_id: str, scenario: Scenario, tools: Any) -> list[EngineEvent]:
        if not isinstance(target.registry, SnapshotRegistry):
            raise HarnessUnavailable("target_registry_not_snapshot")
        entry: ManifestEntry | None = None
        if self._mode != "native":
            entry = self._manifest.get(scenario.id)
            if entry is None:
                raise HarnessUnavailable("manifest_missing")
        if self._meter is not None:
            self._meter.start_job()
        repetition = self._next_repetition(target.label, scenario.id)
        arm = self._arm or target.label
        key = run_key(self._execution_id, scenario.id, arm, repetition, self._nonce)
        try:
            run_id, events = self._run(target, agent_id, scenario, tools, entry, key)
        except HarnessUnavailable:
            raise
        except (OSError, TimeoutError, psycopg.Error) as exc:
            raise HarnessUnavailable("infra_" + type(exc).__name__) from None
        with self._lock:
            self.runs.append(RunRecord(self._execution_id, self._mode, target.label, arm, scenario.id,
                                       repetition, key, run_id))
        return events

    def _run(self, target: EvalTarget, agent_id: str, scenario: Scenario, tools: Any,
             entry: ManifestEntry | None, key: str) -> tuple[str, list[EngineEvent]]:
        storage = self._storage()
        probe = _ProbingGateway(self._gateway)
        provider_failures: list[str] = []
        providers = {name: _ProbingProvider(p, provider_failures)
                     for name, p in self._providers(scenario.id).items()}
        engine = build_turn_engine(EngineDeps(
            clock=self._clock, ids=self._ids, keys=self._keys, uow_factory=storage.uow_factory,
            audit=storage.audit, registry=target.registry, releases=lambda _rid: target.release, tools=tools,
            gateway=probe, providers=providers, calibrations=self._calibrations,
            transcript=storage.transcript, authz=self._authz, classifier=self._classifier,
            config=self._config))
        run_id: str | None = None
        session_id: str | None = None
        token: str | None = None
        for n, step in enumerate(scenario.steps):
            principal = self._principal(scenario, step.auth, entry)
            if step.op == "start":
                data: dict[str, Any] = {"agent": AgentSelector(id=agent_id, alias="prod"),
                                        "idempotency_key": key}
                lang = step.lang or (entry.lang if entry else None)
                if lang is not None:
                    data["lang"] = lang
                if entry is not None and entry.input is not None:
                    data["input"] = dict(entry.input)
                result = engine.start_run(principal, None, RunInput.model_validate(data))
                run_id, session_id, turn = result.run_id, result.session_id, result.first_turn
            else:
                if session_id is None:
                    break  # task mode: the run already ended at start
                confirm = (ConfirmAnswer(token=token or "", answer=step.answer)  # type: ignore[arg-type]
                           if step.op == "confirm" else None)
                try:
                    turn = engine.handle_turn(principal, None, TurnInput(
                        session_id=session_id, text=step.text or "", channel="web", client_turn_id=f"c-{n}",
                        confirm=confirm))
                except EngineError as exc:
                    if exc.code is ProblemCode.run_closed:
                        break  # the candidate closed the run early (e.g. escalated): events are the evidence
                    raise
            token = turn.confirmation.token if turn is not None and turn.confirmation is not None else None
        if self._meter is not None:
            self._meter.check_after_run()
        if probe.failed:
            raise HarnessUnavailable("gateway_failed")
        if provider_failures:
            raise HarnessUnavailable("decision_provider_failed")
        if run_id is None:
            raise HarnessUnavailable("no_run_started")
        return run_id, storage.audit.read(run_id)


class EvalAuthz:
    """Authorisation for synthetic evaluation principals only (never wired to live traffic).

    Customers are the scenario principals of native / stateful modes; builders are the constructor bot of
    `task_builder`. No subject/OBO, no field reads (fail closed), no param binding."""

    REPORTABLE = frozenset({"country", "stage", "pin_release_id"})

    def authorize_agent(self, principal: Any, agent: Any, subject: Any) -> Any:
        from agent_core.ports import AuthzDecision

        ok = principal.type.value in ("customer", "builder")
        return AuthzDecision(allowed=ok, reason=None if ok else "principal_type")

    def authorize_subject(self, principal: Any, obo: Any, subject: Any) -> Any:
        from agent_core.ports import AuthzDecision

        if subject is None and obo is None:
            return AuthzDecision(allowed=True)
        return AuthzDecision(allowed=False, reason="subject_not_supported")

    def bind_params(self, principal: Any, obo: Any, subject: Any) -> dict[str, str]:
        return {}

    def can_read_field(self, reader: Any, obo: Any, field: str, purpose: str) -> bool:
        return False

    def knowledge_view(self, principal: Any, purpose: Any) -> Any:
        from agent_core.domain.knowledge import KnowledgeView

        return KnowledgeView(audiences=frozenset(), approved_only=True)

    def reportable_attrs(self) -> frozenset[str]:
        return self.REPORTABLE

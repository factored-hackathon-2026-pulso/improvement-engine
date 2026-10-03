"""The seven `module:attr` factories passed to `resolve_ports` (called as `fn(DemoContext)`).

Core lists none of them in `ServePorts.doubles` outside demo, so stand-ins are reported by us through
`STAND_INS` (-> `/internal/v1/version.doubles[]`). Every factory is fail-closed."""

from __future__ import annotations

import os
from collections.abc import Callable
from datetime import datetime
from pathlib import Path

from agent_core.composition.serve_ports import DemoContext
from agent_core.decision.calibration.artifact import CalibrationArtifact, DirectoryCalibrationSource
from agent_core.decision.providers.classifier import ClassifierProvider
from agent_core.decision.types import ProviderError
from agent_core.domain import EntityRef, JsonValue, ToolDef
from agent_core.domain.entities import Agent
from agent_core.domain.identity import OnBehalfOf, Principal, PrincipalType, SubjectRef
from agent_core.domain.knowledge import KnowledgeView, Purpose
from agent_core.domain.shared import ToolStatus, TranscriptEntry
from agent_core.ports import AuthzDecision, ToolCallContext, ToolResult
from agent_core.ports.ids import IdKind
from agent_core.views import FieldClassifier
from agent_core.views.classification import FieldRule

# (factory attribute, public name). The closed set of seven pieces.
FACTORIES: tuple[tuple[str, str], ...] = (
    ("tools", "tools"), ("authz", "authz"), ("transcript", "transcript"), ("calibration", "calibration"),
    ("classifier", "classifier"), ("field_classifier", "field-classifier"), ("grant_active", "grant-active"),
)
FACTORY_NAMES: tuple[str, ...] = tuple(name for _, name in FACTORIES)
DEFAULT_PATHS: dict[str, str] = {name: f"pulso_core_runtime.factories:{attr}" for attr, name in FACTORIES}

# Pieces that are stand-ins in this build (reported in version.doubles[]). L3 removes `tools` when its
# `pulso/*` handlers land.
STAND_INS: dict[str, str] = {"tools": "no pulso/* handlers registered (L3)"}

BUILDER_ROLES = frozenset({"constructor", "aprobador"})
REPORTABLE_ATTRS = frozenset({"stage", "pin_release_id"})

ToolHandler = Callable[[dict[str, JsonValue], ToolCallContext], ToolResult]


class PulsoToolDispatcher:
    """Dispatch by `tool.id@version` to registered `pulso/*` handlers; unknown -> `error unregistered_tool`."""

    def __init__(self, ctx: DemoContext, handlers: dict[str, ToolHandler] | None = None) -> None:
        self._registry, self._ids = ctx.registry, ctx.ids
        self._handlers = dict(handlers or {})

    def register(self, tool_key: str, handler: ToolHandler) -> None:
        self._handlers[tool_key] = handler

    def execute(self, tool: EntityRef, args: dict[str, JsonValue], bound_params: dict[str, str],
                ctx: ToolCallContext, idempotency_key: str | None = None) -> ToolResult:
        handler = self._handlers.get(f"{tool.id}@{tool.version}")
        if handler is None:
            return ToolResult(status=ToolStatus.error, error="unregistered_tool",
                              call_id=self._ids.new_id(IdKind.call))
        return handler(args, ctx)

    def definition(self, tool: EntityRef) -> ToolDef:
        return self._registry.get(tool, ToolDef)


class PulsoAuthz:
    """Admits builder principals carrying a stage role; never a subject; closed reportable attrs."""

    def authorize_agent(self, principal: Principal, agent: Agent, subject: SubjectRef | None) -> AuthzDecision:
        if principal.type is not PrincipalType.builder:
            return AuthzDecision(allowed=False, reason="principal_type")
        if not BUILDER_ROLES.intersection(principal.roles):
            return AuthzDecision(allowed=False, reason="role")
        return AuthzDecision(allowed=True)

    def authorize_subject(self, principal: Principal, obo: OnBehalfOf | None,
                          subject: SubjectRef | None) -> AuthzDecision:
        if subject is None and obo is None:
            return AuthzDecision(allowed=True)
        return AuthzDecision(allowed=False, reason="subject_not_supported")

    def bind_params(self, principal: Principal, obo: OnBehalfOf | None,
                    subject: SubjectRef | None) -> dict[str, str]:
        return {}

    def can_read_field(self, reader: Principal, obo: OnBehalfOf | None, field: str, purpose: str) -> bool:
        return False

    def knowledge_view(self, principal: Principal, purpose: Purpose) -> KnowledgeView:
        return KnowledgeView(audiences=frozenset(), approved_only=True)

    def reportable_attrs(self) -> frozenset[str]:
        return REPORTABLE_ATTRS


class TranscriptRejected(RuntimeError):
    """Conversational transcripts are not supported by task-only runtimes."""


class NullTranscript:
    def append(self, entry: TranscriptEntry) -> str:
        raise TranscriptRejected("conversational_not_supported")

    def read(self, run_id: str) -> list[TranscriptEntry]:
        return []

    def recent_turns(self, run_id: str, n: int) -> list[TranscriptEntry]:
        return []


class PulsoCalibrations:
    """`DirectoryCalibrationSource` on the assets dir; a missing directory or file -> None (live path blocked)."""

    def __init__(self, path: Path) -> None:
        self._inner = DirectoryCalibrationSource(path)
        self._path = path

    def get(self, run_id: str) -> CalibrationArtifact | None:
        if not self._path.is_dir():
            return None
        return self._inner.get(run_id)


class _AssetLoader:
    def __init__(self, root: Path) -> None:
        self._root = root

    def load(self, ref: str) -> str:
        if not ref or ".." in ref or ref.startswith(("/", "\\")) or ":" in ref:
            raise ProviderError("classifier: invalid artifact ref")
        file = self._root / ref
        if not file.is_file():
            raise ProviderError("classifier: artifact absent")
        return file.read_text(encoding="utf-8")


def _env_dir(var: str, default: str) -> Path:
    return Path(os.environ.get(var) or default)


# Lab catalogue (closed). An uncatalogued column is not given a class here: lookups return None and the
# lab tools map it to `unclassified_column`. Core's default (`pii_direct` tokenising) is TBV.
LAB_CATALOG: dict[str, FieldRule] = {
    "event_count": FieldRule(field_class="public"),
    "metric_value": FieldRule(field_class="public"),
    "amount": FieldRule(field_class="financial"),
    "free_text": FieldRule(field_class="untrusted_text"),
}


def tools(ctx: DemoContext) -> PulsoToolDispatcher:
    return PulsoToolDispatcher(ctx)


def authz(ctx: DemoContext) -> PulsoAuthz:
    return PulsoAuthz()


def transcript(ctx: DemoContext) -> NullTranscript:
    return NullTranscript()


def calibration(ctx: DemoContext) -> PulsoCalibrations:
    return PulsoCalibrations(_env_dir("PULSO_CALIBRATIONS_DIR", "/opt/pulso/assets/calibrations"))


def classifier(ctx: DemoContext) -> ClassifierProvider:
    return ClassifierProvider(_AssetLoader(_env_dir("PULSO_CLASSIFIER_DIR", "/opt/pulso/assets/classifiers")))


def field_classifier(ctx: DemoContext) -> FieldClassifier:
    return FieldClassifier(LAB_CATALOG)


def grant_active(ctx: DemoContext) -> Callable[[str, datetime], bool]:
    return lambda ref, now: False

"""Shared synthetic world for L5 tests: real engine, scripted decision providers, in-memory stand-ins.

DOUBLES (declared in every report): FakeClock, ScriptedProvider(jev/classifier), CitingGateway, SyntheticAuthz,
FakeKeyProvider, InMemoryTranscript. The registry, the stores and Postgres are real."""

from __future__ import annotations

from collections.abc import Mapping
from pathlib import Path
from typing import Any

import agent_core
from agent_core.adapters.system_ids import SystemIds
from agent_core.composition import EvalStorage
from agent_core.decision import DecisionProvider
from agent_core.decision.calibration.artifact import InMemoryCalibrationSource
from agent_core.registry import EvalSuite
from agent_core.views import FieldClassifier
from testing.engine_world import CATALOG, CitingGateway, SyntheticAuthz, demo_calibration
from testing.fakes.clock import FakeClock
from testing.fakes.keys import FakeKeyProvider
from testing.fakes.provider import ScriptedProvider
from testing.fakes.transcript import InMemoryTranscript
from testing.registry_demo import DEMO_SCRIPTS, demo_suite

REGISTRY_DEMO = Path(agent_core.__file__).parents[1] / "tests" / "fixtures" / "registry-demo"


def demo_pinned() -> Any:
    from agent_core.flows import load_registry, pin_release

    reg, violations = load_registry(REGISTRY_DEMO)
    assert not violations
    return pin_release(reg, "demo")


DOUBLES = ["FakeClock", "ScriptedProvider", "CitingGateway", "SyntheticAuthz", "FakeKeyProvider",
           "InMemoryTranscript"]


def suite_with(repetitions: int = 1, version: str = "1.0.0") -> EvalSuite:
    data: dict[str, Any] = demo_suite().model_dump(mode="json")
    data["repetitions"] = repetitions
    data["version"] = version
    return EvalSuite.model_validate(data)


def providers_for(clock: FakeClock) -> Any:
    def providers(scenario_id: str) -> Mapping[str, DecisionProvider]:
        jev, classifier = ScriptedProvider("jev", clock=clock), ScriptedProvider("classifier", clock=clock)
        script = DEMO_SCRIPTS.get(scenario_id)
        if script is not None:
            script(jev, classifier)
        return {"jev": jev, "classifier": classifier}
    return providers


def common_kwargs(eval_dsn: str | None = None, gateway: Any = None) -> dict[str, Any]:
    """Keyword args shared by the stock harness and `PulsoScenarioHarness`."""
    from agent_core.adapters.postgres_uow import PostgresStore
    from testing.fakes.storage import InMemoryAuditSink, InMemoryStore

    clock = FakeClock()
    if eval_dsn is not None:
        store = PostgresStore(eval_dsn)
        audit = store.audit()

        def storage() -> EvalStorage:
            return EvalStorage(uow_factory=store.uow, audit=audit, transcript=InMemoryTranscript())
    else:
        def storage() -> EvalStorage:
            mem = InMemoryStore()
            return EvalStorage(uow_factory=mem.uow, audit=InMemoryAuditSink(mem), transcript=InMemoryTranscript())
    return dict(
        clock=clock, ids=SystemIds(), keys=FakeKeyProvider.default(), gateway=gateway or CitingGateway(),
        providers=providers_for(clock), calibrations=InMemoryCalibrationSource({"cal-demo": demo_calibration()}),
        authz=SyntheticAuthz(), storage=storage, classifier=FieldClassifier(CATALOG))


# --- registry world on a real Postgres runtime DB ---------------------------------------------------------

AGENT = "atencion"


def bot_actor() -> Any:
    from testing.builders import principal

    return principal(type="builder", id="builder:pulso-constructor:t1", roles=["constructor"], attrs={})


def _docs(text: str = "cambio de prueba") -> Any:
    from agent_core.registry.models import VersionDocs

    return VersionDocs(description=text, rationale="mejorar", changelog=text)


def prompt_draft(version: str = "1.1.0") -> Any:
    from agent_core.registry.models import EntityDraft

    return EntityDraft(kind="prompt", content={
        "id": "p/resumen_radicado", "version": version,
        "locales": {"es": "Confirma en una frase que la disputa quedo radicada.",
                    "pt": "Confirme em uma frase que a contestacao foi registrada."},
        "model_profile": "perfil-generacion@1.0.0"}, docs=_docs())


def suite_draft(version: str = "1.0.0", outcome: str = "resolved", repetitions: int = 1) -> Any:
    from agent_core.registry.models import EntityDraft

    data = suite_with(repetitions, version).model_dump(mode="json")
    data["scenarios"][0]["expect"]["outcome"] = outcome
    return EntityDraft(kind="eval_suite", content=data, docs=_docs("suite"))


def seed_demo(store: Any) -> str:
    """Seeds the demo release as `staging`/`prod` base (setup only: counted in the baseline)."""
    from agent_core.domain import EntityKind
    from agent_core.registry.candidate import release_hash
    from agent_core.registry.entities import content_hash, encode_entity, version_ref
    from agent_core.registry.models import AliasChange, StoredRelease, StoredVersion, VersionDocs
    from testing.builders import NOW

    pinned = demo_pinned()
    docs = VersionDocs(description="seed", rationale="", changelog="")
    with store.transaction() as tx:
        refs = []
        for entity in pinned.entities:
            tx.blobs.put(encode_entity(entity))
            ref = version_ref(entity)
            refs.append(ref)
            tx.insert_version(StoredVersion(ref=ref, content_hash=content_hash(entity), docs=docs,
                                            proposal_id=None, created_by="seed", created_at=NOW))
        release = pinned.release.model_copy(update={"id": "rel-demo"})
        tx.insert_release(StoredRelease(release=release, release_hash=release_hash(release), agent_id=AGENT,
                                        agent_version=release.entities[EntityKind.agent][AGENT],
                                        base_release_id=None, proposal_id=None, published_by="seed",
                                        published_at=NOW), refs)
        for alias in ("staging", "prod"):
            tx.set_alias(AliasChange(agent_id=AGENT, alias=alias, before=None, after="rel-demo", actor="seed",
                                     reason="seed", at=NOW))
    return "rel-demo"


def operative_counts(dsn: str) -> dict[str, int]:
    """Row counts of every runtime table that must stay untouched by an evaluation (plus the ones allowed)."""
    import psycopg

    tables = ["reg_blobs", "reg_entity_versions", "reg_releases", "reg_release_entities", "reg_aliases",
              "reg_alias_log", "reg_approvals", "reg_publish_keys", "runs", "audit_events", "usage",
              "reg_eval_runs", "reg_events", "reg_draft_writes", "reg_proposals"]
    out: dict[str, int] = {}
    with psycopg.connect(dsn, autocommit=True) as conn:
        for t in tables:
            out[t] = conn.execute(f"SELECT count(*) FROM {t}").fetchone()[0]  # type: ignore[index,misc]
    return out

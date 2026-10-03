"""Arms: targets, eight steps, status mapping, single-flight, mixed-world, unpublished candidate (real PG16).

Doubles: FakeBank (EvaluationSandboxPort), FakeArtifacts (sealed manifest), FakeBroker, FixedBudgets."""

from __future__ import annotations

import copy
from dataclasses import dataclass, field
from typing import Any

import pytest
from agent_core.domain import AgentSelector, EntityRef, ToolDef
from agent_core.ports import ToolCallContext, ToolStatus
from agent_core.registry import PostgresRegistry
from testing.builders import principal
from testing.fakes.clock import FakeClock

from l5.test_evaluate_path import FakeBroker, FixedBudgets, World
from l5.world import AGENT, operative_counts, suite_with
from pulso_core_runtime.evaluation.arms import ArmDenied, ArmRunner, execution_id_for
from pulso_core_runtime.evaluation.report import InMemoryArmStore, PgArmStore
from pulso_core_runtime.evaluation.sandbox_port import (
    ActionResult,
    SandboxPortAdapter,
    SandboxSession,
    SandboxTimeout,
    StatefulSandboxToolExecutor,
)
from pulso_core_runtime.evaluation.targets import TargetError, TargetLoader, drafts_digest

pytestmark = [pytest.mark.runtime, pytest.mark.pg]


@dataclass
class FakeBank:
    opened: int = 0
    closed: int = 0
    fail_open: Exception | None = None
    fail_act: bool = False
    actions: dict[str, ActionResult] = field(default_factory=dict)
    contexts: list[Any] = field(default_factory=list)

    def open(self, binding_ref: str, seed_manifest_ref: str, context: Any = None) -> SandboxSession:
        self.contexts.append(context)
        if self.fail_open:
            raise self.fail_open
        self.opened += 1
        return SandboxSession(f"s{self.opened}", 0, "sha256:initial")

    def reset(self, session: SandboxSession) -> SandboxSession:
        return session

    def act(self, session: SandboxSession, action_key: str, expected_revision: int, action: Any) -> ActionResult:
        res = ActionResult(expected_revision + 1, f"rcpt-{action_key[:8]}", {"ok": True}, "sha256:s")
        self.actions[action_key] = res  # the bank applied it even when the answer is lost
        if self.fail_act:
            raise SandboxTimeout()
        return res

    def readback(self, session: SandboxSession, action_key: str) -> ActionResult | None:
        return self.actions.get(action_key)

    def close(self, session: SandboxSession, reason: str) -> str:
        self.closed += 1
        return f"final:{session.session_ref}"


class FakeArtifacts:
    def __init__(self, scenarios: list[dict[str, Any]] | None = None) -> None:
        self.scenarios = scenarios if scenarios is not None else [suite_with().scenarios[0].model_dump(mode="json")]
        self.fail = False
        self.calls: list[tuple[str, str, str]] = []

    def artifact_get(self, ref: str, tenant_id: str, binding_ref: str) -> dict[str, Any]:
        self.calls.append((ref, tenant_id, binding_ref))
        if self.fail or ref == "art-missing":
            raise KeyError(ref)
        return {"scenarios": copy.deepcopy(self.scenarios), "entries": {s["id"]: {} for s in self.scenarios}}


def runner(w: World, bank: FakeBank | None = None, store: Any = None, artifacts: Any = None) -> ArmRunner:
    return ArmRunner(store=store or PgArmStore(w.pg.runtime), broker=w.broker, artifacts=artifacts or FakeArtifacts(),
                     budgets=FixedBudgets(), loader=TargetLoader(w.store), composition=w.rt.composition,
                     gate=w.rt.evaluation_gate, sandbox=bank)


def req(key: str = "k1", **over: Any) -> dict[str, Any]:
    base: dict[str, Any] = dict(
        idempotency_key=key, binding_ref="bind-1", case_ref="case-1", arm="base", repetition=0, seed=7,
        mode="native", agent_id=AGENT, target={"kind": "published_release", "release_id": "rel-demo"},
        scenario_manifest_ref="art-1", budget_ref="bud-1", oracle_ref="oracle-opaque")
    return {**base, **over}


def frozen_target(w: World) -> tuple[dict[str, Any], str]:
    pid, chash = w.frozen_proposal()
    detail = w.rt.service.get_proposal(pid)
    return ({"kind": "frozen_candidate", "proposal_id": pid, "expected_rev": detail.proposal.rev,
             "base_release_id": detail.proposal.base_release_id, "candidate_hash": chash,
             "draft_plan_ref": "plan-1", "draft_plan_digest": drafts_digest(detail.changes)}, pid)


def test_native_arm_on_published_release_completes_and_replays_by_key(pg) -> None:  # type: ignore[no-untyped-def]
    w = World(pg)
    r = runner(w)
    row = r.run(req(), tenant_id="t1")
    rep = row.report or {}
    assert rep["status"] == "completed" and rep["execution_id"] == execution_id_for("k1", "t1")
    assert rep["event_refs"] and rep["oracle_ref"] == "oracle-opaque" and rep["cost_known"] is True
    assert rep["target_commitment"] and rep["effect_receipts"] == []
    jobs = w.storage.jobs
    again = r.run(req(), tenant_id="t1")  # same key + digest: stored report, no re-run
    assert again.report == rep and w.storage.jobs == jobs
    with pytest.raises(ArmDenied) as conflict:
        r.run(req(seed=8), tenant_id="t1")  # same key, other digest
    assert conflict.value.code == "idempotency_conflict" and conflict.value.status == 409
    assert r.read(execution_id_for("k1", "t1"), tenant_id="t1").report == rep  # type: ignore[union-attr]
    assert r.read_by_key("k1", tenant_id="t1").report == rep  # type: ignore[union-attr]


def test_frozen_candidate_runs_without_publication_and_prod_never_resolves_it(pg) -> None:  # type: ignore[no-untyped-def]
    w = World(pg)
    target, _ = frozen_target(w)
    before = operative_counts(pg.runtime)
    row = runner(w).run(req(arm="candidate", target=target), tenant_id="t1")
    assert (row.report or {})["status"] == "completed", row.report
    assert operative_counts(pg.runtime) == before  # no operational write at all (arms never touch the registry)
    registry = PostgresRegistry(w.store, FakeClock())
    sel = AgentSelector(id=AGENT, alias="prod")
    live = registry.resolve_release(sel, principal(type="customer", id="c1"))
    assert live.id == "rel-demo" and not any(r.id.startswith("rel-") and r.id != "rel-demo" for r in [live])
    with pytest.raises(KeyError):
        registry.release_status("candidate")  # the constant candidate id is unknown to the live registry


@pytest.mark.parametrize("tamper", ["candidate_hash", "draft_plan_digest", "expected_rev", "base_release_id",
                                    "proposal_id", "kind"])
def test_target_integrity_negatives_are_failed_infra_never_a_fallback(pg, tamper: str) -> None:  # type: ignore[no-untyped-def]
    w = World(pg)
    target, _ = frozen_target(w)
    target[tamper] = {"expected_rev": 99, "proposal_id": "nope", "kind": "weird"}.get(tamper, "0" * 64)
    rep = runner(w).run(req(arm="candidate", target=target), tenant_id="t1").report or {}
    assert rep["status"] == "failed_infra" and rep["reason"] == "target_preparation_failed"
    assert w.storage.jobs == 0 and rep["event_refs"] == []


def test_commitment_mismatch_and_inactive_release(pg) -> None:  # type: ignore[no-untyped-def]
    w = World(pg)
    r = runner(w)
    assert (r.run(req("k-a", target_commitment="0" * 64), tenant_id="t1").report or {})["reason"] == \
        "target_preparation_failed"
    rep = r.run(req("k-b", target={"kind": "published_release", "release_id": "rel-ghost"}), tenant_id="t1").report
    assert rep and rep["status"] == "failed_infra" and w.storage.jobs == 0
    with pytest.raises(TargetError):
        TargetLoader(w.store).load({"kind": "published_release", "release_id": "rel-ghost"})


def test_infra_failures_map_to_failed_infra(pg) -> None:  # type: ignore[no-untyped-def]
    w = World(pg)
    arts = FakeArtifacts()
    r = runner(w, artifacts=arts)
    assert (r.run(req("k1", scenario_manifest_ref="art-missing"), tenant_id="t1").report or {})["reason"] == \
        "manifest_missing"
    assert (r.run(req("k2", budget_ref="bud-missing"), tenant_id="t1").report or {})["reason"] == "budget_unknown"


def test_broker_mismatch_is_403_and_does_zero_work(pg) -> None:  # type: ignore[no-untyped-def]
    w = World(pg)
    w.broker.allow = False
    r = runner(w)
    with pytest.raises(ArmDenied) as e:
        r.run(req(), tenant_id="t1")
    assert e.value.status == 403 and w.storage.jobs == 0
    assert r.read_by_key("k1", tenant_id="t1").status == "failed_infra"  # type: ignore[union-attr]


def test_request_is_extra_forbid_so_no_gold_or_oracle_rides_along(pg) -> None:  # type: ignore[no-untyped-def]
    w = World(pg)
    for extra in ({"gold": "x"}, {"oracle": {"canary": "ORACLE-CANARY-1"}}, {"expected": "y"}):
        with pytest.raises(ArmDenied) as e:
            runner(w).run({**req(), **extra}, tenant_id="t1")
        assert e.value.status == 422


def test_oracle_never_requested_and_canary_absent_from_events_and_report(pg) -> None:  # type: ignore[no-untyped-def]
    """The oracle only travels as an opaque ref; only the sealed scenario manifest is ever fetched."""
    import psycopg

    asked: list[str] = []

    class Spy(FakeArtifacts):
        def artifact_get(self, ref: str, tenant_id: str, binding_ref: str) -> dict[str, Any]:
            asked.append(ref)
            return super().artifact_get(ref, tenant_id, binding_ref)

    canary = "ORACLE-CANARY-7f3a"
    w = World(pg)
    rep = runner(w, artifacts=Spy()).run(req(oracle_ref="oracle-ref-1"), tenant_id="t1").report or {}
    assert asked == ["art-1"] and canary not in str(rep)
    with psycopg.connect(pg.eval) as conn:
        dump = " ".join(r[0] for r in conn.execute("SELECT row_to_json(a)::text FROM audit_events a").fetchall())
    assert dump and canary not in dump and "oracle" not in dump.lower()


def test_mixed_world_is_rejected_and_task_modes_need_the_bank(pg) -> None:  # type: ignore[no-untyped-def]
    w = World(pg)
    with pytest.raises(ArmDenied) as e1:
        runner(w, FakeBank()).run(req(seed_manifest_ref="seed-1"), tenant_id="t1")  # native + bank
    assert e1.value.code == "mixed_world_rejected"
    with pytest.raises(ArmDenied) as e2:
        runner(w, None).run(req("k9", mode="stateful_attention"), tenant_id="t1")  # stateful without bank
    assert e2.value.code == "sandbox_required"


def test_stateful_arm_uses_the_bank_and_closes_with_evidence(pg) -> None:  # type: ignore[no-untyped-def]
    w = World(pg)
    bank = FakeBank()
    rep = runner(w, bank).run(req(mode="stateful_attention", seed_manifest_ref="seed-1"), tenant_id="t1").report or {}
    assert rep["status"] == "completed", rep
    assert bank.opened == 1 and bank.closed == 1
    assert rep["initial_state_digest"] == "sha256:initial" and rep["final_state_ref"] == "final:s1"


def test_timeout_after_send_is_unknown_and_restart_does_not_rerun(pg) -> None:  # type: ignore[no-untyped-def]
    w = World(pg)
    bank = FakeBank(fail_open=SandboxTimeout())
    r = runner(w, bank)
    rep = r.run(req(mode="stateful_attention", seed_manifest_ref="seed-1"), tenant_id="t1").report or {}
    assert rep["status"] == "unknown"
    # simulate a bridge killed mid-arm: row left `running`, then a restarted runner (fresh in-flight set)
    store = PgArmStore(pg.runtime)
    store.begin(execution_id_for("k-killed", "t1"), "t1|k-killed", "dg")
    jobs = w.storage.jobs
    restarted = runner(w, FakeBank(), store=store)
    assert restarted.read_by_key("k-killed", tenant_id="t1").status == "unknown"  # type: ignore[union-attr]
    with pytest.raises(ArmDenied):  # other digest on the same key: conflict, never a run
        restarted.run(req("k-killed"), tenant_id="t1")
    assert w.storage.jobs == jobs


def test_executor_uncertain_then_readback_resolves() -> None:
    from agent_core.adapters.system_ids import SystemIds

    bank = FakeBank(fail_act=True)
    adapter_unresolved: set[str] = set()
    ex = StatefulSandboxToolExecutor(bank, SandboxSession("s", 0, "d"), "ex-1", SystemIds(), None,  # type: ignore[arg-type]
                                     [], adapter_unresolved)
    ctx = ToolCallContext(run_id="r", release="candidate", principal=principal(type="builder", id="b",
                                                                              roles=["constructor"]))
    res = ex.execute(EntityRef(id="sandbox/case_open", version="1.0.0"), {"a": 1}, {}, ctx, "act-1")
    assert res.status is ToolStatus.uncertain and len(adapter_unresolved) == 1
    key = next(iter(adapter_unresolved))
    rb = ex.execute(EntityRef(id="sandbox/get_action", version="1.0.0"), {"action_key": key}, {}, ctx)
    assert rb.status is ToolStatus.ok and not adapter_unresolved
    bad = ex.execute(EntityRef(id="sandbox/drop_database", version="1.0.0"), {}, {}, ctx)
    assert bad.status is ToolStatus.error and bad.error == "unsupported_action"
    miss = ex.execute(EntityRef(id="sandbox/get_action", version="1.0.0"), {"action_key": "nope"}, {}, ctx)
    assert miss.status is ToolStatus.error


def test_in_memory_arm_store_single_flight() -> None:
    s = InMemoryArmStore()
    a, created = s.begin("e1", "k", "d")
    b, again = s.begin("e2", "k", "d")
    assert created and not again and b.execution_id == "e1"
    _ = (SandboxPortAdapter, ToolDef)  # imported for the public surface check


def test_arm_runner_hands_binding_ref_to_the_artifact_port_and_arm_identity_to_the_bank(pg) -> None:  # type: ignore[no-untyped-def]
    w = World(pg)
    arts, bank = FakeArtifacts(), FakeBank()
    rep = runner(w, bank, artifacts=arts).run(
        req(mode="task_builder", seed_manifest_ref="seed-1", campaign_ref="camp-9", binding_ref="bind-7"),
        tenant_id="t1").report or {}
    assert rep["status"] == "completed", rep
    assert arts.calls == [("art-1", "t1", "bind-7")]  # the broker route needs the binding for its grant check
    assert bank.contexts == [{"tenant_id": "t1", "job_id": execution_id_for("k1", "t1"), "campaign_ref": "camp-9",
                              "case_ref": "case-1", "arm": "base", "repetition": 0}]

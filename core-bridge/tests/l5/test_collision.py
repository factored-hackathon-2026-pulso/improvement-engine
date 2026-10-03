"""FIRST RED (plan 17.3.5 / DR-02): a persistent eval DB makes the stock harness replay `eval-{scenario.id}`.

Marked SV (subject to verification): the plan's probe was in memory; here it runs on real Postgres 16."""

from __future__ import annotations

import pytest
from agent_core.adapters.system_ids import SystemIds
from agent_core.composition import EngineScenarioHarness
from agent_core.registry import EvalRequest, EvalTarget, LocalSandbox, ScenarioEvaluator, SnapshotRegistry
from agent_core.registry.evaluation.yardstick import Yardstick

from pulso_core_runtime.harness import PulsoScenarioHarness
from l5.world import common_kwargs, demo_pinned, suite_with

pytestmark = [pytest.mark.runtime, pytest.mark.pg]


def _targets() -> tuple[EvalTarget, EvalTarget]:
    pinned = demo_pinned()
    reg = SnapshotRegistry(pinned.release, pinned.entities)
    return EvalTarget("candidate", pinned.release, reg), EvalTarget("base", pinned.release, reg)


def _request(suite_version_new: str = "1.0.0", reps: int = 3) -> EvalRequest:
    cand, base = _targets()
    new = Yardstick(metrics=[], suite=suite_with(reps, suite_version_new))
    old = Yardstick(metrics=[], suite=suite_with(reps, "1.0.0"))
    return EvalRequest(candidate=cand, new=new, base=base, old=old)


def _run_ids(harness: PulsoScenarioHarness) -> list[str]:
    return [r.run_id for r in harness.runs]


def test_stock_harness_replays_on_a_persistent_eval_db(pg) -> None:  # type: ignore[no-untyped-def]
    """Documents the defect: same principal + scenario -> `start_run` returns the stored run."""
    stock = EngineScenarioHarness(**common_kwargs(pg.eval))
    cand, base = _targets()
    scenario = suite_with(1).scenarios[0]
    sandbox = LocalSandbox(SystemIds())
    ids = []
    for target in (cand, base):
        events = stock.run(target, "atencion", scenario, sandbox.tools(sandbox.provision(scenario.seed, target)))
        ids.append(events[0].run_id)
    assert ids[0] == ids[1], "stock harness is expected to replay one run for base and candidate"


def test_pulso_harness_gives_distinct_runs_per_label_and_repetition(pg) -> None:  # type: ignore[no-untyped-def]
    harness = PulsoScenarioHarness(mode="native", **common_kwargs(pg.eval))
    report = ScenarioEvaluator(harness, LocalSandbox(SystemIds()), max_workers=1).run(_request())
    assert report.verdict != "failed_infra", report.detail
    ids = _run_ids(harness)
    assert len(ids) == 6 and len(set(ids)) == 6  # 2 labels x 3 repetitions (shared suite)
    assert len({r.key for r in harness.runs}) == 6


def test_distinct_suites_run_candidate_twice_and_stay_distinct(pg) -> None:  # type: ignore[no-untyped-def]
    """The plan says "7 distinct run ids"; the evaluator's plan yields 6 (shared suite) or 9 (old != new)."""
    harness = PulsoScenarioHarness(mode="native", **common_kwargs(pg.eval))
    ScenarioEvaluator(harness, LocalSandbox(SystemIds()), max_workers=1).run(_request("1.1.0"))
    ids = _run_ids(harness)
    assert len(ids) == 9 and len(set(ids)) == 9


def test_two_harness_instances_on_one_db_never_collide(pg) -> None:  # type: ignore[no-untyped-def]
    """A fresh harness per evaluate call (as PulsoEvalPort does) must not reuse keys: nonce per instance."""
    ids: set[str] = set()
    for _ in range(2):
        harness = PulsoScenarioHarness(mode="native", **common_kwargs(pg.eval))
        ScenarioEvaluator(harness, LocalSandbox(SystemIds()), max_workers=1).run(_request(reps=1))
        ids.update(_run_ids(harness))
    assert len(ids) == 4


def test_collision_with_parallel_workers_stays_distinct(pg) -> None:  # type: ignore[no-untyped-def]
    """SV: the evaluator pool (max_workers>1) calls the harness concurrently; keys must still be unique."""
    harness = PulsoScenarioHarness(mode="native", **common_kwargs(pg.eval))
    ScenarioEvaluator(harness, LocalSandbox(SystemIds()), max_workers=4).run(_request())
    ids = _run_ids(harness)
    assert len(ids) == 6 and len(set(ids)) == 6

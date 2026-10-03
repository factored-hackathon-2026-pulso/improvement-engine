"""L5 review round 2 (team-lead decisions): in-progress denial without mutation, canonical native_evaluate digest,
explicit early close evidence, arms tenant isolation and the mixed-world / target hardening."""

from __future__ import annotations

import threading
from datetime import timedelta
from typing import Any

import pytest

from l5.test_arms import FakeArtifacts, FakeBank, req, runner
from l5.test_evaluate_path import World
from l5.world import suite_with
from pulso_core_runtime.evaluation.admission import AdmissionDenied
from pulso_core_runtime.evaluation.arms import ArmDenied, execution_id_for

pytestmark = [pytest.mark.runtime, pytest.mark.pg]


class BlockingGateway:
    """Holds the first model call until released (the first run stays in flight)."""

    def __init__(self, inner: Any) -> None:
        self.inner, self.entered, self.release = inner, threading.Event(), threading.Event()
        self._first = True

    def generate(self, *a: Any, **k: Any) -> Any:
        if self._first:
            self._first = False
            self.entered.set()
            assert self.release.wait(30)
        return self.inner.generate(*a, **k)


def test_concurrent_second_caller_is_in_progress_and_does_not_mutate_the_admission(pg) -> None:  # type: ignore[no-untyped-def]
    from testing.engine_world import CitingGateway

    gw = BlockingGateway(CitingGateway())
    w = World(pg, gateway=gw)
    pid, chash = w.frozen_proposal()
    w.admit(pid, chash)
    first: list[Any] = []
    t = threading.Thread(target=lambda: first.append(w.evaluate(pid)))
    t.start()
    assert gw.entered.wait(30)
    with pytest.raises(AdmissionDenied) as denied:
        w.evaluate(pid)
    assert denied.value.code == "evaluation_in_progress" and denied.value.status == 409
    assert w.rt.admissions.get("ctx-1").state == "consumed"  # type: ignore[union-attr]  # untouched
    gw.release.set()
    t.join(30)
    assert first and first[0].verdict == "pass" and w.storage.jobs == 1  # the first run's finish still succeeds
    assert w.evaluate(pid).replayed  # and the admission replays its stored result


def test_consumed_past_deadline_without_result_becomes_unknown(pg) -> None:  # type: ignore[no-untyped-def]
    w = World(pg)
    pid, chash = w.frozen_proposal()
    w.admit(pid, chash)
    assert w.rt.admissions.transition("ctx-1", "admitted", "consumed")
    w.clock_now += timedelta(hours=2)  # the run cannot still be in flight: crash recovery
    with pytest.raises(AdmissionDenied) as e:
        w.evaluate(pid)
    assert e.value.code == "evaluation_unknown" and w.rt.admissions.get("ctx-1").state == "unknown"  # type: ignore[union-attr]


def test_flow_broker_digest_is_the_canonical_broad_digest(pg) -> None:  # type: ignore[no-untyped-def]
    from pulso_core_runtime.evaluation.digests import native_evaluate_digest

    seen: list[str] = []

    class Rec:
        def check(self, *, tenant_id: str, binding_ref: str, scope: str, payload_digest: str) -> bool:
            seen.append(payload_digest)
            return True

    w = World(pg)
    w.rt.gate._broker = Rec()  # type: ignore[assignment]
    pid, chash = w.frozen_proposal()
    adm = w.admit(pid, chash)
    w.evaluate(pid)
    expected = native_evaluate_digest(proposal_id=pid, evaluation_context_ref="ctx-1", candidate_hash=chash,
                                      suite_id=adm.suite_id, suite_version=adm.suite_version,
                                      suite_digest=adm.suite_digest)
    assert seen == [expected]
    assert expected != native_evaluate_digest(proposal_id=pid, evaluation_context_ref="ctx-1",
                                              candidate_hash="0" * 64, suite_id=adm.suite_id,
                                              suite_version=adm.suite_version, suite_digest=adm.suite_digest)


def _closed_then_turn_scenario() -> dict[str, Any]:
    sc = suite_with().scenarios[0].model_dump(mode="json")
    sc["steps"] = sc["steps"] + [{"op": "turn", "text": "anything after the run closed", "auth": "step_up"}]
    return sc


def test_early_close_is_explicit_evidence_in_harness_and_arm_report(pg) -> None:  # type: ignore[no-untyped-def]
    w = World(pg)
    base = suite_with().scenarios[0].model_dump(mode="json")
    rep = runner(w, artifacts=FakeArtifacts([base])).run(req("k-ok"), tenant_id="t1").report or {}
    assert rep["status"] == "completed" and rep["closed_early"] is False
    rep = runner(w, artifacts=FakeArtifacts([_closed_then_turn_scenario()])).run(
        req("k-early"), tenant_id="t1").report or {}
    assert rep["status"] == "completed" and rep["closed_early"] is True, rep
    assert rep["closed_early_runs"] and rep["closed_early_runs"][0].startswith("audit:")


def test_early_close_is_in_the_stored_eval_report(pg) -> None:  # type: ignore[no-untyped-def]
    w = World(pg)
    pid, chash = w.frozen_proposal()
    w.admit(pid, chash)
    out = w.evaluate(pid)
    assert out.report["pulso_evidence"]["closed_early"] is False
    stored = w.rt.reports.get(pid, out.eval_run_ref)
    assert stored is not None and stored.report["pulso_evidence"] == out.report["pulso_evidence"]


# --- arms review -----------------------------------------------------------------------------------------

def test_arm_reports_are_tenant_isolated(pg) -> None:  # type: ignore[no-untyped-def]
    w = World(pg)
    r = runner(w)
    mine = r.run(req("k-shared"), tenant_id="t1")
    other = r.run(req("k-shared"), tenant_id="t2")  # same key, other tenant: its own execution, never t1's report
    assert other.execution_id != mine.execution_id
    assert r.read(mine.execution_id, tenant_id="t2") is None
    assert r.read_by_key("k-shared", tenant_id="t3") is None
    assert r.read(mine.execution_id, tenant_id="t1") is not None
    assert execution_id_for("k-shared", "t1") == mine.execution_id


def test_native_with_empty_string_seed_manifest_is_still_mixed_world(pg) -> None:  # type: ignore[no-untyped-def]
    w = World(pg)
    with pytest.raises(ArmDenied) as e:
        runner(w, FakeBank()).run(req(seed_manifest_ref=""), tenant_id="t1")
    assert e.value.code == "mixed_world_rejected"


def test_empty_scenario_manifest_is_failed_infra_never_a_silent_completed(pg) -> None:  # type: ignore[no-untyped-def]
    w = World(pg)
    rep = runner(w, artifacts=FakeArtifacts([])).run(req("k-empty"), tenant_id="t1").report or {}
    assert rep["status"] == "failed_infra" and rep["reason"] == "manifest_empty"


def test_target_store_error_is_failed_infra_not_a_500(pg) -> None:  # type: ignore[no-untyped-def]
    w = World(pg)
    r = runner(w)

    class Boom:
        def load(self, target: Any) -> Any:
            raise OSError("db down")

    r.loader = Boom()  # type: ignore[assignment]
    rep = r.run(req("k-boom"), tenant_id="t1").report or {}
    assert rep["status"] == "failed_infra" and rep["reason"] == "target_load_failed"


def test_unexpected_harness_error_is_not_a_500(pg) -> None:  # type: ignore[no-untyped-def]
    w = World(pg)
    r = runner(w)

    class Boom:
        ids = r.composition.ids

        def harness(self, *a: Any, **k: Any) -> Any:
            raise RuntimeError("boom")

    r.composition = Boom()  # type: ignore[assignment]
    rep = r.run(req("k-rt"), tenant_id="t1").report or {}
    assert rep["status"] == "failed_infra" and rep["reason"] == "unexpected:RuntimeError"


def test_supersedes_must_name_an_unknown_arm_of_the_same_tenant(pg) -> None:  # type: ignore[no-untyped-def]
    w = World(pg)
    r = runner(w)
    done = r.run(req("k-done"), tenant_id="t1")
    for n, bad in enumerate(("arm-ghost", done.execution_id)):  # missing / not `unknown`
        with pytest.raises(ArmDenied) as e:
            r.run(req(f"k-re-{n}", supersedes_execution_id=bad), tenant_id="t1")
        assert e.value.code == "supersedes_invalid" and e.value.status == 409

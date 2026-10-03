"""Independent review round 1: key burning, report/commit atomicity, trigger-proof zero operative writes."""

from __future__ import annotations

from typing import Any

import psycopg
import pytest
from agent_core.domain import GatewayError, GatewayErrorKind
from agent_core.registry import RegistryError, RegistryErrorCode

from l5.test_evaluate_path import World
from l5.world import suite_draft
from pulso_core_runtime.evaluation.admission import eval_key

pytestmark = [pytest.mark.runtime, pytest.mark.pg]

# Operative tables an evaluation must never write. Allowed by design: reg_eval_runs (native evaluate record),
# reg_events, reg_draft_writes (service idempotency record), reg_proposals (state transition).
FORBIDDEN = ["reg_blobs", "reg_entity_versions", "reg_releases", "reg_release_entities", "reg_aliases",
             "reg_alias_log", "reg_approvals", "reg_publish_keys", "runs", "audit_events", "usage"]


def _arm_triggers(dsn: str) -> None:
    with psycopg.connect(dsn, autocommit=True) as c:
        c.execute("CREATE TABLE IF NOT EXISTS write_probe (tbl text, op text)")
        c.execute("CREATE OR REPLACE FUNCTION probe_fn() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN "
                  "INSERT INTO write_probe VALUES (TG_TABLE_NAME, TG_OP); RETURN NULL; END $$")
        for t in FORBIDDEN:
            c.execute(f"CREATE TRIGGER probe_{t} AFTER INSERT OR UPDATE OR DELETE ON {t} "  # type: ignore[arg-type]
                      "FOR EACH STATEMENT EXECUTE FUNCTION probe_fn()")


def _probe(dsn: str) -> list[Any]:
    with psycopg.connect(dsn, autocommit=True) as c:
        return c.execute("SELECT tbl, op FROM write_probe").fetchall()


@pytest.mark.parametrize("kind", ["pass", "fail", "infra"])
def test_zero_operative_writes_trigger_proof(pg, kind: str) -> None:  # type: ignore[no-untyped-def]
    class Down:
        def generate(self, *a: Any, **k: Any) -> Any:
            raise GatewayError(GatewayErrorKind.unavailable)

    w = World(pg, gateway=Down() if kind == "infra" else None)
    pid, chash = w.frozen_proposal(suite_draft(outcome="escalated") if kind == "fail" else None)
    w.admit(pid, chash)
    _arm_triggers(pg.runtime)
    try:
        w.evaluate(pid)
    except RegistryError as exc:
        assert kind == "fail" and exc.code is RegistryErrorCode.gate_failed
    assert _probe(pg.runtime) == []
    try:
        w.evaluate(pid)  # replay writes nothing either
    except RegistryError:
        pass
    assert _probe(pg.runtime) == []


def test_no_admission_call_cannot_burn_the_future_admitted_key(pg) -> None:  # type: ignore[no-untyped-def]
    """The shared (fail-closed) service must not leave a stored `failed_infra` under `pulso-eval:<ref>`."""
    w = World(pg)
    pid, chash = w.frozen_proposal()
    w.admit(pid, chash, "ctx-burn")
    report = w.rt.service.evaluate(w.actor, pid, "disputas-suite", "1.0.0", idempotency_key=eval_key("ctx-burn"))
    assert report.verdict == "failed_infra"
    out = w.evaluate(pid, evaluation_context_ref="ctx-burn")
    assert out.verdict == "pass" and not out.replayed and w.storage.jobs == 1


@pytest.mark.parametrize("fail", [False, True])
def test_report_persistence_failure_after_core_commit_heals_on_replay(pg, fail: bool) -> None:  # type: ignore[no-untyped-def]
    w = World(pg)
    pid, chash = w.frozen_proposal(suite_draft(outcome="escalated") if fail else None)
    w.admit(pid, chash)
    real_put = w.rt.reports.put
    calls = {"n": 0}

    def flaky(*a: Any, **k: Any) -> Any:
        calls["n"] += 1
        if calls["n"] == 1:
            raise OSError("db blip")
        return real_put(*a, **k)

    w.rt.reports.put = flaky  # type: ignore[method-assign]
    with pytest.raises(OSError):
        w.evaluate(pid)
    assert w.rt.reports.list_for(pid) == []
    jobs = w.storage.jobs
    if fail:
        with pytest.raises(RegistryError):
            w.evaluate(pid)
    else:
        assert w.evaluate(pid).replayed
    assert w.storage.jobs == jobs  # no second run
    rows = w.rt.reports.list_for(pid)
    assert len(rows) == 1 and rows[0].gate_failed is fail

"""Exporter -> ingest fixture: completeness, exactly-once, chain verification, tenant, fault resume."""

from __future__ import annotations

import time
from typing import Any

import pytest

from codex_standin import PIN_SHA
from codex_standin.engine import RESEARCH, TENANT, hypotheses_output
from helpers import tag

pytestmark = pytest.mark.live
AUDIT_SOURCE = "local.audit"


def _audit_keys(ing: dict[str, Any]) -> list[Any]:
    """Ledger identity is (tenant, source_id, kind, level, native_event_id)."""
    return [tuple(k) for k in ing["event_keys"] if k[1] == AUDIT_SOURCE and k[3] == "engine_event"]


def _audit_rows(stack: Any) -> int:
    return int(stack.runtime_db.one("select count(*) from audit_events"))


def _wait_exported(stack: Any, timeout: float = 100.0) -> dict[str, Any]:
    """Waits until the audit events of the runtime DB are all in the ingest ledger (fast_poll or rescan)."""
    deadline, last = time.time() + timeout, {}
    while time.time() < deadline:
        last = stack.engine.state()["ingest"]
        audit = _audit_keys(last)
        if len(audit) >= _audit_rows(stack):
            return last  # type: ignore[no-any-return]
        time.sleep(3)
    raise AssertionError(f"exporter did not catch up: ingest={len(last.get('event_keys', []))} db={_audit_rows(stack)}")


def _closed_runs(stack: Any, pipeline: Any) -> set[str]:
    return {s.out["core_run_id"] for s in (pipeline.scout, pipeline.verifier, pipeline.design, pipeline.writer)}


def test_exporter_delivers_every_audit_event_exactly_once_to_the_ingest_fixture(stack: Any, pipeline: Any, effect: Any) -> None:
    ing = _wait_exported(stack)
    audit = _audit_keys(ing)
    assert len(audit) == len(set(audit)) == _audit_rows(stack)  # complete, one ledger entry per audit row
    fast = [b for b in ing["batch_summaries"] if b["scan_mode"] == "fast_poll"]
    assert fast and sum(b["n"] for b in fast if b["source_id"] == AUDIT_SOURCE) == len(audit)  # no event resent on the fast path
    runs = {r for b in ing["batch_summaries"] for r in b["runs"]}
    assert _closed_runs(stack, pipeline) <= runs
    assert all(c["revision"] >= 1 for c in ing["cursors"].values())  # checkpoints advanced after commit
    effect("exporter_audit_events", len(audit))


def test_exporter_chain_verification_receipts_are_ok_for_the_pipeline_runs(stack: Any, pipeline: Any, effect: Any) -> None:
    ing = _wait_exported(stack)
    by_run = {r["run_id"]: r for r in ing["verification_receipts"]}
    for run_id in _closed_runs(stack, pipeline):
        rc = by_run.get(run_id)
        assert rc is not None, f"no verification receipt for {run_id}"
        assert rc["check_result"] == {"broken_at": None, "ok": True, "reason": None}
        assert rc["verifier_agent_core_sha"] == PIN_SHA and rc["verifier_contract_version"] == "1.3.0"
        head = stack.runtime_db.rows("select hash from audit_events where run_id=%s order by seq desc limit 1", run_id)
        assert rc["chain_head_hash"] == head[0][0]  # the receipt verifies the head the DB really has
    effect("chain_receipts_ok", len(by_run))


def test_no_event_leaves_under_another_tenant(stack: Any, pipeline: Any) -> None:
    ing = _wait_exported(stack)
    assert {b["tenant_id"] for b in ing["batch_summaries"]} == {TENANT}
    assert {a["tenant_id"] for a in ing["auth_log"]} == {TENANT}
    assert {k[0] for k in ing["event_keys"]} == {TENANT}  # the ledger identity carries the tenant
    assert {k[1].split(".")[0] for k in ing["event_keys"]} == {"local"}


def test_ingest_outage_does_not_duplicate_or_lose_events_after_resume(stack: Any, effect: Any) -> None:
    e, n = stack.engine, tag()
    e.configure(llm_replace=True, llm_rules=[])
    e.script_stage_model(f"scout-x-{n}", RESEARCH, hypotheses_output(TENANT))
    e.configure(ingest_fail_next=[503, 503, 503])  # the next three exporter POSTs fail before any write
    s = e.stage("scout", f"job-x-{n}", "x", "pulso-scout", {"briefing_ref": f"wiki/x-{n}.md"},
                memory_snapshot_ref="m", extract_manifest_ref="x")
    assert s.out["state"] == "terminal_ok", s.out
    ing = _wait_exported(stack)
    audit = _audit_keys(ing)
    assert len(audit) == len(set(audit)) == _audit_rows(stack)
    fast = [b for b in ing["batch_summaries"] if b["scan_mode"] == "fast_poll" and b["source_id"] == AUDIT_SOURCE]
    assert sum(b["n"] for b in fast) == len(audit)  # resumed from the checkpoint: nothing sent twice
    assert s.out["core_run_id"] in {r for b in ing["batch_summaries"] for r in b["runs"]}
    effect("exporter_outage_resume_events", len(audit))

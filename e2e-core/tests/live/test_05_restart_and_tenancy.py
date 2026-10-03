"""Durability across a runtime restart (receipts, jti replay store) and the tenant-claim gap probe (runs last)."""

from __future__ import annotations

import time
from typing import Any

import httpx
import pytest

from codex_standin.engine import OTHER, RESEARCH, TENANT, hypotheses_output
from codex_standin.stack import podman, project
from helpers import tag

pytestmark = pytest.mark.live


def _restart(stack: Any) -> None:
    podman("restart", f"{project(stack.env['namespace'])}-core-runtime-1")
    deadline = time.time() + 120
    while time.time() < deadline:
        try:
            if httpx.get(stack.runtime + "/readyz", timeout=3).status_code == 200:
                return
        except httpx.HTTPError:
            pass
        time.sleep(2)
    raise AssertionError("runtime did not become ready after restart")


def test_receipts_and_jti_store_survive_a_runtime_restart_without_second_effects(stack: Any, pipeline: Any, effect: Any) -> None:
    e = stack.engine
    used = stack.bridge.token("core_task_invoke", TENANT)
    hdr = {"Authorization": "Bearer " + used, "Idempotency-Key": pipeline.writer.key}
    assert httpx.post(stack.bridge.base + "/core-tasks/invoke", json=pipeline.writer.body, headers=hdr).status_code == 200
    before = {"props": stack.runtime_db.one("select count(*) from reg_proposals"),
              "writes": stack.runtime_db.one("select count(*) from reg_draft_writes"),
              "runs": stack.runtime_db.one("select count(*) from runs"),
              "bindings": dict(e.state()["binding_effects"]), "llm": len(e.state()["llm_calls"])}
    _restart(stack)
    again = stack.bridge.invoke(TENANT, pipeline.writer.key, pipeline.writer.body)
    assert again.status_code == 200 and again.json()["core_run_id"] == pipeline.writer.out["core_run_id"]
    assert again.json()["state"] == "terminal_ok"
    replay = httpx.post(stack.bridge.base + "/core-tasks/invoke", json=pipeline.writer.body, headers=hdr)
    assert replay.status_code == 401 and replay.json()["details"]["reason"] == "jti_replayed"  # PG-backed, durable
    after = {"props": stack.runtime_db.one("select count(*) from reg_proposals"),
             "writes": stack.runtime_db.one("select count(*) from reg_draft_writes"),
             "runs": stack.runtime_db.one("select count(*) from runs"),
             "bindings": dict(e.state()["binding_effects"]), "llm": len(e.state()["llm_calls"])}
    assert after == before
    ver = stack.bridge.version()
    assert ver["image_digest"] == stack.env["expected_image_digest"]
    effect("restart_replay_extra_effects", {k: after[k] != before[k] for k in before})


def test_runtime_refuses_a_tenant_it_is_not_configured_for(stack: Any, pipeline: Any, gap: Any, effect: Any) -> None:
    """Probe, runs LAST: the stack is configured for one tenant (PULSO_TENANT_ID) and the exporter exports the whole
    Core DB under that tenant. A runtime that accepts another tenant's signed claim would start a run whose audit
    events the exporter ships as this tenant's (cross-tenant leak). Today the runtime accepts any tenant claim equal
    to the body tenant: recorded as a gap, asserted as a failing expectation (xfail) so it cannot pass silently."""
    e, n = stack.engine, tag()
    e.configure(llm_replace=True, llm_rules=[])
    e.script_stage_model(f"scout-o-{n}", RESEARCH, hypotheses_output(OTHER))
    s = e.stage("scout", f"job-o-{n}", "o", "pulso-scout", {"briefing_ref": "wiki/o.md"}, tenant=OTHER,
                memory_snapshot_ref="m", extract_manifest_ref="x")
    if s.response.status_code in (401, 403):
        return  # fixed: the runtime pins its tenant
    run = s.out.get("core_run_id")
    gap("runtime_accepts_foreign_tenant_claims",
        f"POST /core-tasks/invoke with a valid service JWT for tenant '{OTHER}' ran a real Core run "
        f"(state={s.out.get('state')}) on the runtime configured with PULSO_TENANT_ID={TENANT}; its audit events live in "
        "the same Core DB the exporter ships under the single configured tenant.",
        "L2/L3a: reject (403 pulso:tenant_mismatch) any tenant claim different from the runtime's configured "
        "PULSO_TENANT_ID (or give the exporter per-tenant sources) before the receipt CAS.")
    effect("foreign_tenant_run_started", bool(run))
    pytest.xfail("runtime accepted a foreign tenant claim (gap runtime_accepts_foreign_tenant_claims)")

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
    used = stack.bridge.token("core_task_invoke", TENANT, job_id=pipeline.writer.body["job_id"])
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


def test_runtime_refuses_a_tenant_it_is_not_configured_for(stack: Any, pipeline: Any, effect: Any) -> None:
    """Runs LAST: the stack is configured for one tenant (PULSO_TENANT_ID) and the exporter exports the whole Core DB
    under it. A valid service JWT for another tenant must be refused 403 pulso:tenant_mismatch before the receipt CAS:
    no receipt, no Core run, no binding, no model call for the intruder."""
    e, n = stack.engine, tag()
    e.configure(llm_replace=True, llm_rules=[])
    e.script_stage_model(f"scout-o-{n}", RESEARCH, hypotheses_output(OTHER))
    receipts = stack.runtime_db.one("select count(*) from pulso_bridge.receipts where tenant_id=%s", OTHER)
    runs = stack.runtime_db.one("select count(*) from runs")
    calls = len(e.state()["llm_calls"])
    s = e.stage("scout", f"job-o-{n}", "o", "pulso-scout", {"briefing_ref": "wiki/o.md"}, tenant=OTHER,
                memory_snapshot_ref="m", extract_manifest_ref="x")
    assert s.response.status_code == 403, s.response.text
    assert s.out["code"] == "pulso:tenant_mismatch", s.out
    assert stack.runtime_db.one("select count(*) from pulso_bridge.receipts where tenant_id=%s", OTHER) == receipts
    assert stack.runtime_db.one("select count(*) from runs") == runs  # no Core run started
    assert len(e.state()["llm_calls"]) == calls
    assert not [b for b in e.state()["bindings"] if b["tenant"] == OTHER]
    effect("foreign_tenant_refused", s.out["code"])

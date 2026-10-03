"""e2e-core stand-in defects, reproduced against the composed runtime on real PG16 + the real pinned Core:
(1) a service JWT for a tenant the deployment is not configured for must be refused (403) on every tenant route;
(2) a stage/agent mismatch is refused before any receipt, run or binding; (3) a 5xx on the binding callback AFTER
the platform applied it leaves the receipt `manual_reconcile` (`binding_unproven`), never `terminal_failed`.
Doubles: loopback control-api/broker, scripted gateway."""

from __future__ import annotations

from typing import Any

import psycopg
import pytest

from integration.conftest import WORLD, Composed
from l3a.helpers import body, idem_key

pytestmark = [pytest.mark.integration, pytest.mark.pg, pytest.mark.skipif(not WORLD.is_dir(), reason="world absent")]
OTHER = "t-other"


def _counts(c: Composed) -> tuple[int, int]:
    with psycopg.connect(c.pg.runtime) as conn:
        runs = conn.execute("SELECT count(*) FROM runs").fetchone()[0]  # type: ignore[index]
        receipts = conn.execute("SELECT count(*) FROM pulso_bridge.receipts").fetchone()[0]  # type: ignore[index]
    return runs, receipts


def _invoke(c: Composed, *, tenant: str = "t1", stage: str = "scout", agent: str = "pulso-scout", logical: str = "k",
            token_tenant: str | None = None) -> Any:
    payload = body(tenant=tenant, stage=stage, agent_id=agent, agent_version="1.0.0", logical=logical,
                   release_id=c.release_ids["scout"], input={"briefing_ref": "wiki/b.md"}, memory_snapshot_ref="m")
    return c.client.post("/internal/v1/core-tasks/invoke", json=payload, headers={
        **c.headers("core_task_invoke", tenant=token_tenant or tenant),
        "Idempotency-Key": idem_key(tenant, "j1", stage, 1, logical)})


def test_foreign_tenant_claim_is_refused_on_every_tenant_route_with_zero_effects(composed: Composed) -> None:
    before = _counts(composed)
    r = _invoke(composed, tenant=OTHER)
    assert r.status_code == 403 and r.json()["code"] == "pulso:tenant_mismatch", r.text
    for method, path, purpose in (
            ("GET", "/internal/v1/core-tasks/some-task", "core_task_read"),
            ("POST", "/internal/v1/evaluation/admissions", "evaluation_admit"),
            ("POST", "/internal/v1/evaluation/arms/run", "evaluation_arm_run"),
            ("GET", "/internal/v1/evaluation/arms/arm-1", "evaluation_arm_read"),
            ("GET", "/internal/v1/evaluation/arms/by-key/k", "evaluation_arm_read"),
            ("POST", "/internal/v1/core-credentials/issue", "credential_issue")):
        resp = composed.client.request(method, path, json={} if method == "POST" else None,
                                       headers=composed.headers(purpose, tenant=OTHER))
        assert resp.status_code == 403 and resp.json()["code"] == "pulso:tenant_mismatch", (path, resp.text)
    assert _counts(composed) == before


def test_configured_tenant_still_runs_after_a_refused_foreign_one(composed: Composed) -> None:
    assert _invoke(composed, tenant=OTHER).status_code == 403
    ok = _invoke(composed, tenant="t1", logical="ok")  # any outcome but a tenant refusal
    assert ok.status_code != 403 and ok.json()["core_run_id"]


def test_body_tenant_must_equal_the_claim_even_for_the_configured_tenant(composed: Composed) -> None:
    r = _invoke(composed, tenant="t1", token_tenant=OTHER)
    assert r.status_code == 403 and r.json()["code"] == "pulso:tenant_mismatch"


def test_stage_agent_mismatch_is_refused_before_any_effect(composed: Composed) -> None:
    before = _counts(composed)
    r = _invoke(composed, stage="writer", agent="pulso-scout", logical="mm")
    assert r.status_code == 422 and r.json()["code"] == "pulso:stage_agent_mismatch", r.text
    assert _counts(composed) == before and composed.loop.bindings == []


def test_binding_5xx_after_the_effect_is_manual_reconcile_never_terminal_failed(composed: Composed) -> None:
    composed.loop.backend.bind_mode = "applied_then_503"
    key = idem_key("t1", "j1", "scout", 1, "lost")
    out = _invoke(composed, logical="lost")
    assert out.json()["state"] == "manual_reconcile", out.text
    assert out.json()["reason"] == "binding_unproven"
    assert composed.loop.backend.bound == [("t1", "j1")]  # the platform applied it exactly once
    from pulso_core_runtime.store.receipts import ReceiptStore
    row = ReceiptStore(composed.pg.runtime).get("t1", key)
    assert row is not None and row.state == "manual_reconcile" and row.reason == "binding_unproven"
    again = _invoke(composed, logical="lost")  # replay never binds again and never closes as a failure
    assert again.json()["state"] == "manual_reconcile" and len(composed.loop.backend.bound) == 1

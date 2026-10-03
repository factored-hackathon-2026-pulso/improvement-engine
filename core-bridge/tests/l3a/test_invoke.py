"""Invoke flow (plan 17.3.3 steps 1-11) against real PG for receipts and a recorded Core double."""

from __future__ import annotations

import asyncio
import random
from concurrent.futures import ThreadPoolExecutor
from typing import Any

import pytest

from pulso_core_runtime.invoke.context import ContextMissing, InvocationRegistry, current_context
from pulso_core_runtime.invoke.core_client import CoreResponse
from pulso_core_runtime.invoke.projection import FactSpec, WhitelistProjector
from pulso_core_runtime.invoke.service import InvokeSettings
from pulso_core_runtime.store.migrations import apply_l3

from .conftest import PgDbs
from .fakes import FakeReleases
from .helpers import body, build_service, idem_key

pytestmark = [pytest.mark.l3a, pytest.mark.pg, pytest.mark.anyio]


@pytest.fixture
def dsn(pg: PgDbs) -> str:
    from pulso_core_runtime.internal.store import ensure_schema
    ensure_schema(pg.runtime)
    apply_l3(pg.runtime)
    return pg.runtime


def key(**kw: Any) -> str:
    return idem_key(kw.get("tenant", "t1"), kw.get("job", "j1"), kw.get("stage", "scout"), kw.get("attempt", 1),
                    kw.get("logical", "k"))


async def test_happy_path_receipt_pin_and_context_cleanup(dsn: str) -> None:
    reg = InvocationRegistry()
    svc, core = build_service(dsn, registry=reg)
    out = await svc.invoke("t1", key(), body())
    assert out.status == 200 and out.body["state"] == "terminal_ok"
    r = out.body["receipt"]
    assert r["core_run_id"] == "run-1" and r["release_id"] == "rel-1" and r["outcome"] == "completed"
    assert r["idempotency_key_digest"] != key() and len(r["task_binding_ref"]) == 64
    call = core.start_calls[0]
    assert call["key"] == key() and call["body"] == {"agent": "pulso-scout@1.0.0", "subject": None,
                                                     "input": {"q": "x"}}
    assert call["principal"] == "pulso-bot:t1:task:scout"
    assert len(reg) == 0  # context removed on terminal
    again = await svc.invoke("t1", key(), body())  # replay of a terminal receipt: stored, no Core call
    assert again.status == 200 and again.body["state"] == "terminal_ok" and len(core.start_calls) == 1


@pytest.mark.parametrize("over,code,status", [
    ({"stage": "nope"}, "pulso:stage_unknown", 400),
    ({"extra_field": 1}, "pulso:invalid_request", 422),
    ({"tenant_id": "other"}, "pulso:tenant_mismatch", 403),
    ({"input": {"blob": "x" * (300 * 1024)}}, "pulso:input_too_large", 413),
])
async def test_validation_rejects_before_any_effect(dsn: str, over: dict[str, Any], code: str, status: int) -> None:
    svc, core = build_service(dsn)
    b = body(**over)
    k = idem_key(b["tenant_id"], b["job_id"], b["stage"], b["attempt"], b["logical_key"])
    out = await svc.invoke("t1", k, b)
    assert (out.status, out.body["code"]) == (status, code) and core.start_calls == []


async def test_unknown_input_slot_and_bad_key(dsn: str) -> None:
    from pulso_core_runtime.invoke.service import InvokeSettings as S
    svc, core = build_service(dsn, settings=S(stage_slots={"scout": frozenset({"q"})}))
    out = await svc.invoke("t1", key(), body(input={"q": 1, "evil": 2}))
    assert out.status == 400 and out.body["code"] == "pulso:unknown_input_slot"
    assert out.body["details"]["slots"] == ["evil"]
    bad = await svc.invoke("t1", "not-the-derived-key", body())
    assert bad.status == 422 and core.start_calls == []


@pytest.mark.parametrize("releases,code", [
    (FakeReleases(status="revoked"), "pulso:release_revoked"),
    (FakeReleases(exists=False), "pulso:release_pin_unavailable"),
    (FakeReleases(version="9.9.9"), "pulso:release_pin_unavailable"),
])
async def test_pre_pin_checks_block_the_call(dsn: str, releases: FakeReleases, code: str) -> None:
    svc, core = build_service(dsn, releases=releases)
    out = await svc.invoke("t1", key(), body())
    assert out.status == 409 and out.body["code"] == code and core.start_calls == []


async def test_release_drift_discards_the_result(dsn: str) -> None:
    svc, core = build_service(dsn)
    core.release_override = "rel-OTHER"
    out = await svc.invoke("t1", key(), body())
    assert out.status == 409 and out.body["state"] == "terminal_failed" and out.body["reason"] == "release_drift"
    assert "result" not in out.body and "receipt" not in out.body


async def test_failed_outcome_is_terminal_failed_not_unknown(dsn: str) -> None:
    svc, core = build_service(dsn)
    orig = core.start_run

    async def failed(bearer: str, k: str, b: dict[str, Any]) -> CoreResponse:
        r = await orig(bearer, k, b)
        return CoreResponse(201, {**r.body, "outcome": "failed", "status": "failed"})

    core.start_run = failed  # type: ignore[method-assign]
    out = await svc.invoke("t1", key(), body())
    assert out.body["state"] == "terminal_failed" and out.body["outcome"] == "failed"


@pytest.mark.parametrize("mode", ["timeout_before_effect", "http_500"])
async def test_post_send_failures_are_unknown_never_failed(dsn: str, mode: str) -> None:
    svc, core = build_service(dsn)
    core.mode = mode
    out = await svc.invoke("t1", key(), body())
    assert out.body["state"] == "unknown" and out.status == 202
    core.mode = "ok"
    again = await svc.invoke("t1", key(), body())  # same key: re-read only, never re-executed
    assert len(core.start_calls) == 1
    assert again.body["state"] in {"unknown", "manual_reconcile"}


async def test_bridge_busy_is_429_and_leaves_no_row(dsn: str) -> None:
    svc, core = build_service(dsn, settings=InvokeSettings(max_inflight=1))
    gate = asyncio.Event()
    orig = core.start_run

    async def slow(bearer: str, k: str, b: dict[str, Any]) -> CoreResponse:
        await gate.wait()
        return await orig(bearer, k, b)

    core.start_run = slow  # type: ignore[method-assign]
    first = asyncio.create_task(svc.invoke("t1", key(), body()))
    await asyncio.sleep(0.3)
    busy = await svc.invoke("t1", key(job="j2"), body(job="j2"))
    assert busy.status == 429 and busy.body["code"] == "pulso:bridge_busy" and busy.body["retryable"] is True
    gate.set()
    assert (await first).body["state"] == "terminal_ok"
    retry = await svc.invoke("t1", key(job="j2"), body(job="j2"))  # same key after 429 starts clean
    assert retry.body["state"] == "terminal_ok"


async def test_projection_missing_required_fact_fails_stage(dsn: str) -> None:
    proj = WhitelistProjector({"scout": {"pulso_hypotheses": FactSpec(required=True)}})
    svc, core = build_service(dsn, projector=proj)
    out = await svc.invoke("t1", key(), body())
    assert out.body["state"] == "terminal_failed" and out.body["reason"] == "output_missing" and out.status == 422
    svc2, core2 = build_service(dsn, projector=proj)
    core2.run_facts = {"pulso_hypotheses": {"value": {"items": [1]}, "source_kind": "agent"}, "secret": {"value": 1}}
    ok = await svc2.invoke("t1", key(logical="k2"), body(logical="k2"))
    facts = ok.body["result"]["facts"]
    assert list(facts) == ["pulso_hypotheses"] and facts["pulso_hypotheses"]["source_kind"] == "agent"
    assert len(facts["pulso_hypotheses"]["digest"]) == 64


# ---- context gate (DR-05): 32 concurrent invokes echo (tenant, job); attr channel vs ContextVar ------------

async def test_32_concurrent_invokes_do_not_cross_context(dsn: str) -> None:
    reg = InvocationRegistry()
    svc, core = build_service(dsn, registry=reg, settings=InvokeSettings(max_inflight=64))
    seen: dict[str, tuple[str, str]] = {}
    orig = core.start_run

    async def echo(bearer: str, k: str, b: dict[str, Any]) -> CoreResponse:
        import base64
        import json
        part = bearer.split(".")[1]
        payload = json.loads(base64.urlsafe_b64decode(part + "=" * (-len(part) % 4)))
        await asyncio.sleep(random.random() * 0.05)
        ctx = reg.resolve(payload["attrs"])  # attr channel
        cv = current_context()
        assert cv is not None and cv.task_binding_ref == ctx.task_binding_ref  # ContextVar agrees (same task)
        seen[b["input"]["job"]] = (ctx.tenant_id, ctx.job_id)
        with ThreadPoolExecutor(1) as pool:  # a worker thread: attr channel works, a bare ContextVar would not
            via_attrs = pool.submit(reg.resolve, payload["attrs"]).result()
            via_var = pool.submit(current_context).result()
        assert via_attrs.job_id == ctx.job_id and via_var is None
        return await orig(bearer, k, b)

    core.start_run = echo  # type: ignore[method-assign]
    jobs = [f"job{i}" for i in range(32)]
    outs = await asyncio.gather(*(svc.invoke("t1", key(job=j), body(job=j, input={"job": j})) for j in jobs))
    assert all(o.body["state"] == "terminal_ok" for o in outs), [o.body for o in outs if o.body["state"] != "terminal_ok"][:2]
    assert seen == {j: ("t1", j) for j in jobs}
    with pytest.raises(ContextMissing):
        reg.resolve({"task_binding_ref": "nope"})

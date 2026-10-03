"""Mechanism A (SP-03): the extension registered before `registry_extension` owns the evaluate route; upstream
outcomes pass through unchanged. Real Core `registry_extension`, real PG."""

from __future__ import annotations

from typing import Any

import pytest
from agent_core.registry.http import registry_extension
from fastapi import FastAPI, Request
from fastapi.testclient import TestClient

from l5.test_evaluate_path import World
from l5.world import bot_actor, suite_draft
from pulso_core_runtime.evaluation.http_campaign import (
    HEADER,
    PATH,
    evaluation_admission_extension,
    single_handler_cleanup,
)

pytestmark = [pytest.mark.runtime, pytest.mark.pg]


def _app(w: World) -> FastAPI:
    app = FastAPI()
    actor = bot_actor()

    def authenticate(request: Request, authorization: str | None) -> Any:
        return actor

    evaluation_admission_extension(w.rt, authenticate, tenant_of=lambda a: "t1")(app, authenticate)
    registry_extension(w.rt.service)(app, authenticate)
    single_handler_cleanup(app, authenticate)
    return app


def _post(c: TestClient, pid: str, ref: str | None, **body: Any) -> Any:
    headers = {"Authorization": "Bearer x"} | ({HEADER: ref} if ref else {})
    return c.post(f"/v1/registry/proposals/{pid}/evaluate",
                  json={"suite_id": "disputas-suite", "suite_version": "1.0.0", **body}, headers=headers)


def test_route_table_has_exactly_one_handler_for_the_path(pg) -> None:  # type: ignore[no-untyped-def]
    app = _app(World(pg))
    matching = [r for r in app.router.routes if getattr(r, "path", None) == PATH and "POST" in r.methods]  # type: ignore[attr-defined]
    assert len(matching) == 1
    assert matching[0].endpoint.__module__.endswith("http_campaign")  # type: ignore[attr-defined]


def test_header_missing_unknown_and_mismatched_suite_do_zero_work(pg) -> None:  # type: ignore[no-untyped-def]
    w = World(pg)
    pid, chash = w.frozen_proposal()
    c = TestClient(_app(w), raise_server_exceptions=False)
    assert _post(c, pid, None).status_code == 403
    assert _post(c, pid, "ctx-none").status_code == 403
    w.admit(pid, chash)
    r = _post(c, pid, "ctx-1", suite_version="9.9.9")
    assert r.status_code == 409 and r.json()["code"] == "suite_mismatch"
    assert w.storage.jobs == 0


def test_pass_and_upstream_gate_failed_unchanged(pg) -> None:  # type: ignore[no-untyped-def]
    w = World(pg)
    pid, chash = w.frozen_proposal()
    w.admit(pid, chash)
    c = TestClient(_app(w), raise_server_exceptions=False)
    ok = _post(c, pid, "ctx-1")
    assert ok.status_code == 200 and ok.json()["verdict"] == "pass"
    jobs = w.storage.jobs
    assert _post(c, pid, "ctx-1").status_code == 200 and w.storage.jobs == jobs  # replay, no second run


def test_upstream_gate_failed_problem_json_is_unchanged(pg) -> None:  # type: ignore[no-untyped-def]
    w = World(pg)
    pid, chash = w.frozen_proposal(suite_draft(outcome="escalated"))
    w.admit(pid, chash, "ctx-f")
    bad = _post(TestClient(_app(w), raise_server_exceptions=False), pid, "ctx-f")
    assert bad.status_code == 409 and bad.headers["content-type"].startswith("application/problem+json")
    assert bad.json()["code"] == "gate_failed" and "items" in str(bad.json())
    assert w.rt.reports.list_for(pid)[0].gate_failed  # the bridge copy exists too


def test_campaign_actor_tenant_must_equal_admission_tenant(pg) -> None:  # type: ignore[no-untyped-def]
    w = World(pg)
    pid, chash = w.frozen_proposal()
    w.admit(pid, chash)  # admission tenant t1
    app = FastAPI()
    actor = bot_actor()
    auth = lambda request, authorization: actor  # noqa: E731
    evaluation_admission_extension(w.rt, auth, tenant_of=lambda a: "t2")(app, auth)
    registry_extension(w.rt.service)(app, auth)
    single_handler_cleanup(app, auth)
    r = _post(TestClient(app, raise_server_exceptions=False), pid, "ctx-1")
    assert r.status_code == 403 and r.json()["code"] == "admission_cross_tenant" and w.storage.jobs == 0

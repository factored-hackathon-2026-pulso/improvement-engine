"""Context channel gate (L3-real, DR-05): 32 concurrent invokes echo (tenant, job) with no crossing; a
ThreadPoolExecutor variant proves the signed-attr channel works where a ContextVar alone would fail."""

from __future__ import annotations

import contextvars
import threading
from concurrent.futures import ThreadPoolExecutor
from typing import Any

from agent_core.domain.shared import ToolStatus

from pulso_core_runtime.tools.context import CURRENT_BINDING, current_binding, use_binding

from .support import Env, ref, tcx

N = 32


def _setup() -> tuple[Env, list[Any]]:
    env = Env()
    ics = [env.invocation("scout", tenant=f"T{i}", job=f"J{i}", memory_snapshot_ref=f"mem-T{i}") for i in range(N)]
    return env, ics


def _assert_no_crossing(env: Env, ics: list[Any]) -> None:
    by_ref = {ic.binding_ref: ic for ic in ics}
    seen = set()
    for req, body in zip(env.backend.requests, env.backend.bodies, strict=True):
        if req.url.path.endswith("/wiki/read"):
            ic = by_ref[req.headers["X-Pulso-Binding-Ref"]]
            assert body["memory_snapshot_ref"] == ic.memory_snapshot_ref  # snapshot of ITS tenant only
            seen.add(ic.tenant_id)
        if req.url.path.endswith("core-task-bindings"):
            assert body["tenant_id"] == by_ref[body["task_binding_ref"]].tenant_id
            assert body["job_id"] == by_ref[body["task_binding_ref"]].job_id
    assert seen == {f"T{i}" for i in range(N)}
    assert sorted(env.backend.bound) == sorted((f"T{i}", f"J{i}") for i in range(N))


def test_32_concurrent_invocations_echo_their_own_tenant_and_job() -> None:
    env, ics = _setup()
    barrier = threading.Barrier(N)
    results: dict[int, tuple[Any, Any]] = {}

    def worker(i: int) -> None:
        ic = ics[i]
        barrier.wait()
        with use_binding(ic.binding_ref):  # redundant channel agrees
            bound = env.dispatcher.execute(ref("pulso/bind_context"), {}, {}, tcx(ic))
            read = env.dispatcher.execute(ref("pulso/wiki_read"), {"path": f"p{i}"}, {}, tcx(ic))
        results[i] = (bound, read)

    threads = [threading.Thread(target=worker, args=(i,)) for i in range(N)]
    [t.start() for t in threads]
    [t.join() for t in threads]
    assert len(results) == N
    for i, (bound, read) in results.items():
        assert bound.status is ToolStatus.ok and read.status is ToolStatus.ok
        assert (bound.result_full["tenant_id"], bound.result_full["job_id"]) == (f"T{i}", f"J{i}")
        assert read.result_full["entries"][0]["path"] == f"p{i}"
    _assert_no_crossing(env, ics)


def test_threadpool_attr_channel_works_and_contextvar_alone_would_fail() -> None:
    env, ics = _setup()
    with use_binding(ics[0].binding_ref):  # set on the submitting thread only
        assert current_binding() == ics[0].binding_ref
        with ThreadPoolExecutor(max_workers=8) as pool:
            # 1) a ContextVar alone is NOT propagated into pool workers (DR-05)
            leaked = list(pool.map(lambda _: CURRENT_BINDING.get(), range(N)))
            assert set(leaked) == {None}

            # 2) the signed principal attr channel needs no ContextVar and does not cross
            def call(i: int) -> Any:
                return env.dispatcher.execute(ref("pulso/bind_context"), {}, {}, tcx(ics[i]))

            outs = list(pool.map(call, range(N)))
    assert all(o.status is ToolStatus.ok for o in outs)
    assert [(o.result_full["tenant_id"], o.result_full["job_id"]) for o in outs] == [
        (f"T{i}", f"J{i}") for i in range(N)]
    # 3) a worker that DOES inherit a mismatching ContextVar is refused (never silently re-routed)
    ctx = contextvars.copy_context()
    ctx.run(CURRENT_BINDING.set, ics[1].binding_ref)
    r = ctx.run(env.dispatcher.execute, ref("pulso/bind_context"), {}, {}, tcx(ics[2]))
    assert r.status is ToolStatus.denied and r.error == "pulso:context_mismatch"


def test_tool_args_can_never_name_another_context() -> None:
    env, ics = _setup()
    env.bind(ics[0])
    r = env.dispatcher.execute(ref("pulso/wiki_read"), {"path": "a", "task_binding_ref": ics[1].binding_ref}, {},
                               tcx(ics[0]))
    assert r.status is ToolStatus.denied and env.backend.wiki_requests == 0


def test_context_is_deleted_on_terminal_and_expires() -> None:
    env, ics = _setup()
    env.bind(ics[0])
    env.contexts.remove(ics[0].binding_ref)
    r = env.dispatcher.execute(ref("pulso/wiki_read"), {"path": "a"}, {}, tcx(ics[0]))
    assert r.status is ToolStatus.denied and r.error == "pulso:context_missing"

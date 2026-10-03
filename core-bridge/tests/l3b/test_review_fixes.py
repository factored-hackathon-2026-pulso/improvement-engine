"""Independent-review regressions (L3b): duplicate create, get_write scoping, canary escaping, map races."""

from __future__ import annotations

import threading
from typing import Any

import pytest
from agent_core.domain.shared import ToolStatus

from pulso_core_runtime.facts import whitelist as wl
from pulso_core_runtime.tools.builder import write_key

from .test_protected_builder import CREATE, writer


def test_second_create_proposal_with_new_engine_key_is_denied() -> None:
    env, ic = writer()
    assert env.call(ic, "registry/create_proposal", CREATE, key="engine-1").status is ToolStatus.ok
    n = len(env.inner.writes)
    r = env.call(ic, "registry/create_proposal", CREATE, key="engine-9")
    assert r.status is ToolStatus.denied and r.error == "commitment_mismatch"
    assert len(env.inner.writes) == n
    # the replay of the very same engine key stays idempotent
    assert env.call(ic, "registry/create_proposal", CREATE, key="engine-1").status is ToolStatus.ok


def test_get_write_cannot_read_a_foreign_key() -> None:
    env, ic = writer()
    n = len(env.inner.calls)
    for foreign in ("pulso-w:" + "0" * 48, "someone-elses-key", "pulso-eval:other"):
        r = env.call(ic, "registry/get_write", {"idempotency_key": foreign})
        assert r.status is ToolStatus.denied and r.error == "commitment_mismatch"
    assert len(env.inner.calls) == n
    own = write_key(ic.command_key, "writer", 0)  # recovery after a crash: derived key of this command
    assert env.call(ic, "registry/get_write", {"idempotency_key": own}).status is ToolStatus.ok


@pytest.mark.parametrize("canary", ['say "hi"', "line1\nline2", r"backslash"])
def test_canary_with_json_escapable_chars_is_detected(canary: str) -> None:
    value: dict[str, Any] = {
        "schema_version": "1", "change_spec": {"note": f"x {canary} y"}, "rationale": "r",
        "evidence_refs": [{"id": "a1", "digest": "a" * 64, "media_type": "application/json"}],
        "alternatives": [{"id": "n", "kind": "do_nothing", "summary": "keep"}]}
    with pytest.raises(wl.FactError) as e:
        wl.validate_fact("pulso_change_spec", value, canaries=[canary])
    assert e.value.code == "pulso:canary_detected"


def test_concurrent_key_memory_does_not_lose_entries() -> None:
    env, ic = writer()
    barrier = threading.Barrier(16)

    def work(i: int) -> None:
        barrier.wait()
        env.contexts.remember_in_map(ic.binding_ref, "m", f"k{i}", i)

    ts = [threading.Thread(target=work, args=(i,)) for i in range(16)]
    [t.start() for t in ts]
    [t.join() for t in ts]
    assert len(env.contexts.recall(ic.binding_ref, "m")) == 16


def test_writer_can_reopen_and_receipt_carries_it() -> None:
    from pulso_core_runtime.stages.catalog import CATALOG, stage_allows
    assert stage_allows("writer", "registry/reopen@1.0.0")
    assert "registry/reopen@1" in CATALOG["writer"].node_tools
    facts = {"proposal": {"value": {"proposal_id": "p", "rev": 3}, "source": {"kind": "tool"}},
             "reopen_verified": {"value": {"op": "reopen", "rev_after": 3, "request_hash": "h"},
                                 "source": {"kind": "tool"}}}
    out = wl.compose_writer_receipts(facts, [{"tool": {"id": "registry/reopen"}, "idempotency_key": "k",
                                               "state": "done"}])
    assert [r["op"] for r in out["write_receipts"]] == ["reopen"] and out["state"] == "confirmed"

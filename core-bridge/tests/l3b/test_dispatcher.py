"""PulsoToolDispatcher against the D.3-generated broker fixture."""

from __future__ import annotations

import hashlib
from datetime import timedelta
from typing import Any

import pytest
from agent_core.domain.shared import ToolStatus

from pulso_core_runtime.tools.broker import PREFIX
from pulso_core_runtime.tools.context import use_binding
from pulso_core_runtime.tools.lab import normalise_sql, query_key

from .d3_fixture import D3_ROUTES
from .support import Env, ref, tcx


def confirmed(stage: str = "scout") -> tuple[Env, Any]:
    env = Env()
    ic = env.invocation(stage)
    assert env.bind(ic).status is ToolStatus.ok
    return env, ic


def test_fixture_routes_cover_the_whole_annex_d3_table() -> None:
    assert len(D3_ROUTES) == 11
    from .d3_fixture import FakeBackend
    for _m, suffix, _req, _resp in D3_ROUTES:
        import re
        assert hasattr(FakeBackend, "r_" + re.sub(r"\W+", "_", suffix).strip("_"))
    assert PREFIX == "/internal/v1/broker"


def test_bind_context_posts_cap27_body_and_returns_stage_facts() -> None:
    env = Env()
    ic = env.invocation("writer")
    r = env.bind(ic)
    assert r.status is ToolStatus.ok
    assert r.result_full == {"binding_state": "confirmed", "tenant_id": ic.tenant_id, "job_id": ic.job_id,
                             "is_scout": False, "is_verifier": False, "is_builder": True, "is_writer": True}
    body = next(b for b in env.backend.bodies if b and "task_binding_ref" in b)
    assert set(body) == {"schema_version", "tenant_id", "job_id", "command_key", "request_digest", "attempt",
                         "core_run_id", "bridge_instance_id", "task_binding_ref"}
    req = next(q for q in env.backend.requests if q.url.path.endswith("core-task-bindings"))
    assert req.headers["Idempotency-Key"] == ic.command_key and req.headers["Authorization"] == "Bearer jwt-binding"
    assert env.contexts.is_confirmed(ic.binding_ref)


def test_bind_retries_once_on_network_error_but_not_on_timeout() -> None:
    env = Env()
    env.backend.bind_mode = "network_once"
    assert env.bind(env.invocation()).status is ToolStatus.ok
    assert env.backend.count("core-task-bindings") == 2
    env2 = Env()
    env2.backend.bind_mode = "timeout"
    assert env2.bind(env2.invocation()).status is ToolStatus.denied
    assert env2.backend.count("core-task-bindings") == 1


def test_unknown_tool_and_unknown_version_are_unregistered() -> None:
    env, ic = confirmed()
    assert env.call(ic, "pulso/lab_write_scratch").error == "unregistered_tool"
    assert env.call(ic, "registry/approve").error == "unregistered_tool"
    assert env.call(ic, "registry/publish").error == "unregistered_tool"
    r = env.dispatcher.execute(ref("pulso/lab_query").model_copy(update={"version": "2.0.0"}), {"sql": "x"}, {},
                               tcx(ic))
    assert r.error == "unregistered_tool"


def test_context_missing_and_mismatch_are_denied_without_effects() -> None:
    env, ic = confirmed()
    no_ref = env.dispatcher.execute(ref("pulso/wiki_read"), {"path": "a"}, {}, tcx(ic, with_ref=False))
    assert no_ref.status is ToolStatus.denied and no_ref.error == "pulso:context_missing"
    other = env.invocation("scout", confirmed=True)
    with use_binding(other.binding_ref):  # ContextVar disagrees with the signed attr
        r = env.call(ic, "pulso/wiki_read", {"path": "a"})
    assert r.status is ToolStatus.denied and r.error == "pulso:context_mismatch"
    assert env.backend.wiki_requests == 0
    forged = tcx(ic)
    forged.principal.attrs["tenant"] = "someone-else"
    assert env.dispatcher.execute(ref("pulso/wiki_read"), {"path": "a"}, {}, forged).error == "pulso:context_mismatch"


def test_stage_allow_list_and_closed_args() -> None:
    env, ic = confirmed("builder_design")
    assert env.call(ic, "pulso/wiki_transform", {"source_ref": "a", "transform": "[]"}).error == "tool_not_allowed"
    assert env.call(ic, "pulso/wiki_read", {"path": "a", "tenant_id": "other"}).error == "invalid_args"
    assert env.call(ic, "pulso/wiki_read", {"path": "a", "task_binding_ref": "x"}).status is ToolStatus.denied
    assert env.call(ic, "pulso/wiki_read", {}).error == "invalid_args"
    assert env.backend.wiki_requests == 0


def test_lab_query_flow_containers_key_and_unknown_total() -> None:
    env, ic = confirmed()
    env.backend.total_rows = None
    env.backend.query_polls_before_done = 2
    r = env.call(ic, "pulso/lab_query", {"sql": "select  1;"})
    assert r.status is ToolStatus.ok, r.error
    v = r.result_full
    assert set(v["containers"]) == {"public", "financial", "untrusted_text"}
    assert v["containers"]["financial"]["rows"] == [["10.50"], ["3.00"]]
    assert v["total_rows"] is None  # never rendered as 0
    submit = next(b for b in env.backend.bodies if b and "query_key" in b)
    assert submit["query_key"] == query_key(ic.binding_ref, "sess-1", 3, "select 1")
    assert submit["query_key"] == hashlib.sha256(
        f"{ic.binding_ref}|sess-1|3|{normalise_sql('select  1;')}".encode()).hexdigest()
    assert env.backend.count("/lab/queries/") == 3  # internal poll
    # the same query twice re-uses the session and the same key
    env.call(ic, "pulso/lab_query", {"sql": "SELECT 1".replace("SELECT", "select")})
    assert sum(1 for q in env.backend.requests if q.url.path.endswith("/lab/sessions")) == 1


@pytest.mark.parametrize(("cls", "error"), [("pii_direct", "pii_class_received"), ("secret", "unclassified_column"),
                                           (None, "unclassified_column")])
def test_unclassified_and_pii_columns_are_refused(cls: str | None, error: str) -> None:
    env, ic = confirmed()
    env.backend.columns = [{"name": "x", "type": "text", "data_class": cls}]
    env.backend.rows = [["v"]]
    r = env.call(ic, "pulso/lab_query", {"sql": "select x"})
    assert r.status is ToolStatus.error and r.error == error and r.result_full is None
    if cls == "pii_direct":
        assert (ic.binding_ref, "pii_class_received") in env.dispatcher.deps.leaks


def test_result_paging_caps_rows_and_never_invents_a_cursor() -> None:
    env, ic = confirmed()
    env.backend.columns = [{"name": "event_count", "type": "int", "data_class": "public"}]
    env.backend.rows = [[i] for i in range(500)]
    env.backend.truncated = False
    v = env.call(ic, "pulso/lab_query", {"sql": "select 1"}).result_full
    assert v["row_count"] == 200 and v["truncated"] is True and v["next_cursor"] is None


def test_lab_poll_budget_exhaustion_is_timeout_not_error() -> None:
    env, ic = confirmed()
    env.backend.query_polls_before_done = 10**6
    env.broker.poll_budget_s = 0.0
    r = env.call(ic, "pulso/lab_query", {"sql": "select 1"})
    assert r.status is ToolStatus.timeout


def test_wiki_artifact_tools_use_context_snapshot_and_record_refs() -> None:
    env, ic = confirmed("verifier")
    r = env.call(ic, "pulso/wiki_read", {"path": "notes/a.md"})
    assert r.status is ToolStatus.ok
    sent = next(b for b in env.backend.bodies if b and "paths" in b)
    assert sent == {"memory_snapshot_ref": "mem-1", "paths": ["notes/a.md"]}
    assert env.call(ic, "pulso/wiki_explore", {"query": "q"}).status is ToolStatus.ok
    tr = env.call(ic, "pulso/wiki_transform", {"source_ref": "b" * 64, "transform": '[{"op":"put","path":"x","content":"c"}]'})
    assert tr.status is ToolStatus.ok and tr.result_full["scratch_ref"] == "scr-1"
    assert env.call(ic, "pulso/wiki_transform", {"source_ref": "b", "transform": "not json"}).status is ToolStatus.denied
    assert "scr-1" in env.contexts.seen_refs(ic.binding_ref)


def test_artifact_get_verifies_digest_size_and_final_lock() -> None:
    from agent_core.domain.json import canonical_bytes
    env, ic = confirmed("verifier")
    content = {"a": 1}
    digest = hashlib.sha256(canonical_bytes(content)).hexdigest()
    env.backend.artifacts["art-1"] = {"schema_version": "1", "artifact": {"id": "art-1", "digest": digest,
                                      "media_type": "application/json"}, "encoding": "json", "content": content,
                                      "byte_length": 8}
    r = env.call(ic, "pulso/artifact_get", {"artifact_ref": "art-1"})
    assert r.status is ToolStatus.ok and r.result_full["content"] == content
    assert "art-1" in env.contexts.seen_refs(ic.binding_ref)
    env.backend.artifacts["art-2"] = {**env.backend.artifacts["art-1"], "byte_length": (1 << 20) + 1}
    assert env.call(ic, "pulso/artifact_get", {"artifact_ref": "art-2"}).error == "pulso:artifact_too_large"
    bad = {**env.backend.artifacts["art-1"], "artifact": {"id": "art-3", "digest": "0" * 64, "media_type": "x"}}
    env.backend.artifacts["art-3"] = bad
    assert env.call(ic, "pulso/artifact_get", {"artifact_ref": "art-3"}).error == "pulso:artifact_digest_mismatch"
    env.backend.artifacts["art-4"] = {**env.backend.artifacts["art-1"],
                                      "artifact": {"id": "art-4", "digest": digest, "media_type": "x", "final_locked": True}}
    assert env.call(ic, "pulso/artifact_get", {"artifact_ref": "art-4"}).status is ToolStatus.denied
    assert env.call(ic, "pulso/artifact_get", {"artifact_ref": "missing"}).status is ToolStatus.error


@pytest.mark.parametrize("mode", ["5xx", "timeout"])
def test_authorization_down_denies_every_effect(mode: str) -> None:
    env, ic = confirmed()
    env.backend.auth_mode = mode
    before = env.backend.broker_effect_calls
    r = env.call(ic, "pulso/lab_query", {"sql": "select 1"})
    assert r.status is ToolStatus.denied and r.error == "pulso:authorization_denied"
    assert env.backend.broker_effect_calls == before


def test_authorization_false_denies_and_positive_cache_is_per_resource_and_expires() -> None:
    env, ic = confirmed()
    env.backend.auth_allowed = False
    assert env.call(ic, "pulso/wiki_read", {"path": "a"}).status is ToolStatus.denied
    env.backend.auth_allowed = True
    assert env.call(ic, "pulso/wiki_read", {"path": "a"}).status is ToolStatus.ok
    n = env.backend.count("authorizations")
    env.call(ic, "pulso/wiki_read", {"path": "b"})  # same snapshot resource -> cached
    assert env.backend.count("authorizations") == n
    env.call(ic, "pulso/lab_query", {"sql": "select 1"})  # other resource -> a new check
    assert env.backend.count("authorizations") == n + 1
    env.backend.auth_allowed = False  # revoked but cached until valid_until
    assert env.call(ic, "pulso/wiki_read", {"path": "c"}).status is ToolStatus.ok
    env.contexts._clock = lambda: __import__("datetime").datetime.now(__import__("datetime").UTC) + timedelta(hours=1)
    # context itself expired by then (TTL) -> context_missing, nothing cached survives
    assert env.call(ic, "pulso/wiki_read", {"path": "d"}).status is ToolStatus.denied


def test_stale_valid_until_is_denied() -> None:
    env, ic = confirmed()
    env.backend.auth_valid_for_s = -5
    assert env.call(ic, "pulso/wiki_read", {"path": "a"}).error == "pulso:authorization_denied"


def test_ids_make_unique_call_ids_and_definition_reads_registry() -> None:
    env, ic = confirmed()
    a = env.call(ic, "pulso/wiki_read", {"path": "a"}).call_id
    b = env.call(ic, "pulso/wiki_read", {"path": "a"}).call_id
    assert a != b
    assert env.dispatcher.definition(ref("pulso/wiki_read")).id == "pulso/wiki_read"

"""Writer path on the composed runtime, in-process, real PG16: create_proposal -> put_draft -> freeze (committed
operations, derived keys, broker authorisation) -> admission through the real route -> evaluate-only invocation ->
NATIVE evaluate through `ProtectedBuilderToolExecutor` + `FlowEvaluationGate` + the per-admission service clone,
with the fixture bank (demo suite `resuelto`). The model/decision providers are scripted doubles (`conftest`);
registry, stores, evaluator, admission gate, tools and the dispatcher are real.

Also: the sealed `registry_mutation_commitment` of `CoreTaskInvocation` reaches the frozen context through the real
invoke route, and a failed writer run is `manual_reconcile`, never `terminal_failed` (decision L3a review 4)."""

from __future__ import annotations

import hashlib
from datetime import UTC, datetime, timedelta
from typing import Any

import psycopg
import pytest
from agent_core.domain import EntityRef
from agent_core.registry import EvalSuite
from agent_core.registry.entities import content_hash

from integration.conftest import WORLD, Composed
from l3a.helpers import body, idem_key
from l3b.support import tcx
from l5.world import prompt_draft, suite_draft
from pulso_core_runtime.tools.builder import eval_key, put_draft_digest, write_key
from pulso_core_runtime.tools.context import InvocationContext, RegistryMutationCommitment

pytestmark = [pytest.mark.integration, pytest.mark.pg]
OPS = ("create_proposal", "put_draft", "freeze")


def _ic(c: Composed, n: str, commitment: RegistryMutationCommitment) -> InvocationContext:
    ic = InvocationContext(
        tenant_id="t1", job_id=f"job-{n}", stage="writer", attempt=1, binding_ref=f"bind-{n}",
        command_key=f"cmd-{n}", request_digest="r" * 64, bridge_instance_id="bridge-1",
        expires_at=datetime.now(UTC) + timedelta(minutes=15), commitment=commitment)
    c.contexts.register(ic)
    c.contexts.confirm(ic.binding_ref)
    return ic


def _call(c: Composed, ic: InvocationContext, tool: str, args: dict[str, Any], key: str | None = None) -> Any:
    return c.ports.tools.execute(EntityRef(id=tool, version="1.0.0"), args, {}, tcx(ic), key)


def _changes() -> list[dict[str, Any]]:
    return [d.model_dump(mode="json") for d in (prompt_draft(), suite_draft())]


def _write_flow(c: Composed) -> tuple[str, str, str]:
    """Runs the committed write sequence; returns (proposal_id, candidate_hash, suite_digest)."""
    changes = _changes()
    commit = RegistryMutationCommitment(
        mode="write", proposal_id=None, expected_rev=None, base_release_id="rel-demo", evaluate_enabled=False,
        create_agent_id="atencion", create_origin="builder_chat", create_title="pulso-key:k1",
        put_draft_digest=put_draft_digest(None, None, changes), operations=OPS)
    ic = _ic(c, "w1", commit)
    created = _call(c, ic, "registry/create_proposal",
                    {"agent_id": "atencion", "origin": "builder_chat", "title": "pulso-key:k1"}, "eng-1")
    assert created.status.value == "ok", created
    pid, rev = created.result_full["proposal_id"], created.result_full["rev"]
    assert created.result_full["key_digest"] == hashlib.sha256(write_key("cmd-w1", "writer", 0).encode()).hexdigest()
    put = _call(c, ic, "registry/put_draft", {"proposal_id": pid, "expected_rev": rev, "changes": changes}, "eng-2")
    assert put.status.value == "ok", put
    frozen = _call(c, ic, "registry/freeze", {"proposal_id": pid}, "eng-3")
    assert frozen.status.value == "ok", frozen
    # evaluate is not part of this commitment: denied at the executor, zero effect
    denied = _call(c, ic, "registry/evaluate", {"proposal_id": pid, "suite_id": "disputas-suite"}, "eng-4")
    assert denied.status.value == "denied" and denied.error == "tool_not_allowed"
    suite = EvalSuite.model_validate(changes[1]["content"])
    return pid, frozen.result_full["candidate_hash"], content_hash(suite)


def _admit(c: Composed, pid: str, chash: str, sdigest: str, ref: str, n: str) -> Any:
    return c.client.post("/internal/v1/evaluation/admissions", headers=c.headers(
        "evaluation_admit", job_id=f"job-{n}"), json={
        "schema_version": "1", "evaluation_context_ref": ref, "binding_ref": f"bind-{n}", "proposal_id": pid,
        "candidate_hash": chash, "suite_id": "disputas-suite", "suite_version": "1.0.0", "suite_digest": sdigest,
        "evaluation_attempt": 1, "budget_ref": "bud-1",
        "deadline": (datetime.now(UTC) + timedelta(hours=1)).isoformat(), "request_digest": "d" * 64})


def _eval_only(c: Composed, pid: str, ref: str, n: str) -> InvocationContext:
    return _ic(c, n, RegistryMutationCommitment(
        mode="evaluate_only", proposal_id=pid, expected_rev=None, base_release_id="rel-demo",
        evaluate_enabled=True, evaluation_context_ref=ref, operations=()))


def _eval_runs(c: Composed) -> int:
    with psycopg.connect(c.pg.eval) as conn:
        return int(conn.execute("SELECT count(*) FROM runs").fetchone()[0])  # type: ignore[index]


def test_writer_create_put_freeze_then_native_evaluate_end_to_end(composed: Composed) -> None:
    pid, chash, sdigest = _write_flow(composed)
    runs_before = _eval_runs(composed)
    assert runs_before == 0
    adm = _admit(composed, pid, chash, sdigest, "ctx-w-1", "w2")
    assert adm.status_code == 201, adm.text
    ic = _eval_only(composed, pid, "ctx-w-1", "w2")
    # evaluate-only denies every mutator at the executor
    blocked = _call(composed, ic, "registry/create_proposal",
                    {"agent_id": "atencion", "origin": "builder_chat", "title": "x"}, "eng-9")
    assert blocked.status.value == "denied" and blocked.error == "tool_not_allowed"
    out = _call(composed, ic, "registry/evaluate", {"proposal_id": pid, "suite_id": "disputas-suite"}, "eng-5")
    assert out.status.value == "ok", out
    assert out.result_full["verdict"] == "pass" and out.result_full["eval_run_ref"]
    assert out.result_full["report_digest"]
    assert _eval_runs(composed) > runs_before  # ran in the isolated eval DB, through the real engine
    # replay of the same admission: stored result, no second run
    jobs = _eval_runs(composed)
    again = _call(composed, ic, "registry/evaluate", {"proposal_id": pid, "suite_id": "disputas-suite"}, "eng-5")
    assert again.status.value == "ok" and again.result_full["report_digest"] == out.result_full["report_digest"]
    assert _eval_runs(composed) == jobs
    # the broker was asked before the writes and before the evaluate
    ops = [b["operation"] for r, b in zip(composed.loop.backend.requests, composed.loop.backend.bodies, strict=True)
           if r.url.path.endswith("/authorizations/check") and b]
    assert {"registry/create_proposal", "registry/put_draft", "registry/freeze", "native_evaluate"} <= set(ops)
    # both broker checks (tool-side and admission gate) carry the SAME canonical digest (ADR 0002)
    native = [b["payload_digest"] for r, b in zip(composed.loop.backend.requests, composed.loop.backend.bodies,
                                                  strict=True)
              if r.url.path.endswith("/authorizations/check") and b and b["operation"] == "native_evaluate"]
    assert len(native) >= 2 and len(set(native)) == 1, native
    # the evaluate key is the admission-derived one
    assert eval_key("ctx-w-1") == "pulso-eval:ctx-w-1"
    r = composed.client.get("/internal/v1/version", headers=composed.headers("version_probe"))
    assert r.status_code == 200


def test_evaluate_without_admission_fails_closed(composed: Composed) -> None:
    pid, _chash, _ = _write_flow(composed)
    ic = _eval_only(composed, pid, "ctx-none", "w3")
    out = _call(composed, ic, "registry/evaluate", {"proposal_id": pid, "suite_id": "disputas-suite"}, "eng-6")
    assert out.status.value == "denied" and "admission" in str(out.error)
    assert _eval_runs(composed) == 0


def test_concurrency_gate_serialises_evaluations(composed: Composed) -> None:
    """One permit (`PULSO_EVAL_PERMITS=1`): two admitted evaluations in parallel never overlap."""
    import threading

    from pulso_core_runtime.evaluation.native import EvaluationGate

    gate: EvaluationGate = composed.ports.tools  # placeholder to keep type checkers quiet
    del gate
    # the composed gate is the one given to the evaluation runtime: observe it through the harness slot
    import pulso_core_runtime.evaluation.native as native

    active, peak, lock = [0], [0], threading.Lock()
    real_slot = native.EvaluationGate.slot

    class Counting:
        def __init__(self, inner: Any) -> None:
            self.inner = inner

        def __enter__(self) -> Any:
            r = self.inner.__enter__()
            with lock:
                active[0] += 1
                peak[0] = max(peak[0], active[0])
            return r

        def __exit__(self, *a: Any) -> Any:
            with lock:
                active[0] -= 1
            return self.inner.__exit__(*a)

    native.EvaluationGate.slot = lambda self: Counting(real_slot(self))  # type: ignore[method-assign,assignment]
    try:
        results: list[Any] = []
        prepared = []
        for n in ("c1", "c2"):
            pid, chash, sd = _write_flow_n(composed, n)
            assert _admit(composed, pid, chash, sd, f"ctx-{n}", f"e{n}").status_code == 201
            prepared.append((pid, _eval_only(composed, pid, f"ctx-{n}", f"e{n}")))

        def run(pid: str, ic: InvocationContext, n: int) -> None:
            results.append(_call(composed, ic, "registry/evaluate",
                                 {"proposal_id": pid, "suite_id": "disputas-suite"}, f"eng-c{n}"))

        threads = [threading.Thread(target=run, args=(pid, ic, i)) for i, (pid, ic) in enumerate(prepared)]
        [t.start() for t in threads]
        [t.join() for t in threads]
    finally:
        native.EvaluationGate.slot = real_slot  # type: ignore[method-assign]
    assert len(results) == 2 and all(r.status.value == "ok" for r in results), results
    assert peak[0] == 1


def _write_flow_n(c: Composed, n: str) -> tuple[str, str, str]:
    """Second independent proposal (distinct command key / title)."""
    changes = _changes()
    commit = RegistryMutationCommitment(
        mode="write", proposal_id=None, expected_rev=None, base_release_id="rel-demo", evaluate_enabled=False,
        create_agent_id="atencion", create_origin="builder_chat", create_title=f"pulso-key:{n}",
        put_draft_digest=put_draft_digest(None, None, changes), operations=OPS)
    ic = _ic(c, f"w{n}", commit)
    created = _call(c, ic, "registry/create_proposal",
                    {"agent_id": "atencion", "origin": "builder_chat", "title": f"pulso-key:{n}"}, f"{n}-1")
    pid, rev = created.result_full["proposal_id"], created.result_full["rev"]
    _call(c, ic, "registry/put_draft", {"proposal_id": pid, "expected_rev": rev, "changes": changes}, f"{n}-2")
    frozen = _call(c, ic, "registry/freeze", {"proposal_id": pid}, f"{n}-3")
    suite = EvalSuite.model_validate(changes[1]["content"])
    return pid, frozen.result_full["candidate_hash"], content_hash(suite)


@pytest.mark.skipif(not WORLD.is_dir(), reason="agent-core-assets world absent")
def test_invoke_seals_commitment_into_context_and_failed_writer_is_manual_reconcile(composed: Composed) -> None:
    payload = body(stage="writer", agent_id="pulso-writer", agent_version="1.0.0",
                   release_id=composed.release_ids["writer"], logical="wc", memory_snapshot_ref=None,
                   input={"draft_plan_ref": "artifact:plan", "proposal_id": None, "base_release_id": "rel-demo",
                          "evaluate_enabled": False},
                   registry_mutation_commitment={"mode": "write", "base_release_id": "rel-demo",
                                                 "create_agent_id": "atencion", "create_origin": "builder_chat",
                                                 "create_title": "pulso-key:inv", "operations": list(OPS)})
    key = idem_key("t1", "j1", "writer", 1, "wc")
    r = composed.client.post("/internal/v1/core-tasks/invoke", json=payload,
                             headers={**composed.headers("core_task_invoke"), "Idempotency-Key": key})
    assert r.status_code in (200, 202, 409, 422), r.text
    out = r.json()
    if r.status_code == 422:
        pytest.fail(f"commitment DTO rejected: {out}")
    ic = composed.contexts.lookup(out["task_binding_ref"])  # kept: a writer that is not provably clean stays open
    assert ic.commitment is not None and ic.commitment.operations == OPS and ic.commitment.mode == "write"
    assert out["state"] == "manual_reconcile", out  # the unvalidated-slot escalation after possible effects
    assert out["state"] != "terminal_failed"


def test_commitment_on_non_writer_stage_is_rejected(composed: Composed) -> None:
    payload = body(registry_mutation_commitment={"mode": "write"})
    key = idem_key("t1", "j1", "scout", 1, "k")
    r = composed.client.post("/internal/v1/core-tasks/invoke", json=payload,
                             headers={**composed.headers("core_task_invoke"), "Idempotency-Key": key})
    assert r.status_code == 422 and r.json()["code"] == "pulso:invalid_request"


def test_production_reconciler_is_wired_with_binding_lookup_and_write_probe(composed: Composed) -> None:
    """Reconcile paths (2)/(3) run in production: expected write keys come from the sealed operations, the probe
    reads the real registry `get_write`, and a created proposal's derived key is found (adopted, not redone)."""
    from pulso_core_runtime.adapters import ServiceWriteProbe, expected_write_keys

    pid, _c, _s = _write_flow(composed)
    from pulso_core_runtime.store.receipts import ReceiptStore
    store = ReceiptStore(composed.pg.runtime)
    probe = ServiceWriteProbe(lambda: composed.registry_service)
    assert probe.get_write(write_key("cmd-w1", "writer", 0)) is not None  # create_proposal key is readable
    assert probe.get_write(write_key("cmd-w1", "writer", 9)) is None
    assert callable(expected_write_keys(store)) and pid


@pytest.mark.skipif(not WORLD.is_dir(), reason="agent-core-assets world absent")
def test_writer_flow_reads_inputs_from_bind_facts_end_to_end(composed: Composed) -> None:
    """Core 1.3.0 never validates run-input slots: the writer Flow reads `proposal_id`, `base_release_id`,
    `evaluate_enabled` and the draft-plan ref from the `binding` fact provided by `pulso/bind_context`, then runs
    the committed create -> put -> validate -> freeze sequence and ends without evaluating (evaluate_enabled=false)."""
    changes = _changes()
    plan = {"agent_id": "atencion", "title": "pulso-key:flow", "changes": changes}
    from agent_core.domain.json import canonical_bytes
    digest = hashlib.sha256(canonical_bytes(plan)).hexdigest()
    composed.loop.backend.artifacts["plan-1"] = {
        "schema_version": "1", "artifact": {"id": "plan-1", "digest": digest, "media_type": "application/json"},
        "encoding": "json", "content": plan, "byte_length": len(canonical_bytes(plan))}
    payload = body(stage="writer", agent_id="pulso-writer", agent_version="1.0.0",
                   release_id=composed.release_ids["writer"], logical="wflow", memory_snapshot_ref=None,
                   input={"draft_plan_ref": "plan-1", "proposal_id": None, "base_release_id": "rel-demo",
                          "evaluate_enabled": False},
                   registry_mutation_commitment={
                       "mode": "write", "base_release_id": "rel-demo", "create_agent_id": "atencion",
                       "create_origin": "builder_chat", "create_title": "pulso-key:flow",
                       "put_draft_digest": put_draft_digest(None, None, changes), "operations": list(OPS)})
    key = idem_key("t1", "j1", "writer", 1, "wflow")
    r = composed.client.post("/internal/v1/core-tasks/invoke", json=payload,
                             headers={**composed.headers("core_task_invoke"), "Idempotency-Key": key})
    out = r.json()
    ops = [b["operation"] for rq, b in zip(composed.loop.backend.requests, composed.loop.backend.bodies, strict=True)
           if rq.url.path.endswith("/authorizations/check") and b]
    assert out["state"] == "terminal_ok", (out, ops)
    assert [o for o in ops if o.startswith("registry/")][:3] == [
        "registry/create_proposal", "registry/put_draft", "registry/freeze"]
    assert "native_evaluate" not in ops  # evaluate_enabled=false: the Flow ended at `chk_evaluate`


def _plan(composed: Composed, name: str, title: str) -> tuple[str, list[dict[str, Any]]]:
    changes = _changes()
    plan = {"agent_id": "atencion", "title": title, "changes": changes}
    from agent_core.domain.json import canonical_bytes
    digest = hashlib.sha256(canonical_bytes(plan)).hexdigest()
    composed.loop.backend.artifacts[name] = {
        "schema_version": "1", "artifact": {"id": name, "digest": digest, "media_type": "application/json"},
        "encoding": "json", "content": plan, "byte_length": len(canonical_bytes(plan))}
    return name, changes


@pytest.mark.skipif(not WORLD.is_dir(), reason="agent-core-assets world absent")
def test_evaluate_only_invocation_over_http_reaches_native_report_and_never_reopens(composed: Composed) -> None:
    """First RED of the evaluate-only Flow branch (plan 17.3.3 / A04): write run (create/put/freeze) through the
    invoke route, admission, then an evaluate-only invocation of the SAME `pulso-writer@1.0.0` for the frozen proposal
    must run `registry/evaluate` (evaluate -> verify -> end), never `registry/reopen`."""
    plan_ref, changes = _plan(composed, "plan-eo", "pulso-key:eo")
    wkey = idem_key("t1", "j1", "writer", 1, "weo")
    wr = composed.client.post("/internal/v1/core-tasks/invoke", headers={
        **composed.headers("core_task_invoke"), "Idempotency-Key": wkey}, json=body(
        stage="writer", agent_id="pulso-writer", agent_version="1.0.0", release_id=composed.release_ids["writer"],
        logical="weo", memory_snapshot_ref=None,
        input={"draft_plan_ref": plan_ref, "proposal_id": None, "base_release_id": "rel-demo",
               "evaluate_enabled": False},
        registry_mutation_commitment={
            "mode": "write", "base_release_id": "rel-demo", "create_agent_id": "atencion",
            "create_origin": "builder_chat", "create_title": "pulso-key:eo",
            "put_draft_digest": put_draft_digest(None, None, changes), "operations": list(OPS)})).json()
    assert wr["state"] == "terminal_ok", wr
    with psycopg.connect(composed.pg.runtime) as conn:
        row = conn.execute("select proposal_id, proposal_json::json->>'candidate_hash' from reg_proposals "
                           "order by (proposal_json::json->>'rev')::int desc limit 1").fetchone()
    pid, chash = row  # type: ignore[misc]
    assert chash
    suite = EvalSuite.model_validate(changes[1]["content"])
    ekey = idem_key("t1", "j2", "writer", 1, "eoeval")
    # the binding ref of an invocation is sha256(tenant|Idempotency-Key): Codex admits against it before dispatch
    from pulso_core_runtime.invoke.models import sha256_text
    adm = composed.client.post("/internal/v1/evaluation/admissions", headers=composed.headers(
        "evaluation_admit", job_id="j2"), json={
        "schema_version": "1", "evaluation_context_ref": "ctx-eo", "binding_ref": sha256_text(f"t1|{ekey}"),
        "proposal_id": pid, "candidate_hash": chash, "suite_id": "disputas-suite", "suite_version": "1.0.0",
        "suite_digest": content_hash(suite), "evaluation_attempt": 1, "budget_ref": "bud-1",
        "deadline": (datetime.now(UTC) + timedelta(hours=1)).isoformat(), "request_digest": "d" * 64})
    assert adm.status_code == 201, adm.text
    runs0 = _eval_runs(composed)
    n_before = len(composed.loop.backend.bodies)
    er = composed.client.post("/internal/v1/core-tasks/invoke", headers={
        **composed.headers("core_task_invoke", job_id="j2"), "Idempotency-Key": ekey}, json=body(
        stage="writer", agent_id="pulso-writer", agent_version="1.0.0", release_id=composed.release_ids["writer"],
        logical="eoeval", job="j2", memory_snapshot_ref=None,
        input={"draft_plan_ref": plan_ref, "proposal_id": pid, "base_release_id": "rel-demo",
               "evaluate_enabled": True, "evaluation_suite_id": "disputas-suite", "evaluation_suite_version": "1.0.0"},
        registry_mutation_commitment={
            "mode": "evaluate_only", "proposal_id": pid, "base_release_id": "rel-demo", "evaluate_enabled": True,
            "evaluation_context_ref": "ctx-eo", "operations": []})).json()
    ops = [b["operation"] for b in composed.loop.backend.bodies[n_before:] if b and "operation" in b]
    assert er["state"] == "terminal_ok", (er, ops)
    assert "registry/reopen" not in ops and "native_evaluate" in ops
    assert _eval_runs(composed) > runs0
    got = composed.client.get(f"/internal/v1/core-tasks/{er['core_run_id']}",
                              headers=composed.headers("core_task_read", job_id="j2"))
    assert got.status_code == 200, got.text
    rec = got.json()
    receipts = rec["result"]["facts"]["pulso_writer_receipts"]["value"] if "result" in rec else rec
    assert receipts["proposal_id"] == pid and receipts["candidate_hash"] == chash and receipts["state"] == "confirmed"
    native = receipts["native_evaluation"]
    assert native["verdict"] == "pass" and native["eval_run_ref"] and len(native["report_digest"]) == 64
    assert [w["op"] for w in receipts["write_receipts"]] == ["evaluate"]

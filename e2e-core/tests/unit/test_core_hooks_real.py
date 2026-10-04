"""INT0 (steps 5-9): `RealCore`, the real-Core hooks of the thread, against fakes of the bridge, the engine (writer
stage), the registry (/v1/registry) and the human authorizer. Live evidence: tests/live/test_08_thread01_core_hooks.py."""
from types import SimpleNamespace

import pytest

from claude_standin import core_hooks as H
from test_cmppy_compile import WORLD, add, rep

CAND = "ab" * 32
BASE = "rel-base"


class Resp:
    def __init__(self, status=200, body=None):
        self.status_code, self._b, self.text = status, body or {}, ""

    def json(self):
        return self._b


class FakeEngine:
    def __init__(self, cand=CAND):
        self.cand, self.sealed, self.stages, self.releases = cand, {}, [], {"pulso-writer": "rel-writer"}
        self.eval_verdict = "pass"
        self.bridge = SimpleNamespace(admit=lambda tenant, job, body: Resp(201, {"state": "admitted"}))

    def seal(self, art_id, content):
        self.sealed[art_id] = content
        return art_id

    def configure(self, **cfg):
        pass

    def stage(self, stage, job, logical, agent_id, input, **extra):
        self.stages.append((stage, job, agent_id, input, extra))
        return SimpleNamespace(out={"core_run_id": "run-1", "task_binding_ref": "bind-1"}, response=Resp())

    def facts(self, run_id):
        evaluating = bool(self.stages) and self.stages[-1][3].get("evaluate_enabled")
        native = {"verdict": self.eval_verdict, "eval_run_ref": "er-1", "report_digest": "d" * 64} if evaluating else None
        return {"pulso_writer_receipts": {"value": {"proposal_id": "prop-1", "candidate_hash": self.cand,
                                                    "native_evaluation": native,
                                                    "write_receipts": [{"op": o, "verified": True} for o in
                                                                       ("create_proposal", "put_draft", "freeze")]}}}


class FakeBridge:
    def __init__(self, arm_status="completed"):
        self.arms, self.arm_status, self.aliases = [], arm_status, {"prod": BASE, "staging": BASE}

    def call(self, method, path, op, tenant, json=None, **kw):
        if op == "credentials":
            return Resp(200, {"jws": "bot.jws.x"})
        if op == "aliases":
            a = path.rsplit("/", 1)[-1]
            return Resp(200, {"release_id": self.aliases[a], "alias": a})
        raise AssertionError(op)

    def arm_run(self, tenant, body):
        self.arms.append(body)
        return Resp(200, {"execution_id": "arm-" + "0" * 32, "status": self.arm_status, "closed_early": False,
                          "cost_known": True, "oracle_ref": None, "final_state_ref": "final:bank-1",
                          "arm": body["arm"], "case_ref": body["case_ref"], "effect_receipts": [], "reason": None})


class FakeRegistry:
    def __init__(self, bridge):
        self.calls, self.bridge, self.state, self.needs_eval = [], bridge, "candidate", False

    def __call__(self, method, path, bearer, **kw):
        self.calls.append((method, path, bearer, kw))
        if method == "GET" and path.startswith("/proposals/"):
            return Resp(200, {"proposal": {"rev": 3, "state": self.state, "candidate_hash": CAND}})
        if path.endswith("/approve"):
            if kw["json"]["candidate_hash"] != CAND:
                return Resp(409, {"code": "candidate_changed"})
            if self.state == "approved" or self.needs_eval:
                return Resp(409, {"code": "illegal_transition"})
            self.state = "approved"
            return Resp(200, {"actor": "local-supervisor", "decision": "approved", "candidate_hash": CAND})
        if path.endswith("/publish"):
            if self.needs_eval:
                return Resp(409, {"code": "illegal_transition"})
            self.bridge.aliases["staging"] = "rel-new"
            return Resp(200, {"release_id": "rel-new"})
        raise AssertionError(path)


def authorizer(log):
    def auth(operation, target):
        log.append((operation, target))
        return f"jws.{operation}.sig"
    return auth


def ctx():
    ops = [{k: o[k] for k in ("op", "target_kind", "target_ref", "new_ref", "precondition_digest")} for o in (rep(), add())]
    return SimpleNamespace(out={"world": WORLD, "compiled": {"draft_plan": {"operations": ops, "digest": "sha256:" + CAND}}})


def core(engine=None, bridge=None, log=None):
    bridge = bridge or FakeBridge()
    reg = FakeRegistry(bridge)
    rc = H.RealCore(engine=engine or FakeEngine(), bridge=bridge, registry=reg,
                    authorize=authorizer(log if log is not None else []), world=WORLD, tenant="t1", agent_id="atencion-tarea")
    return rc, bridge, reg


def test_freeze_runs_the_writer_on_the_threads_own_draft_and_binds_the_hash():
    rc, _, _ = core()
    c = ctx()
    fz = rc.freeze(c)
    stage, job, agent, inp, extra = rc.engine.stages[0]
    assert (stage, agent) == ("writer", "pulso-writer")
    plan = rc.engine.sealed[inp["draft_plan_ref"]]
    assert plan["agent_id"] == "atencion-tarea" and [x["kind"] for x in plan["changes"]] == ["prompt", "eval_suite"]
    assert plan["changes"][0]["content"]["version"] == "2.0.0"
    assert inp["base_release_id"] == BASE and inp["evaluate_enabled"] is False
    com = extra["registry_mutation_commitment"]
    assert com["create_agent_id"] == "atencion-tarea" and com["operations"] == ["create_proposal", "put_draft", "freeze"]
    assert fz.proposal_id == "prop-1" and fz.candidate_hash == CAND and fz.binding_ref == "bind-1"
    assert rc.freeze(c) is fz and len(rc.engine.stages) == 1  # memoised: one proposal per thread


def test_freeze_refuses_when_core_froze_something_else_than_the_dry_run_digest():
    rc, _, _ = core(engine=FakeEngine(cand="cd" * 32))
    with pytest.raises(RuntimeError, match="differs from the draft digest"):
        rc.freeze(ctx())


def test_run_arms_runs_base_and_candidate_per_scenario_and_maps_reports_to_runs():
    rc, bridge, _ = core()
    out = rc.run_arms(ctx())
    n = len(out["base"])
    assert n >= 2 and len(out["candidate"]) == n
    assert [r["case_ref"] for r in out["base"]] == [r["case_ref"] for r in out["candidate"]]
    kinds = {(b["arm"], b["target"]["kind"]) for b in bridge.arms}
    assert kinds == {("baseline", "published_release"), ("candidate", "frozen_candidate")}
    cand = next(b for b in bridge.arms if b["arm"] == "candidate")
    assert cand["target"]["candidate_hash"] == CAND and cand["target"]["expected_rev"] == 3
    assert all(r["status"] == "completed" and r["cost_known"] is True and r["oracle_ref"] for r in out["base"] + out["candidate"])
    assert len({b["idempotency_key"] for b in bridge.arms}) == len(bridge.arms)


def test_a_failed_infra_arm_is_not_evidence_and_raises():
    rc, _, _ = core(bridge=FakeBridge(arm_status="failed_infra"))
    with pytest.raises(RuntimeError, match="failed_infra"):
        rc.run_arms(ctx())


def test_candidate_failed_is_a_failed_run_not_an_error():
    rc, _, _ = core(bridge=FakeBridge(arm_status="candidate_failed"))
    assert all(r["status"] == "failed" for r in rc.run_arms(ctx())["base"])


def test_approve_binds_the_human_jws_to_the_proposal_hash_and_proves_tamper_and_replay_refused():
    log = []
    rc, _, reg = core(log=log)
    res = rc.approve(ctx())
    assert res["decision"] == "approved" and res["candidate_hash"] == CAND and res["approver"] == "local-supervisor"
    assert res["tamper_refused"] is True and res["replay_refused"] is True
    assert [(o, t["candidate_hash"]) for o, t in log] == [("approve", "0" * 64), ("approve", CAND), ]
    assert all(t["proposal_id"] == "prop-1" and t["expected_revision"] == 3 for _, t in log)
    assert "jws" not in repr(res)


def test_publish_needs_a_prior_approval_then_moves_staging_and_alias_read_follows():
    rc, bridge, reg = core()
    with pytest.raises(RuntimeError, match="not approved"):
        rc.publish(ctx())
    rc.approve(ctx())
    assert rc.publish(ctx()) == {"release_id": "rel-new", "alias": "staging"}
    key = [k for m, p, b, k in reg.calls if p.endswith("/publish")][0]["headers"]["Idempotency-Key"]
    assert key.startswith("pub-")
    assert rc.alias_read(ctx(), "staging") == {"release_id": "rel-new", "alias": "staging"}


def test_alias_read_of_staging_before_publish_is_refused():
    rc, _, _ = core()
    rc.approve(ctx())
    with pytest.raises(RuntimeError, match="before publish"):
        rc.alias_read(ctx(), "staging")


def test_hooks_supply_all_real_core_hooks_and_time_every_call():
    rc, _, _ = core()
    h = rc.hooks()
    assert all([h.dry_run, h.run_arms, h.approve, h.publish, h.alias_read])
    rc.run_arms(ctx())
    assert rc.timings and all(set(t) == {"call", "seconds"} and t["seconds"] >= 0 for t in rc.timings)


def test_hooks_can_leave_step_6_on_the_stand_in_with_the_blocking_dependency_named():
    rc, _, _ = core()
    h = rc.hooks(arms=False, blocked={6: "blocked(jev)"})
    assert h.run_arms is None and h.blocked == {6: "blocked(jev)"} and all([h.dry_run, h.approve, h.publish, h.alias_read])


def test_evaluate_runs_the_cores_native_evaluation_on_the_frozen_proposal_with_the_threads_suite():
    rc, _, _ = core()
    ev = rc.evaluate(ctx())
    assert ev["verdict"] == "pass" and ev["eval_run_ref"] == "er-1"
    stage, job, agent, inp, extra = rc.engine.stages[-1]
    assert inp["evaluate_enabled"] is True and inp["proposal_id"] == "prop-1" and inp["draft_plan_ref"] == rc.freeze(ctx()).plan_ref
    assert inp["evaluation_suite_id"] == "disputas-tarea-suite" and inp["evaluation_suite_version"] == "2.0.0"
    assert extra["registry_mutation_commitment"]["mode"] == "evaluate_only"


def test_gate_probe_reports_why_approval_and_publish_cannot_move_staging():
    rc, bridge, reg = core()
    reg.needs_eval = True
    rc.engine.eval_verdict = "failed_infra"
    res = rc.gate_probe(ctx())
    assert res["evaluation"] == "failed_infra"
    assert res["approve"] == [409, "illegal_transition"] and res["publish"] == [409, "illegal_transition"]
    assert res["staging_unchanged"] is True and bridge.aliases["staging"] == BASE


def test_approve_runs_the_native_evaluation_first_because_the_registry_approves_only_evaluated_proposals():
    rc, _, reg = core()
    rc.approve(ctx())
    modes = [s[4]["registry_mutation_commitment"]["mode"] for s in rc.engine.stages]
    assert modes == ["write", "evaluate_only"]  # freeze, then the evaluation, then the approve call
    assert [p for m, p, b, k in reg.calls if p.endswith("/approve")]
    assert rc.evaluate(ctx()) is rc.evaluate(ctx()) and len(rc.engine.stages) == 2  # memoised: one evaluation per thread


def test_approve_refuses_without_calling_the_registry_when_the_native_evaluation_does_not_pass():
    rc, _, reg = core()
    rc.engine.eval_verdict = "fail"
    with pytest.raises(RuntimeError, match="native evaluation.*fail"):
        rc.approve(ctx())
    assert not [p for m, p, b, k in reg.calls if p.endswith("/approve")]


def test_each_registry_call_carries_the_jws_of_its_own_operation():
    rc, _, reg = core()
    rc.approve(ctx())
    rc.publish(ctx())
    bearers = {(m, p.rsplit("/", 1)[-1]): b for m, p, b, _ in reg.calls}
    assert bearers[("POST", "approve")] == "jws.approve.sig" and bearers[("POST", "publish")] == "jws.publish.sig"
    assert bearers[("GET", "prop-1")] == "bot.jws.x"  # reads use the bot credential, never a human JWS


def test_publish_that_answers_the_base_release_did_not_publish_the_draft():
    rc, bridge, reg = core()
    rc.approve(ctx())
    reg.__class__.__call__, orig = (lambda self, m, p, b, **k: Resp(200, {"release_id": BASE})
                                    if p.endswith("/publish") else orig(self, m, p, b, **k)), reg.__class__.__call__
    try:
        with pytest.raises(RuntimeError, match="base release"):
            rc.publish(ctx())
    finally:
        reg.__class__.__call__ = orig

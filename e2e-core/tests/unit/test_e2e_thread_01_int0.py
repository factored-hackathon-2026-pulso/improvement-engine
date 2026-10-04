"""INT0: thread steps 6, 8, 9 with real-Core hooks (fake hooks here; the live evidence is tests/live/test_08)."""
from claude_standin import thread01 as T
from test_e2e_thread_01 import ER, cfg, step


def _arm_runs(done, cases=("c1", "c2")):
    return [{"arm": "x", "case_ref": c, "status": s, "closed_early": False, "cost_known": True,
             "oracle_ref": "oracle:handwritten@1", "final_state_ref": "final:x", "effect_receipts": [], "reason": None}
            for c, s in zip(cases, done)]


def test_step_06_core_arms_flip_the_step_but_the_judge_stays_the_stand_in(tmp_path):
    arms = {"base": _arm_runs(("failed", "completed")), "candidate": _arm_runs(("completed", "completed"))}
    s = step(T.run_thread(cfg(tmp_path, hooks=T.CoreHooks(run_arms=lambda ctx: arms))), 6)
    assert s["status"] == "real-narrow" and s["receipt"]["provider"] == "core-arms"
    assert s["detail"]["arms"] == "core" and s["detail"]["verdict_judge"] == "stand-in"
    assert s["actor"] == "claude-gsipy" and s["detail"]["quality_claims"] == "forbidden"


def test_step_08_core_verified_approval_hook_flips_the_step(tmp_path):
    seen = []

    def approve(ctx):
        seen.append(ctx.out["compiled"]["draft_plan"]["digest"])
        return {"approver": "local-supervisor", "decision": "approved", "candidate_hash": seen[0].split(":")[1],
                "tamper_refused": True, "replay_refused": True}

    s = step(T.run_thread(cfg(tmp_path, hooks=T.CoreHooks(approve=approve))), 8)
    assert s["status"] == "real-narrow" and s["receipt"]["provider"] == "local-human-issuer"
    assert s["detail"]["bound_to_digest"] == seen[0] and s["detail"]["verified_by"] == "core"
    assert s["detail"]["issuer"] == "local-human-issuer"


def test_step_08_hook_for_another_digest_is_red(tmp_path):
    bad = lambda ctx: {"approver": "x", "decision": "approved", "candidate_hash": "0" * 64,  # noqa: E731
                       "tamper_refused": True, "replay_refused": True}
    assert step(T.run_thread(cfg(tmp_path, hooks=T.CoreHooks(approve=bad))), 8)["status"] == "red"


def test_step_08_hook_that_did_not_prove_tamper_and_replay_refusal_is_red(tmp_path):
    weak = lambda ctx: {"approver": "x", "decision": "approved",  # noqa: E731
                        "candidate_hash": ctx.out["compiled"]["draft_plan"]["digest"].split(":")[1],
                        "tamper_refused": False, "replay_refused": True}
    assert step(T.run_thread(cfg(tmp_path, hooks=T.CoreHooks(approve=weak))), 8)["status"] == "red"


def test_step_09_alias_read_that_disagrees_with_the_publish_is_red(tmp_path):
    hooks = T.CoreHooks(publish=lambda ctx: {"release_id": "rel-new", "alias": "staging"},
                        alias_read=lambda ctx, alias: {"release_id": "rel-old", "alias": alias})
    assert step(T.run_thread(cfg(tmp_path, hooks=hooks)), 9)["status"] == "red"


def test_step_09_alias_read_runs_after_publish(tmp_path):
    order = []
    hooks = T.CoreHooks(publish=lambda ctx: order.append("publish") or {"release_id": "r", "alias": "staging"},
                        alias_read=lambda ctx, alias: order.append("read") or {"release_id": "r", "alias": alias})
    assert step(T.run_thread(cfg(tmp_path, hooks=hooks)), 9)["status"] == "real-narrow"
    assert order == ["publish", "read"]


def test_step_06_stays_stand_in_and_reports_the_blocking_dependency(tmp_path):
    s = step(T.run_thread(cfg(tmp_path, hooks=T.CoreHooks(blocked={6: "blocked(jev)"}))), 6)
    assert s["status"] == "stand-in" and s["detail"]["blocked"] == "blocked(jev)" and s["detail"]["arms"] == "stand-in"


def test_steps_8_and_9_stay_stand_in_and_report_the_blocking_dependency(tmp_path):
    t = T.run_thread(cfg(tmp_path, hooks=T.CoreHooks(blocked={8: "blocked(jev)", 9: "blocked(jev)"})))
    assert step(t, 8)["status"] == "simulated" and step(t, 8)["detail"]["blocked"] == "blocked(jev)"
    assert step(t, 9)["status"] == "stand-in" and step(t, 9)["detail"]["blocked"] == "blocked(jev)"


# ---- a failed gate must not be followed silently by approval and publish --------------------------------------------
def _same_arms(ctx):  # base and candidate complete the same cases: GSIpy reports fail: no_structural_improvement
    runs = _arm_runs(("completed", "completed", "completed"), cases=("a", "b", "c"))
    return {"base": runs, "candidate": [dict(r) for r in runs]}


def _spy_hooks(calls):
    def approve(ctx):
        calls.append("approve")
        return {"approver": "local-supervisor", "decision": "approved",
                "candidate_hash": ctx.out["compiled"]["draft_plan"]["digest"].split(":")[1],
                "tamper_refused": True, "replay_refused": True}
    return T.CoreHooks(run_arms=_same_arms, approve=approve,
                       publish=lambda ctx: calls.append("publish") or {"release_id": "r", "alias": "staging"},
                       alias_read=lambda ctx, alias: {"release_id": "r", "alias": alias})


def test_failed_gate_blocks_approval_and_publish_without_a_human_override(tmp_path):
    calls = []
    t = T.run_thread(cfg(tmp_path, hooks=_spy_hooks(calls)))
    assert t["gate_verdict"] == "fail"
    assert calls == []  # neither the approval nor the publish hook ran
    assert step(t, 8)["status"] == "blocked(gate)" and step(t, 9)["status"] == "blocked(gate)"
    assert step(t, 8)["detail"]["gate_verdict"] == "fail"
    assert step(t, 10)["status"] == "not_exercised"  # nothing was published, nothing to observe
    assert all(s["status"] != "red" for s in t["steps"])
    assert ER.check(t["report"]) == [] and t["report"]["gate"]["verdict"] == "fail" and not t["report"].get("overrides")


def test_human_override_of_a_failed_gate_is_labelled_everywhere(tmp_path):
    calls = []
    ov = {"by": "human", "actor": "local-supervisor", "reason": "exercise the Core approve/publish mechanics on a failed gate"}
    t = T.run_thread(cfg(tmp_path, hooks=_spy_hooks(calls), human_override=ov))
    assert calls == ["approve", "publish"]
    s8 = step(t, 8)
    assert s8["detail"]["override"]["of"] == "gate" and s8["detail"]["override"]["verdict"] == "fail"
    rep = t["report"]
    assert rep["overrides"] == [{"step": "approval", "of": "gate", "verdict": "fail", "by": "human", "label": "human_override",
                                 "reason": ov["reason"], "actor": "local-supervisor"}]
    assert rep["quality_claims"] == "forbidden" and rep["gate"]["verdict"] == "fail"
    assert any(d["part"] == "gate.override" for d in rep["doubles"])
    assert ER.check(rep) == []


def test_override_without_a_human_or_a_reason_does_not_unblock(tmp_path):
    for ov in ({"by": "engine", "actor": "x", "reason": "r"}, {"by": "human", "actor": "x", "reason": ""}, {}):
        calls = []
        t = T.run_thread(cfg(tmp_path, hooks=_spy_hooks(calls), human_override=ov))
        assert calls == [] and step(t, 8)["status"] == "blocked(gate)", ov


def test_override_is_ignored_when_the_gate_passes(tmp_path):
    arms = {"base": _arm_runs(("failed", "completed")), "candidate": _arm_runs(("completed", "completed"))}
    ov = {"by": "human", "actor": "x", "reason": "unneeded"}
    t = T.run_thread(cfg(tmp_path, hooks=T.CoreHooks(run_arms=lambda ctx: arms), human_override=ov))
    assert t["gate_verdict"] == "pass" and not t["report"].get("overrides") and ER.check(t["report"]) == []


def test_replay_default_gate_passes_and_report_states_it(tmp_path):
    t = T.run_thread(cfg(tmp_path))
    assert t["report"]["gate"]["verdict"] == "pass" and step(t, 9)["status"] == "stand-in"
    assert ER.check(t["report"]) == []

"""INT0: thread steps 6, 8, 9 with real-Core hooks (fake hooks here; the live evidence is tests/live/test_08)."""
from claude_standin import thread01 as T
from test_e2e_thread_01 import cfg, step


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

"""E2E-THREAD-01 ratchet (Q1r): the ten demo steps in REPLAY on the Python host, in-process doubles only.

One test per step. A step is red until it is real; every step is labelled with a status and a data class and the
final engine-run report must pass the G1 `check()` and the honesty tests (mapping mutation, author separation).
Steps that need the real Core (dry-run, evaluation, publish, alias read) stay `stand-in` here and have the hooks
documented in `claude_standin/thread01.py` (CoreHooks) for INT0 to swap in.
"""
import copy
import importlib.util
import json
import os
import sys
from pathlib import Path

import pytest

from claude_standin import thread01 as T

ROOT = Path(__file__).resolve().parents[3]
EXE = os.environ.get("ED0_RUNNER_EXE", "D:/cargo-targets/claude-ed0/debug/improvement-engine.exe")
FIXTURE_QUEUE = ROOT / "e2e-core" / "tests" / "fixtures" / "thread01_queue"


def _load_er():
    spec = importlib.util.spec_from_file_location("engine_run", ROOT / "contracts" / "engine-run" / "engine_run.py")
    m = importlib.util.module_from_spec(spec)
    sys.modules["engine_run"] = m
    spec.loader.exec_module(m)
    return m


ER = _load_er()


def cfg(tmp_path, **kw):
    return T.ThreadConfig(workdir=tmp_path, exe=EXE, queue_dir=FIXTURE_QUEUE, mode="replay", **kw)


@pytest.fixture(scope="module")
def thread(tmp_path_factory):
    return T.run_thread(cfg(tmp_path_factory.mktemp("thread")))


def step(thread, n, id_=None):
    got = [s for s in thread["steps"] if s["n"] == n and (id_ is None or s["id"] == id_)]
    assert got, f"step {n} missing"
    return got[0]


def green(s):
    assert s["status"] != "red", s.get("error")
    assert ER._status_ok(s["status"]), s["status"]
    assert s["data_class"] and s["host"] == "python"


# ---- the ten steps ---------------------------------------------------------------------------------------------
def test_step_01_data_wakes_the_engine_manual_command(thread):
    s = step(thread, 1); green(s)
    assert s["status"] == "stand-in" and s["detail"]["trigger"] == "manual_command"
    assert s["data_class"] == "generated_sample"


def test_step_02_signals_and_discards_by_the_rust_sensor(thread):
    s = step(thread, 2); green(s)
    assert s["status"] == "real-narrow" and s["detail"]["producer"] == "rust_local_sim_sensor"
    assert s["detail"]["admitted_family"] and s["detail"]["discards"]


def test_step_03_scout_and_separate_verifier(thread):
    sc, ve = step(thread, 3, "scout"), step(thread, 3, "verifier")
    green(sc); green(ve)
    assert sc["status"] == ve["status"] == "agent_roleplay"
    assert sc["actor"] != ve["actor"] and sc["model"] != ve["model"]
    assert sc["receipt"]["scanner_id"] == ve["receipt"]["scanner_id"] == "tps-1"
    assert ve["detail"]["recompute_ok"] is True
    assert sc["detail"]["calls"] <= 5 and ve["detail"]["calls"] <= 5  # M3 STEP_CAPS
    assert thread["m3"]["timeout_problems"] == [] and thread["m3"]["pin_problems"] == []


def test_step_04_opportunity_target_from_catalogue_never_from_prose(thread):
    s = step(thread, 4); green(s)
    assert s["status"] == "agent_roleplay" and s["detail"]["smap"]["verdict"] == "valid"
    assert s["detail"]["smap"]["target"] == "prompt:resumen_radicado@1"
    assert s["detail"]["do_nothing_considered"] is True and s["detail"]["calls"] <= 7
    assert ER.check_mapping_mutation(thread["mapper"], thread["categories"]) == []


def test_step_05_concrete_change_versions_and_diff(thread):
    s = step(thread, 5); green(s)
    assert s["status"] == "stand-in" and s["detail"]["compiler_label"] == "claude-standin(python)"
    assert [o["op"] for o in s["detail"]["draft_plan"]["operations"]] == ["replace", "add"]
    assert s["detail"]["diff"] == [{"target": "prompt:resumen_radicado@1", "to": "prompt:resumen_radicado@2"},
                                   {"target": "eval_suite:disputas-tarea-suite@1", "to": "eval_suite:disputas-tarea-suite@2"}]


def test_step_05_core_dry_run_hook_replaces_the_stand_in_digest(tmp_path):
    seen = []
    hooks = T.CoreHooks(dry_run=lambda ops: seen.append(ops) or "sha256:" + "c" * 64)
    s = step(T.run_thread(cfg(tmp_path, hooks=hooks)), 5)
    assert seen and s["detail"]["draft_plan"]["digest"] == "sha256:" + "c" * 64 and s["status"] == "real-narrow"


def test_step_06_base_vs_candidate_two_gates_structural(thread):
    s = step(thread, 6); green(s)
    assert s["status"] == "stand-in" and s["detail"]["quality_claims"] == "forbidden"
    assert [g["gate"] for g in s["detail"]["gates"]] == ["safety", "improvement"]
    assert s["detail"]["verdict"] == "pass" and s["detail"]["judge_actor"] == "claude-gsipy"


def test_step_06_arm_reports_hook_takes_real_core_arms(tmp_path):
    called = []

    def arms(ctx):
        called.append(ctx)
        return None  # None keeps the stand-in reports

    step(T.run_thread(cfg(tmp_path, hooks=T.CoreHooks(run_arms=arms))), 6)
    assert called


def test_step_07_revision_not_exercised_unless_the_gate_fails(thread, tmp_path):
    s = step(thread, 7); green(s)
    assert s["status"] == "not_exercised" and thread["gate_verdict"] == "pass"
    failing = {"improvement": lambda b, c: ("fail", "no_structural_improvement")}
    t2 = T.run_thread(cfg(tmp_path, gate_evaluators=failing))
    s2 = step(t2, 7); green(s2)
    assert s2["status"] == "stand-in" and s2["detail"]["rounds"] == 1 and s2["detail"]["bounded"] is True


def test_step_08_human_only_for_authority_simulated_issuer(thread):
    s = step(thread, 8); green(s)
    assert s["status"] == "simulated" and s["detail"]["bound_to_digest"] == step(thread, 5)["detail"]["draft_plan"]["digest"]
    assert s["detail"]["tampered_rejected"] is True and s["detail"]["verified_by"] == "local-stand-in-verifier"


def test_step_09_publish_to_staging_confirmed_by_alias_read(thread):
    s = step(thread, 9); green(s)
    assert s["status"] == "stand-in" and s["detail"]["alias_read"]["release_id"] == s["detail"]["published"]["release_id"]
    assert s["detail"]["alias_read"]["alias"] == "staging"


def test_step_09_core_hooks_flip_publish_and_alias_read(tmp_path):
    hooks = T.CoreHooks(publish=lambda ctx: {"release_id": "rel-core-1", "alias": "staging"},
                        alias_read=lambda ctx, alias: {"release_id": "rel-core-1", "alias": alias})
    s = step(T.run_thread(cfg(tmp_path, hooks=hooks)), 9)
    assert s["status"] == "real-narrow" and s["detail"]["alias_read"]["release_id"] == "rel-core-1"


def test_step_10_observation_only(thread):
    s = step(thread, 10); green(s)
    assert s["status"] == "simulated" and s["detail"]["observation_only"] is True
    assert "release.published" in s["detail"]["event_types"]
    assert all(p["data_class"] == "simulated" for p in s["detail"]["effect_series"])
    assert s["detail"]["memory"] == s["detail"]["successor"] == "not_exercised"
    assert s["detail"]["feeds_decision"] is False


# ---- whole thread: labels, replay, report, honesty --------------------------------------------------------------
def test_ten_steps_all_labelled(thread):
    assert sorted({s["n"] for s in thread["steps"]}) == list(range(1, 11))
    for s in thread["steps"]:
        green(s)
        assert s["target"] and s["sha"] and s["contract_revision"]


def test_replay_has_zero_digest_misses(thread):
    assert thread["replay"]["misses"] == 0 and thread["replay"]["calls"] > 0
    assert thread["mode"] == "replay" and thread["host"] == "python"


def test_final_report_passes_g1_check(thread):
    rep = thread["report"]
    assert ER.check(rep) == []
    assert rep["label"] == "DEMO-0" and rep["host"] == "python" and rep["quality_claims"] == "forbidden"
    parts = {d["part"]: d for d in rep["doubles"]}
    assert parts["model"]["status"] == "agent_roleplay" and parts["jev"]["status"].startswith("not_exercised")
    assert parts["issuer"]["status"] == "simulated" and parts["host"]["status"] == "python"
    assert parts["gate"]["status"].startswith("claude-authored") and parts["data.origin"]["status"] == "generated_sample"
    assert parts["product"]["status"] in ("simulated", "stand-in")


def test_report_carries_no_e0_content(thread):
    assert ER.scan_receipt(json.dumps(thread["report"], sort_keys=True)) == []


def test_honesty_author_separation(thread):
    rep = thread["report"]
    for same in ("world", "suite", "effect"):
        bad = copy.deepcopy(rep); bad["authors"]["judge"] = bad["authors"][same]
        assert "H5" in {v["rule"] for v in ER.check(bad)}, same
    bad = copy.deepcopy(rep); bad["authors"]["suite_sealed_at"] = bad["authors"]["candidate_created_at"]
    assert "H5" in {v["rule"] for v in ER.check(bad)}


def test_honesty_verifier_distinct_and_no_lying_real(thread):
    rep = copy.deepcopy(thread["report"])
    ver = next(s for s in rep["steps"] if s["id"] == "verifier")
    ver["actor"] = next(s for s in rep["steps"] if s["id"] == "scout")["actor"]
    assert "H7" in {v["rule"] for v in ER.check(rep)}
    rep = copy.deepcopy(thread["report"])
    next(s for s in rep["steps"] if s["id"] == "scout")["status"] = "real"
    assert "H1" in {v["rule"] for v in ER.check(rep)}


def test_honesty_mapping_mutation_catches_a_keyed_mapper(thread):
    cats = thread["categories"]
    keyed_label = sorted(cats)[0]
    keyed = lambda c: keyed_label if keyed_label in c else max(c, key=c.get)  # noqa: E731
    assert {v["rule"] for v in ER.check_mapping_mutation(keyed, cats)} == {"H3"}


def test_recorded_queue_matches_the_scripted_responder(tmp_path):
    """Drift guard: re-recording with the scripted responder reproduces the committed replay fixtures byte for byte."""
    rec = T.run_thread(T.ThreadConfig(workdir=tmp_path, exe=EXE, queue_dir=tmp_path / "q", mode="record"))
    assert rec["replay"]["misses"] == 0
    new = {p.name: p.read_text() for p in (tmp_path / "q" / "responses").glob("*.json")}
    old = {p.name: p.read_text() for p in (FIXTURE_QUEUE / "responses").glob("*.json")}
    assert new == old and new


# ---- adversarial review additions -------------------------------------------------------------------------------
def test_replay_drift_deleted_fixture_is_counted_as_a_miss(tmp_path):
    import shutil
    q = tmp_path / "q"; shutil.copytree(FIXTURE_QUEUE, q)
    # the builder answer is the last recorded stage: dropping it must show up as a digest miss, not as 0
    for p in (q / "responses").glob("*.json"):
        if json.loads(p.read_text())["responder"]["role"] == "builder_design":
            p.unlink()
    t = T.run_thread(T.ThreadConfig(workdir=tmp_path / "w", exe=EXE, queue_dir=q, mode="replay"))
    assert step(t, 4)["status"] == "red" and t["replay"]["misses"] >= 1


def test_step_08_payload_tamper_and_expiry_are_rejected(thread):
    d = step(thread, 8)["detail"]
    assert d["payload_tamper_rejected"] is True and d["expired_rejected"] is True

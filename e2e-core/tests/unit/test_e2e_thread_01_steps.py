"""E3b ratchet: `--steps rust` flips steps 2..6 to real-narrow (host=python, semantics=claude-standin);
the default run stays python. Rust-dependent tests skip unless STEPS_CLI_EXE names steps_cli."""
import importlib.util
import os
import sys
from pathlib import Path

import pytest

from claude_standin import thread01 as T

ROOT = Path(__file__).resolve().parents[3]
EXE = os.environ.get("ED0_RUNNER_EXE", "D:/cargo-targets/claude-ed0/debug/improvement-engine.exe")
STEPS_EXE = os.environ.get("STEPS_CLI_EXE")
needs_rust = pytest.mark.skipif(not (STEPS_EXE and os.path.exists(STEPS_EXE)), reason="STEPS_CLI_EXE not set")
FIXTURE_QUEUE = ROOT / "e2e-core" / "tests" / "fixtures" / "thread01_queue"


def _er():
    spec = importlib.util.spec_from_file_location("engine_run", ROOT / "contracts" / "engine-run" / "engine_run.py")
    m = importlib.util.module_from_spec(spec)
    sys.modules["engine_run"] = m
    spec.loader.exec_module(m)
    return m


ER = _er()


def cfg(tmp_path, **kw):
    return T.ThreadConfig(workdir=tmp_path, exe=EXE, queue_dir=FIXTURE_QUEUE, mode="replay", **kw)


def one(run, sid):
    return next(s for s in run["steps"] if s["id"] == sid)


def test_default_run_keeps_every_step_python(tmp_path):
    run = T.run_thread(cfg(tmp_path))
    assert run["steps_mode"] == "python"
    assert [one(run, i)["status"] for i in ("signals", "compile", "gate")] == ["real-narrow", "stand-in", "stand-in"]
    assert all(s["id"] not in ("recompute", "validation") for s in run["steps"])
    assert all("semantics" not in s for s in run["steps"])
    assert ER.check(run["report"]) == []


@pytest.fixture(scope="module")
def rust_run(tmp_path_factory):
    if not (STEPS_EXE and os.path.exists(STEPS_EXE)):
        pytest.skip("STEPS_CLI_EXE not set")
    return T.run_thread(cfg(tmp_path_factory.mktemp("rust"), steps_mode="rust", steps_exe=STEPS_EXE))


@needs_rust
def test_rust_run_flips_five_steps_with_host_and_semantics(rust_run):
    assert rust_run["steps_mode"] == "rust"
    for n, sid in ((2, "signals"), (3, "recompute"), (4, "validation"), (5, "compile"), (6, "gate")):
        s = next(r for r in rust_run["steps"] if r["n"] == n and r["id"] == sid)
        assert s["status"] == "real-narrow", s
        assert s["host"] == "python" and s["semantics"] == "claude-standin"
        assert s["receipt"]["provider"] == "claude-standin"
        assert s["steps_label"]


@needs_rust
def test_rust_run_keeps_model_steps_roleplay_and_g1_passes(rust_run):
    for sid in ("scout", "verifier", "opportunity"):
        assert one(rust_run, sid)["status"] == "agent_roleplay"
    assert ER.check(rust_run["report"]) == []
    assert rust_run["report"]["host"] == "python"
    parts = {d["part"] for d in rust_run["report"]["doubles"]}
    assert {"scout", "port.registry"} <= parts  # model stand-ins and ports are still listed
    assert {s["id"] for s in rust_run["report"]["steps"] if s["id"] in ("recompute", "validation")} == {"recompute", "validation"}
    assert one(rust_run, "verifier")["detail"]["recompute_ok"] is True


@needs_rust
def test_rust_and_python_runs_agree_on_results(rust_run, tmp_path):
    py = T.run_thread(cfg(tmp_path))
    assert one(rust_run, "compile")["detail"]["draft_plan"] == one(py, "compile")["detail"]["draft_plan"]
    r6, p6 = one(rust_run, "gate"), one(py, "gate")
    assert (r6["detail"]["verdict"], r6["detail"]["gates"]) == (p6["detail"]["verdict"], p6["detail"]["gates"])
    r2, p2 = one(rust_run, "signals"), one(py, "signals")
    for k in ("admitted_family", "winner_support", "denominator"):
        assert r2["detail"][k] == p2["detail"][k]
    assert rust_run["gate_verdict"] == py["gate_verdict"]


def test_rust_mode_without_exe_is_red_not_silent(tmp_path):
    run = T.run_thread(cfg(tmp_path, steps_mode="rust", steps_exe=None))
    assert one(run, "signals")["status"] == "red"

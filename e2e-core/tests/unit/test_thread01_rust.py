"""Q1: the ten-step thread with host=rust, validated by the frozen G1 check() and compared with the Python twin's step list.

Tests that spawn the Rust binary skip cleanly unless THREAD10_EXE names an existing `thread10` binary
(e.g. D:/cargo-targets/claude-w4f-q1/debug/thread10.exe; `synth_runner` must sit next to it).
Nothing here touches E0, a container or the network; thread01.py (frozen, C-2) is read, never edited.
"""
import copy
import os
from pathlib import Path

import pytest

from claude_standin import thread01_rust as T

ROOT = Path(__file__).resolve().parents[3]
EXE = os.environ.get("THREAD10_EXE")
needs_rust = pytest.mark.skipif(not (EXE and os.path.exists(EXE)), reason="THREAD10_EXE not set")


def er():
    return T.engine_run()


def status(rep, n, sid):
    return next(s["status"] for s in rep["steps"] if s["n"] == n and s["id"] == sid)


@needs_rust
def test_ten_steps_host_rust_pass_g1_check(tmp_path):
    rep = T.run_rust(EXE, tmp_path, human_override=True)
    assert rep["host"] == "rust" and {s["n"] for s in rep["steps"]} == set(range(1, 11))
    assert er().check(rep) == [], er().check(rep)
    assert status(rep, 1, "trigger") == "stand-in", "ratchet step 1 stays stand-in"
    assert all(s["status"] != "real" for s in rep["steps"])
    parts = {d["part"]: d for d in rep["doubles"]}
    assert parts["gate.override"]["status"].startswith("human_override(simulated human")
    assert parts["host"]["status"] == "rust" and "offline-double" in parts["port.core"]["status"]
    assert rep["successor"]["runs"] == 1 and rep["memory_note"]["durable"] is False


@needs_rust
def test_failed_gate_without_override_blocks_8_and_9_and_still_passes_g1(tmp_path):
    rep = T.run_rust(EXE, tmp_path, human_override=False, expect_stop=True)
    assert [status(rep, n, i) for n, i in ((8, "approval"), (9, "publish"), (10, "observation"))] == ["blocked(gate)", "blocked(gate)", "not_exercised"]
    assert er().check(rep) == []


@needs_rust
def test_denied_kind_blocks_compile_and_passes_g1(tmp_path):
    rep = T.run_rust(EXE, tmp_path, human_override=True, denied_kind=True, expect_stop=True)
    assert status(rep, 5, "compile") == "blocked(kind_not_supported)" and status(rep, 9, "publish") == "not_exercised"
    assert er().check(rep) == []


@needs_rust
def test_g1_rejects_a_rust_report_that_lies(tmp_path):
    rep = T.run_rust(EXE, tmp_path, human_override=True)
    lie = copy.deepcopy(rep)
    next(s for s in lie["steps"] if s["id"] == "publish")["status"] = "real"
    assert any(v["rule"] == "H1" for v in er().check(lie)), "real with a claude-standin receipt"
    no_ov = copy.deepcopy(rep)
    del no_ov["overrides"]
    assert any(v["rule"] == "G1" for v in er().check(no_ov)), "publish after a failed gate needs the labelled override"


@needs_rust
def test_rust_step_list_covers_the_python_twin_step_list(tmp_path):
    rep = T.run_rust(EXE, tmp_path, human_override=True)
    twin = T.python_twin_steps()
    got = {(s["n"], s["id"]) for s in rep["steps"]}
    assert twin and set(twin) <= got, set(twin) - got


def test_python_twin_step_list_is_the_ten_steps():
    assert [n for n, _ in T.python_twin_steps()] == list(range(1, 11))

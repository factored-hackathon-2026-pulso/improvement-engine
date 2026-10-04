"""Q1: Python regression twin of the Rust-shell thread (host=rust).

The ten steps run in the Rust binary `thread10` (seams/crates/thread10): engine executor, steps stand-ins, a labelled
Core double, thin memory note, control-api successor correlation. This module only spawns it, adds the engine-generated
`doubles[]` (G1 `generate_doubles`, never hand-written) and hands the report to the frozen G1 `check()`. It never
relabels a step: status, data class and host are exactly what Rust reported. `thread01.py` (frozen, C-2) is the Python
host and is read here, never edited.
"""
from __future__ import annotations

import importlib.util
import json
import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]


def engine_run():
    """The G1 module (contracts/engine-run/engine_run.py), loaded by path like thread01 does."""
    if "engine_run" in sys.modules:
        return sys.modules["engine_run"]
    spec = importlib.util.spec_from_file_location("engine_run", ROOT / "contracts" / "engine-run" / "engine_run.py")
    m = importlib.util.module_from_spec(spec)
    sys.modules["engine_run"] = m
    spec.loader.exec_module(m)
    return m


def python_twin_steps() -> list[tuple[int, str]]:
    """(n, id) of the Python host's ten steps (thread01.STEPS), the regression twin this host must cover."""
    from claude_standin import thread01
    return [(n, sid) for n, sid, _ in thread01.STEPS]


def _sha() -> str:
    try:
        s = subprocess.run(["git", "rev-parse", "HEAD"], cwd=ROOT, capture_output=True, text=True).stdout.strip()
    except OSError:
        s = ""
    return s if len(s) == 40 else "0" * 40


def run_rust(exe, workdir, *, human_override=False, denied_kind=False, claimed_rate=None, expect_stop=False) -> dict:
    """Run `thread10` once and return its report with `doubles[]` added. Exit 1 (job stopped on a block) is accepted only
    with `expect_stop`; any other failure raises. The runner exe is the one next to `exe` (the fixed-output synth_runner)."""
    exe = Path(exe)
    cmd = [str(exe), str(workdir), "--sha", _sha()]
    if human_override:
        cmd.append("--override")
    if denied_kind:
        cmd.append("--denied-kind")
    if claimed_rate is not None:
        cmd += ["--claimed-rate", str(claimed_rate)]
    env = {k: v for k, v in os.environ.items() if k != "STEPS_RUNNER_EXE"}
    done = subprocess.run(cmd, capture_output=True, env=env, timeout=300)
    if done.returncode not in ((0, 1) if expect_stop else (0,)):
        raise RuntimeError(f"thread10 exited {done.returncode}: {done.stderr.decode('utf-8', 'replace').strip()[-300:]}")
    rep = json.loads(done.stdout.decode("utf-8"))
    st = lambda n, i: next(s["status"] for s in rep["steps"] if s["n"] == n and s["id"] == i)  # noqa: E731
    observed = {"model": "not_exercised(no model in the offline Rust thread)",
                "jev": "not_exercised(blocked: agent-core PR 28 not on main)",
                "issuer": st(8, "approval"), "product": "simulated" if st(9, "publish") == "stand-in" else st(9, "publish"),
                "host": rep["host"], "core": "offline-double", "gate": "claude-authored(structural, quality_claims forbidden)",
                "data_origin": "generated_sample"}
    rep["doubles"] = engine_run().generate_doubles(rep, observed)
    return rep

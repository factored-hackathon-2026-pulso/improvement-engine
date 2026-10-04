"""GT05 collector: runs the ten-step thread in REPLAY on the recorded synthetic fixtures twice, once with `--steps rust`
(steps 2-6 through the Rust step stand-ins) and once in the default Python mode, and writes the artifacts the GT05 gate reads:
rust-report.json, default-report.json, rust-run.json (gt05-run/v1, bound to the Rust report bytes and the steps_cli binary).
Optionally (--ratchet) runs the E3b ratchet tests and writes ratchet.json (ratchet-receipt/v1). Nothing is written by hand.

Needs STEPS_CLI_EXE (steps_cli binary) and ED0_RUNNER_EXE / STEPS_RUNNER_EXE (the Rust sensor), plus the e2e-core
dependencies; a missing dependency shows up as a red step in the report, never as a silent pass.
The collector re-executes itself under `uv run --offline --python 3.12 --with <DEPS>` (PYTHONPATH src;tests;agent-core, cwd
e2e-core), so any Python can start it. Runner binary: STEPS_RUNNER_EXE (default D:/cargo-targets/claude-ed0/debug/improvement-engine.exe).
Usage: python gt05_collect.py --out-dir DIR [--steps-exe EXE] [--ratchet]
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
FIXTURES = ROOT / "e2e-core" / "tests" / "fixtures" / "thread01_queue"
RATCHET_TEST = "tests/unit/test_e2e_thread_01_steps.py"  # relative to e2e-core (the legs run there)
DEPS = ("cryptography", "pyyaml", "httpx", "fastapi", "jsonschema", "pydantic", "anyio", "rfc8785", "psycopg[binary]",
        "boto3", "sqlalchemy", "moto", "pyarrow", "pytest")
AGENT_CORE = "D:/.codex/factored/references/agent-core-c814c2b"
PY_PATH = f"src;tests;{AGENT_CORE}"  # relative to e2e-core
REEXEC_ENV = "GT05_COLLECT_UNDER_UV"


def uv_command(tail: list) -> list:
    """`uv run` with the full e2e-core dependency set (offline: uv's cache only, no download)."""
    cmd = ["uv", "run", "--offline", "--python", "3.12"]
    for d in DEPS:
        cmd += ["--with", d]
    return cmd + list(tail)


def run_record(report_path: Path, steps_exe: Path, replay: dict, exit_code: int, command: str) -> dict:
    calls, misses = replay.get("calls"), replay.get("misses")
    return {"schema": "gt05-run/v1", "mode": "rust", "steps_exe": str(steps_exe),
            "steps_exe_sha256": hashlib.sha256(Path(steps_exe).read_bytes()).hexdigest(),
            "report_sha256": "sha256:" + hashlib.sha256(Path(report_path).read_bytes()).hexdigest(),
            "command": command, "exit_code": exit_code, "calls": calls if isinstance(calls, int) else 0,
            "misses": misses if isinstance(misses, int) else 1}  # absent counters never read as zero misses


def parse_pytest_summary(text: str) -> tuple:
    def n(word):
        m = re.search(rf"(\d+) {word}", text)
        return int(m.group(1)) if m else 0
    return n("passed"), n("failed"), n("skipped")


def ratchet_receipt(command: str, exit_code: int, output: str) -> dict:
    passed, failed, skipped = parse_pytest_summary(output)
    return {"schema": "ratchet-receipt/v1", "command": command, "exit_code": exit_code, "passed": passed, "failed": failed,
            "skipped": skipped}


def _write(path: Path, doc) -> None:
    path.write_text(json.dumps(doc, indent=1, sort_keys=True) + "\n", encoding="utf-8", newline="\n")


def main(argv=None) -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--out-dir", required=True)
    ap.add_argument("--steps-exe", default=os.environ.get("STEPS_CLI_EXE"))
    ap.add_argument("--ratchet", action="store_true", help="also run the E3b ratchet tests (pytest via uv, offline)")
    a = ap.parse_args(argv)
    if os.environ.get(REEXEC_ENV) != "1" and argv is None:
        # run the Python legs under uv with the dependency set (the system Python lacks cryptography etc.)
        env = {**os.environ, REEXEC_ENV: "1", "PYTHONPATH": PY_PATH}
        args = [str(Path(__file__).resolve()), "--out-dir", str(Path(a.out_dir).resolve())] + (
            ["--steps-exe", str(Path(a.steps_exe).resolve())] if a.steps_exe else []) + (["--ratchet"] if a.ratchet else [])
        return subprocess.run(uv_command(["python"] + args), cwd=str(ROOT / "e2e-core"), env=env).returncode
    if not a.steps_exe or not Path(a.steps_exe).is_file():
        print("steps_cli binary not found (STEPS_CLI_EXE or --steps-exe)", file=sys.stderr)
        return 2
    sys.path.insert(0, str(ROOT / "e2e-core" / "src"))
    from claude_standin import thread01 as T  # heavy imports stay out of the helpers above
    exe = os.environ.get("ED0_RUNNER_EXE") or os.environ.get("STEPS_RUNNER_EXE") or "D:/cargo-targets/claude-ed0/debug/improvement-engine.exe"
    out = Path(a.out_dir)
    out.mkdir(parents=True, exist_ok=True)
    res = {}
    for mode in ("rust", "python"):
        with tempfile.TemporaryDirectory() as tmp:
            res[mode] = T.run_thread(T.ThreadConfig(workdir=Path(tmp), exe=exe, queue_dir=FIXTURES, mode="replay",
                                                    steps_mode=mode, steps_exe=a.steps_exe if mode == "rust" else None))
    _write(out / "rust-report.json", res["rust"]["report"])
    _write(out / "default-report.json", res["python"]["report"])
    red = [s["id"] for s in res["rust"]["steps"] if s["status"] == "red"]
    rec = run_record(out / "rust-report.json", Path(a.steps_exe), res["rust"].get("replay") or {}, 1 if red else 0,
                     "python scripts/gov/gt05_collect.py --out-dir <dir> (thread01 replay, steps_mode=rust)")
    _write(out / "rust-run.json", rec)
    print(f"wrote rust-report.json, default-report.json, rust-run.json: {rec['calls']} calls, {rec['misses']} misses, red steps: {red or 'none'}")
    if a.ratchet:
        cmd = uv_command(["python", "-m", "pytest", RATCHET_TEST, "-q", "-p", "no:cacheprovider"])
        env = {**os.environ, "STEPS_CLI_EXE": str(a.steps_exe), "PYTHONPATH": PY_PATH}
        r = subprocess.run(cmd, cwd=str(ROOT / "e2e-core"), capture_output=True, text=True, env=env)
        rr = ratchet_receipt(" ".join(cmd), r.returncode, r.stdout + r.stderr)
        _write(out / "ratchet.json", rr)
        print(f"ratchet tests: exit {r.returncode}, {rr['passed']} passed, {rr['failed']} failed")
    return 0


if __name__ == "__main__":
    sys.exit(main())

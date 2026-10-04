"""GT0 collector: runs the ten-step thread in REPLAY on the recorded fixtures and writes the artifacts the gate reads:
report.json (the engine-run report, ids and labels only) and replay.json (gt0-replay/v1, bound to the report bytes).

Needs the e2e-core dependencies (pyyaml, cryptography, ...) and the Rust sensor exe (ED0_RUNNER_EXE); without the exe
step 2 is blocked and the report says so. It never writes numbers by hand: everything comes from the run.

Usage: python gt0_collect.py --out-dir DIR [--override-reason TEXT]
  --override-reason runs steps 8-9 under the labelled SIMULATED human override (the stand-in gate fails honestly).
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
FIXTURES = ROOT / "e2e-core" / "tests" / "fixtures" / "thread01_queue"


def fixtures_digest(root: Path) -> str:
    files = {p.relative_to(root).as_posix(): hashlib.sha256(p.read_bytes().replace(b"\r\n", b"\n")).hexdigest()
             for p in sorted(root.rglob("*")) if p.is_file()}
    return "sha256:" + hashlib.sha256(json.dumps(files, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def replay_record(report_path: Path, fix_digest: str, replay: dict, mutation_violations: list) -> dict:
    rep_sha = "sha256:" + hashlib.sha256(report_path.read_bytes()).hexdigest()
    run_id = "replay-" + hashlib.sha256((fix_digest + rep_sha).encode()).hexdigest()[:16]
    misses = replay.get("misses")
    calls = replay.get("calls")
    return {"schema": "gt0-replay/v1", "run_id": run_id, "mode": "replay", "calls": calls if isinstance(calls, int) else 0,
            "misses": misses if isinstance(misses, int) else 1,   # absent counters never read as zero misses
            "fixtures_digest": fix_digest, "report_sha256": rep_sha,
            "mapping_mutation_violations": mutation_violations}


def main(argv=None) -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--out-dir", required=True)
    ap.add_argument("--override-reason")
    a = ap.parse_args(argv)
    sys.path.insert(0, str(ROOT / "e2e-core" / "src"))
    from claude_standin import thread01 as T  # heavy imports stay out of the helpers above
    sys.path.insert(0, str(HERE))
    import gt0_gate
    er = gt0_gate.engine_run()
    exe = os.environ.get("ED0_RUNNER_EXE", "D:/cargo-targets/claude-ed0/debug/improvement-engine.exe")
    override = ({"by": "human", "actor": "simulated-human-demo-0", "reason": a.override_reason, "simulated": True}
                if a.override_reason else None)
    with tempfile.TemporaryDirectory() as tmp:
        res = T.run_thread(T.ThreadConfig(workdir=Path(tmp), exe=exe, queue_dir=FIXTURES, mode="replay",
                                          human_override=override))
    out = Path(a.out_dir)
    out.mkdir(parents=True, exist_ok=True)
    report_path = out / "report.json"
    report_path.write_text(json.dumps(res["report"], indent=1, sort_keys=True) + "\n", encoding="utf-8", newline="\n")
    mapper, cats = res.get("mapper"), res.get("categories")
    violations = er.check_mapping_mutation(mapper, cats) if mapper and cats else [
        {"rule": "H3", "where": "mapping", "msg": "no mapper evidence in the run"}]
    rec = replay_record(report_path, fixtures_digest(FIXTURES), res.get("replay") or {}, violations)
    (out / "replay.json").write_text(json.dumps(rec, indent=1) + "\n", encoding="utf-8", newline="\n")
    print(f"wrote {report_path} and replay.json: run {rec['run_id']}, {rec['calls']} calls, {rec['misses']} misses")
    return 0


if __name__ == "__main__":
    sys.exit(main())

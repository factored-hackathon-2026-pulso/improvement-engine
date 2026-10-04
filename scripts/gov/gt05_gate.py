"""GT05 gate evidence script (DEMO-1a, tier T0.5, strangler stage S2). Reads a manifest of artifacts, recomputes every check
from them and exits 1 if any item fails or is MISSING. Nothing is taken from a hand-written summary.

Manifest (gt05-manifest/v1), paths relative to the manifest file:
  {"schema": "gt05-manifest/v1", "reviews_dir": "...", "gt0_manifest": "path to the GT0 manifest",
   "artifacts": {"rust_report": ..., "default_report": ..., "run": ..., "ratchet_receipt": ..., "original_run": ...}}

Items:
  rust_flips          steps 2-6 (signals, recompute, validation, compile, gate) are real-narrow, host=python,
                      semantics=claude-standin and carry a steps label and the steps_cli sha256
  exe_sha256          every recorded steps_exe_sha256 equals the sha256 of the steps_cli binary on disk
  fallback_disclosure a step of 2-6 that is not Rust-served must say so (steps_fallback); a Rust-served one must not
  g1_check            engine_run.check() is clean on the Rust-steps report
  no_red_steps        no step of the Rust-steps run is red
  run_record          the run record is a rust-mode run, exit 0, no replay misses, bound to the report bytes
  gt0_dependency      the GT0 gate passes on every item but capacity
  ratchet_default     the default (Python) run is unchanged: same steps, no Rust labels, and the steps the flips do not
                      touch agree between the two runs
  crv1                independent-review closure of the S1/S2 work (crv0_closure machinery)
  rg1                 RG-1, checkable part: every step the report marks real-narrow is served by a Rust step
  rg1_ratchet_tests   RG-1: a passing receipt of the ratchet tests on the Rust mode (MISSING when none is recorded)
  rg1_original        RG-1: the ED0b original-source run goes through the Rust steps (MISSING when none is recorded)

RG-1 (plan, section on the strangler stages): "every step the ratchet marks real-narrow is served by a Rust step".
It is not fully checkable from one synthetic run: the ratchet tests must be green on the Rust mode, and the original-source
(ED0b) run, where the Rust sensor is not mapped, must not leave a real-narrow step on Python.
Usage: python gt05_gate.py --manifest M.json [--repo PATH] [--steps-exe EXE] [--skip-gt0] [--out RESULT.json]
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
sys.path.insert(0, str(HERE))
import crv0_closure  # noqa: E402
import gt0_gate  # noqa: E402

FLIPS = (("signals", 2), ("recompute", 3), ("validation", 4), ("compile", 5), ("gate", 6))
FLIP_IDS = frozenset(i for i, _ in FLIPS)
RUST_KEYS = ("semantics", "steps_label", "steps_exe_sha256", "steps_fallback")
# S1/S2 work reviewed under CRV1 (the row title: S1 retro-review and S2; the Rust steps, their host glue and the W1 lanes).
REQUIRED_CRV1 = ["STP1", "CMP", "GSI", "E3b", "K0", "ED0b", "M5a", "PX0", "G2", "TA0"]
DEFAULT_LABEL = "DEMO-0"


def _res(item, ok, detail):
    return {"item": item, "ok": bool(ok), "detail": detail}


def _missing(item, what):
    return _res(item, False, f"MISSING: {what}")


def _step(report, sid):
    return next((s for s in report.get("steps", []) if s.get("id") == sid), None)


def _hex(x):
    return x[7:] if isinstance(x, str) and x.startswith("sha256:") else x


def _file_sha(path: Path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def check_flips(report):
    p = []
    for sid, n in FLIPS:
        s = _step(report, sid)
        if s is None:
            p.append(f"{sid}: step absent")
            continue
        bad = []
        if s.get("n") != n:
            bad.append(f"n={s.get('n')!r}")
        if s.get("status") != "real-narrow":
            bad.append(f"status {s.get('status')!r} (still stand-in or other)")
        if s.get("host") != "python":
            bad.append(f"host {s.get('host')!r}")
        if s.get("semantics") != "claude-standin":
            bad.append(f"semantics {s.get('semantics')!r}")
        if not s.get("steps_label"):
            bad.append("no steps_label")
        if not s.get("steps_exe_sha256"):
            bad.append("no steps_exe_sha256")
        if bad:
            p.append(f"{sid}: " + ", ".join(bad))
    return _res("rust_flips", not p, "; ".join(p) or "steps 2-6 real-narrow, host=python, semantics=claude-standin, Rust-labelled")


def check_exe(report, run, steps_exe):
    exe = steps_exe or os.environ.get("STEPS_CLI_EXE") or (run or {}).get("steps_exe")
    if not exe or not Path(exe).is_file():
        return _res("exe_sha256", False, f"steps_cli binary not found on disk ({exe!r}); the recorded sha256 cannot be verified")
    want = _file_sha(Path(exe))
    p = []
    seen = 0
    for sid, _ in FLIPS:
        s = _step(report, sid) or {}
        got = _hex(s.get("steps_exe_sha256"))
        if got is None:
            continue
        seen += 1
        if got != want:
            p.append(f"{sid}: recorded {str(got)[:12]} != binary {want[:12]}")
    if (run or {}).get("steps_exe_sha256") is not None and _hex(run["steps_exe_sha256"]) != want:
        p.append("run record sha256 differs from the binary on disk")
    if not seen:
        p.append("no step records a steps_exe_sha256")
    return _res("exe_sha256", not p, "; ".join(p) or f"{seen} step(s) match the binary on disk ({want[:12]})")


def check_fallback(report):
    p = []
    for sid, _ in FLIPS:
        s = _step(report, sid)
        if s is None:
            p.append(f"{sid}: step absent")
            continue
        rust, fb = bool(s.get("steps_exe_sha256")), s.get("steps_fallback")
        if rust and fb:
            p.append(f"{sid}: claims Rust (sha256 recorded) and a Python fallback at once")
        elif not rust and not (isinstance(fb, str) and fb.startswith("python(")):
            p.append(f"{sid}: not served by Rust and no steps_fallback says so (silent Python)")
    return _res("fallback_disclosure", not p, "; ".join(p) or "no silent Python fallback among steps 2-6")


def check_g1(report):
    v = gt0_gate.engine_run().check(report)
    return _res("g1_check", not v, "; ".join(f"{x['rule']} {x['where']}: {x['msg']}" for x in v) or "engine_run.check() clean (G1, S1, tests 1-8)")


def check_red(report):
    red = [f"{s.get('id')}: {s.get('error', '')}"[:160] for s in report.get("steps", []) if s.get("status") == "red"]
    return _res("no_red_steps", not red, "; ".join(red) or "no red step")


def check_run(run, report_path: Path):
    p = []
    if not isinstance(run, dict) or run.get("schema") != "gt05-run/v1":
        return _res("run_record", False, "run record must be gt05-run/v1")
    if run.get("mode") != "rust":
        p.append(f"mode {run.get('mode')!r}, need rust")
    if run.get("exit_code") != 0 or isinstance(run.get("exit_code"), bool):
        p.append(f"exit_code {run.get('exit_code')!r}")
    if run.get("misses") != 0 or isinstance(run.get("misses"), bool):
        p.append(f"replay misses {run.get('misses')!r}, need 0")
    if not isinstance(run.get("calls"), int) or run["calls"] <= 0:
        p.append("replay served no calls")
    if not report_path.is_file() or _hex(run.get("report_sha256")) != _file_sha(report_path):
        p.append("run record is not bound to the report bytes (report_sha256)")
    return _res("run_record", not p, "; ".join(p) or f"rust-mode run, exit 0, {run['calls']} calls, 0 misses, bound to the report")


def check_gt0(m, base, repo, evaluator, skip):
    rel = m.get("gt0_manifest")
    if skip:
        return _res("gt0_dependency", False, "SKIPPED: the GT0 dependency was not evaluated")
    if not isinstance(rel, str):
        return _res("gt0_dependency", False, "no gt0_manifest in the manifest")
    res = evaluator(base / rel, repo)
    bad = [f"{r['item']}: {r['detail']}" for r in res if not r["ok"] and r["item"] != "capacity"]
    if not res:
        return _res("gt0_dependency", False, "the GT0 gate returned no items")
    return _res("gt0_dependency", not bad, " | ".join(bad)[:900] or f"GT0 gate: {sum(r['ok'] for r in res)}/{len(res)} items pass (capacity excluded by design)")


def check_ratchet_default(default, rust):
    p = []
    if default.get("label") != DEFAULT_LABEL:
        p.append(f"default run label {default.get('label')!r}, need {DEFAULT_LABEL}")
    for s in default.get("steps", []):
        extra = [k for k in RUST_KEYS if k in s]
        if extra:
            p.append(f"default run step {s.get('id')} carries Rust keys {extra}")
        if s.get("id") in ("recompute", "validation"):
            p.append(f"default run has the Rust-only step {s['id']}")
    want = {"signals": "real-narrow", "compile": "stand-in", "gate": "stand-in"}
    for sid, st in want.items():
        s = _step(default, sid)
        if s is None or s.get("status") != st:
            p.append(f"default {sid}: status {None if s is None else s.get('status')!r}, expected {st}")
    for s in default.get("steps", []):
        if s.get("id") in FLIP_IDS:
            continue
        r = _step(rust, s.get("id"))
        if r is None or (r.get("status"), r.get("data_class")) != (s.get("status"), s.get("data_class")):
            p.append(f"step {s.get('id')} differs between the default and the Rust-steps run")
    return _res("ratchet_default", not p, "; ".join(p) or "default run unchanged (python steps, no Rust keys); untouched steps agree with the Rust run")


def check_crv1(reviews: Path):
    code, lines = crv0_closure.run(reviews, REQUIRED_CRV1)
    bad = [x for x in lines if not x.startswith("closed")]
    return _res("crv1", code == 0, "; ".join(bad) or f"{len(lines)} review logs closed; required {', '.join(REQUIRED_CRV1)} covered")


def check_rg1(report):
    bad = [str(s.get("id")) for s in report.get("steps", [])
           if s.get("status") == "real-narrow" and not (s.get("steps_exe_sha256") and s.get("semantics") == "claude-standin")]
    n = sum(1 for s in report.get("steps", []) if s.get("status") == "real-narrow")
    return _res("rg1", not bad and n > 0, f"real-narrow step(s) not served by a Rust step: {', '.join(bad)}" if bad
                else "no real-narrow step in the run" if not n else f"all {n} real-narrow step(s) are served by a Rust step")


def check_ratchet_tests(path):
    if path is None:
        return _missing("rg1_ratchet_tests", "no ratchet-test receipt in the manifest")
    doc, prob = gt0_gate._json(path)
    if prob:
        return _missing("rg1_ratchet_tests", prob)
    ok = (doc.get("schema") == "ratchet-receipt/v1" and doc.get("exit_code") == 0 and not isinstance(doc.get("exit_code"), bool)
          and isinstance(doc.get("passed"), int) and doc["passed"] > 0 and doc.get("failed") == 0
          and isinstance(doc.get("command"), str) and doc["command"].strip())
    return _res("rg1_ratchet_tests", ok, f"{doc.get('passed')} ratchet test(s) passed" if ok else "receipt is not a passing ratchet-receipt/v1 with a command")


def check_original(path, base):
    if path is None:
        return _missing("rg1_original", "no Rust-steps run of the ED0b original source (the Rust sensor is not mapped to the original CSV)")
    doc, prob = gt0_gate._json(path)
    if prob:
        return _missing("rg1_original", prob)
    rep, p2 = gt0_gate._json(base / str(doc.get("report", "")))
    p = []
    if doc.get("schema") != "gt05-original-run/v1" or doc.get("steps_mode") != "rust":
        p.append("record must be gt05-original-run/v1 with steps_mode rust")
    if p2:
        p.append(f"report {p2}")
    else:
        steps = rep.get("steps", [])
        if not any(str(s.get("data_class", "")).startswith("original") for s in steps):
            p.append("the report has no step of an original data class")
        p += [f"{s.get('id')}: real-narrow but not served by Rust" for s in steps
              if s.get("status") == "real-narrow" and not s.get("steps_exe_sha256")]
    return _res("rg1_original", not p, "; ".join(p) or "original-source run goes through the Rust steps")


def evaluate(manifest: Path, repo: Path = ROOT, steps_exe=None, gt0_evaluator=None, skip_gt0=False) -> list:
    manifest = Path(manifest)
    m, prob = gt0_gate._json(manifest)
    if prob:
        return [_res("manifest", False, prob)]
    base = manifest.parent
    art = {k: base / v for k, v in (m.get("artifacts") or {}).items() if isinstance(v, str)}
    evaluator = gt0_evaluator or gt0_gate.evaluate
    rust, p1 = gt0_gate._json(art.get("rust_report") or base / "<no rust_report in manifest>")
    default, p2 = gt0_gate._json(art.get("default_report") or base / "<no default_report in manifest>")
    run, p3 = gt0_gate._json(art.get("run") or base / "<no run in manifest>")
    out = []
    if p1:
        out += [_res(k, False, p1) for k in ("rust_flips", "exe_sha256", "fallback_disclosure", "g1_check", "no_red_steps", "rg1")]
    else:
        out += [check_flips(rust), check_exe(rust, run, steps_exe), check_fallback(rust), check_g1(rust), check_red(rust)]
    out.append(_res("run_record", False, p3 or p1) if (p1 or p3) else check_run(run, art["rust_report"]))
    out.append(check_gt0(m, base, repo, evaluator, skip_gt0))
    out.append(_res("ratchet_default", False, p2 or p1) if (p1 or p2) else check_ratchet_default(default, rust))
    out.append(check_crv1(base / m.get("reviews_dir", "docs/reviews/claude")))
    if not p1:
        out.append(check_rg1(rust))
    out += [check_ratchet_tests(art.get("ratchet_receipt")), check_original(art.get("original_run"), base)]
    return out


def main(argv=None) -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--manifest", required=True)
    ap.add_argument("--repo", default=str(ROOT))
    ap.add_argument("--steps-exe")
    ap.add_argument("--skip-gt0", action="store_true", help="do not evaluate the GT0 dependency (it then fails)")
    ap.add_argument("--out")
    a = ap.parse_args(argv)
    res = evaluate(Path(a.manifest), Path(a.repo), a.steps_exe, skip_gt0=a.skip_gt0)
    for r in res:
        print(f"{'PASS' if r['ok'] else 'FAIL'} {r['item']}: {r['detail']}")
    ok = all(r["ok"] for r in res)
    print("gt05-gate:", "pass" if ok else "fail")
    if a.out:
        Path(a.out).write_text(json.dumps({"schema": "gt05-gate-result/v1", "verdict": "pass" if ok else "fail", "items": res},
                                          indent=1) + "\n", encoding="utf-8", newline="\n")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())

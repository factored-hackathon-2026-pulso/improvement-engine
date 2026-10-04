"""GT0 gate evidence script (DEMO-0). Reads a manifest of receipts, recomputes every check from the artifacts and exits 1
if any item fails or any receipt is missing. Nothing is taken from a hand-written summary.

Manifest (gt0-manifest/v1), paths relative to the manifest file:
  {"schema": "gt0-manifest/v1", "reviews_dir": "docs/reviews/claude", "required_wps": [...optional...],
   "require_g0p_ancestry": false,
   "artifacts": {"g0p": ..., "trn0": ..., "report": ..., "replay": ..., "live_log": ..., "capacity": ...}}

Items: g0p, trn0, crv0, honesty (the 8 tests against the real report), scanner_ids, doubles, freeze (C-2/C-12 digests),
replay (run id, 0 misses, bound to the report bytes), live_window, capacity.
Usage: python gt0_gate.py --manifest M.json [--repo PATH] [--out RESULT.json]
"""
from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import re
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
sys.path.insert(0, str(HERE))
import crv0_closure  # noqa: E402

PLAN_DOUBLES = ("model", "jev", "issuer", "product", "host", "gate", "data.origin")
HONESTY_TESTS = {"test_1": "H1", "test_2": "H2", "test_3": "H3", "test_4": "H4", "test_5": "H5", "test_6": "H6",
                 "test_7": "H7", "test_8": "H8"}
LEGS = ("ci", "pytest", "ratchet")
MIN_SESSIONS = 12


def _load(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    mod = importlib.util.module_from_spec(spec)
    sys.modules[name] = mod
    spec.loader.exec_module(mod)
    return mod


def engine_run():
    return sys.modules.get("gt0_engine_run") or _load("gt0_engine_run", ROOT / "contracts" / "engine-run" / "engine_run.py")


def freeze_mod():
    return sys.modules.get("gt0_freeze") or _load("gt0_freeze", ROOT / "contracts" / "engine-run" / "freeze.py")


def _res(item, ok, detail):
    return {"item": item, "ok": bool(ok), "detail": detail}


def _json(path: Path):
    """(doc, problem). A missing or unreadable file is a problem, never a pass."""
    if not path.is_file():
        return None, f"missing: {path.name}"
    try:
        return json.loads(path.read_text(encoding="utf-8")), None
    except ValueError:
        return None, f"not JSON: {path.name}"


def sha256_of(path: Path) -> str:
    return "sha256:" + hashlib.sha256(path.read_bytes()).hexdigest()


def _git_lines(repo, *args):
    r = subprocess.run(["git", "-C", str(repo), *args], capture_output=True, text=True)
    return None if r.returncode != 0 else [x for x in r.stdout.splitlines() if x]


RUST_RE = re.compile(r"\.rs$|(^|/)Cargo\.(toml|lock)$|(^|/)rust-toolchain")


def check_receipt_doc(doc: dict, name: str) -> list:
    p = []
    if doc.get("schema") != "pre-pr-gate/v1":
        p.append(f"{name}: schema must be pre-pr-gate/v1")
    if doc.get("verdict") != "pass":
        p.append(f"{name}: verdict {doc.get('verdict')!r}")
    for leg in LEGS:
        lg = (doc.get("legs") or {}).get(leg)
        lg = lg if isinstance(lg, dict) else {}
        if lg.get("status") != "pass":
            p.append(f"{name}: leg {leg} is not pass")
        elif not (isinstance(lg.get("command"), str) and lg["command"].strip()) or lg.get("exit_code") != 0                 or isinstance(lg.get("exit_code"), bool):
            p.append(f"{name}: leg {leg} claims pass without a command and exit_code 0")
    return p


def check_g0p(path, repo, ancestry=True):
    doc, prob = _json(path)
    if prob:
        return _res("g0p", False, prob)
    p = check_receipt_doc(doc, path.name)
    declared = doc.get("rust_files_changed")
    if not isinstance(declared, list):
        p.append("receipt does not state rust_files_changed")
    head = doc.get("head_sha")
    if not (isinstance(head, str) and re.fullmatch(r"[0-9a-f]{40}", head)):
        p.append("receipt carries no valid head_sha")
    elif ancestry:
        r = subprocess.run(["git", "-C", str(repo), "merge-base", "--is-ancestor", head, "HEAD"], capture_output=True)
        if r.returncode != 0:
            p.append("receipt head_sha is not an ancestor of HEAD")
        else:
            later = _git_lines(repo, "diff", "--name-only", head, "HEAD")
            if later is None or any(RUST_RE.search(f) for f in later):
                p.append("Rust files changed after the receipt head (receipt is stale for Rust)")
            base = doc.get("base_ref")
            actual = _git_lines(repo, "diff", "--name-only", base, "HEAD") if isinstance(base, str) and base else None
            if actual is None:
                p.append("receipt base_ref missing or unresolvable; rust_files_changed cannot be recomputed")
            else:
                rust = sorted(f for f in actual if RUST_RE.search(f))
                if isinstance(declared, list) and sorted(declared) != rust:
                    p.append(f"rust_files_changed does not match git ({len(rust)} Rust file(s) since {base})")
                if rust and "cargo" not in str((doc.get("legs") or {}).get("ci", {}).get("command", "")).lower():
                    p.append("Rust files changed but the ci leg did not run cargo")
                else:
                    # a cargo leg that names --manifest-path covers only that workspace's directory
                    cmd = str((doc.get("legs") or {}).get("ci", {}).get("command", ""))
                    roots = [m.rsplit("/", 1)[0] if "/" in m else "" for m in
                             (x.replace("\\", "/").lstrip("./") for x in re.findall(r"--manifest-path[ =]['\"]?([^\s'\"]+)", cmd))]
                    if rust and roots:
                        out = [f for f in rust if not any(r == "" or f == r or f.startswith(r + "/") for r in roots)]
                        if out:
                            p.append(f"Rust files outside the cargo leg's --manifest-path coverage: {', '.join(out)}")
    return _res("g0p", not p, "; ".join(p) or f"pre-pr-gate pass at {head[:12]}")


def check_trn0(path):
    doc, prob = _json(path)
    if prob:
        return _res("trn0", False, prob)
    p = []
    if doc.get("schema") != "train-receipt/v1" or doc.get("verdict") != "pass":
        p.append("train receipt must be train-receipt/v1 with verdict pass")
    prs = doc.get("prs") or []
    if not prs:
        p.append("train receipt lists no PRs")
    for pr in prs:
        w, prob = _json(path.parent / str(pr.get("w0_receipt", "")))
        if prob:
            p.append(f"PR {pr.get('n')}: W0 receipt {prob}")
        else:
            p += check_receipt_doc(w, f"PR {pr.get('n')} W0 receipt")
    dev = doc.get("deviations") or []
    ok_msg = f"{len(prs)} train PR(s), each with a passing W0 receipt" + (f"; deviations: {' | '.join(map(str, dev))}" if dev else "")
    return _res("trn0", not p, "; ".join(p) or ok_msg)


def check_crv0(reviews: Path, required):
    code, lines = crv0_closure.run(reviews, list(required))
    bad = [l for l in lines if not l.startswith("closed")]
    return _res("crv0", code == 0, "; ".join(bad) or f"{len(lines)} review logs closed")


def check_freeze(root: Path):
    try:
        problems = freeze_mod().verify(root)
    except (OSError, ValueError) as e:
        problems = [f"freeze pin unreadable ({type(e).__name__})"]
    return _res("freeze", not problems, "; ".join(problems) or "C-2 and C-12 match FREEZE.json")


def honesty_results(report: dict, replay: dict) -> dict:
    """The eight honesty tests evaluated on the report itself: test n fails iff rule Hn has a violation."""
    er = engine_run()
    viol = er.check(report) + [{"rule": "H3", "where": "mapping", "msg": str(v)} for v in replay.get("mapping_mutation_violations") or []]
    out = {t: [v for v in viol if v["rule"] == rule] for t, rule in HONESTY_TESTS.items()}
    out["other"] = [v for v in viol if v["rule"] not in HONESTY_TESTS.values()]
    return out


def check_honesty(report, replay):
    res = honesty_results(report, replay)
    bad = [f"{t}: " + ", ".join(f"{v['where']}: {v['msg']}" for v in vs) for t, vs in res.items() if vs]
    return _res("honesty", not bad, " | ".join(bad) or "tests 1-8, G1 and S1 clean on the GT0 report")


def check_scanner_ids(report):
    er = engine_run()
    ids, p = set(), []
    for s in report.get("steps", []):
        rc = s.get("receipt") or {}
        sid = rc.get("scanner_id")
        if sid:
            ids.add(sid)
            if sid not in er.KNOWN_SCANNER_IDS:
                p.append(f"{s.get('id')}: unknown scanner id {sid!r}")
        elif er._norm(rc.get("provider")) in {er._norm(x) for x in er.THIRD_PARTY_PROVIDERS}:
            p.append(f"{s.get('id')}: third-party step without a scanner id")
    if not ids:
        p.append("no scanner id on any step")
    return _res("scanner_ids", not p, "; ".join(p) or "scanner ids " + ", ".join(sorted(ids)))


def check_doubles(report):
    parts = {d.get("part") for d in report.get("doubles") or []}
    p = [f"doubles[] lacks {x}" for x in PLAN_DOUBLES if x not in parts]
    if report.get("label") != "DEMO-0":
        p.append("report label must be DEMO-0")
    if report.get("host") != "python":
        p.append("report host must be python")
    if report.get("quality_claims") != "forbidden":
        p.append("quality_claims must be forbidden")
    return _res("doubles", not p, "; ".join(p) or "doubles[] lists the plan parts; DEMO-0, host=python, quality_claims forbidden")


def check_replay(path, report_path, repo=ROOT):
    doc, prob = _json(path)
    if prob:
        return _res("replay", False, prob)
    p = []
    if doc.get("schema") != "gt0-replay/v1" or doc.get("mode") != "replay":
        p.append("replay record must be gt0-replay/v1 with mode replay")
    if not doc.get("run_id"):
        p.append("no run_id")
    if doc.get("misses") != 0:
        p.append(f"digest misses {doc.get('misses')!r}, need 0")
    if not isinstance(doc.get("calls"), int) or doc["calls"] <= 0:
        p.append("replay served no calls")
    fx = Path(repo) / "e2e-core" / "tests" / "fixtures" / "thread01_queue"
    if fx.is_dir():
        import gt0_collect
        if doc.get("fixtures_digest") != gt0_collect.fixtures_digest(fx):
            p.append("fixtures_digest does not match the recorded fixtures")
    if report_path is None or not report_path.is_file() or doc.get("report_sha256") != sha256_of(report_path):
        p.append("replay record is not bound to the report bytes (report_sha256)")
    return _res("replay", not p, "; ".join(p) or f"run {doc['run_id']}: {doc['calls']} calls, 0 misses")


def check_live(path, thread01=None):
    """The live-window log must be well formed AND agree with the facts THREAD01.md records (calls and minutes per window)."""
    doc, prob = _json(path)
    if prob:
        return _res("live_window", False, prob)
    ws = doc.get("windows") or []
    p = []
    if doc.get("schema") != "live-window-log/v1":
        p.append("live log must be live-window-log/v1")
    if not any(str(w.get("kind", "")).startswith("roleplay") for w in ws):
        p.append("no live roleplay window recorded")
    if not str(doc.get("provenance") or "").strip():
        p.append("live log states no provenance")
    for w in ws:
        if not isinstance(w.get("scanner_rejections"), int) or not isinstance(w.get("calls"), int):
            p.append(f"window {w.get('id')}: needs integer calls and scanner_rejections")
    t = Path(thread01) if thread01 else None
    if t is None or not t.is_file():
        p.append("THREAD01.md not found: the log cannot be checked against the recorded facts")
    else:
        text = t.read_text(encoding="utf-8")
        for w in ws:
            if not isinstance(w.get("calls"), int) or not isinstance(w.get("minutes"), (int, float)):
                p.append(f"window {w.get('id')}: needs numeric minutes")
            elif f"{w['calls']} (scout" not in text:
                p.append(f"window {w.get('id')}: {w['calls']} calls not recorded in THREAD01.md")
            elif f"{w['minutes']:.2f}" not in text:
                p.append(f"window {w.get('id')}: {w['minutes']} minutes not recorded in THREAD01.md")
    return _res("live_window", not p, "; ".join(p) or f"{len(ws)} live window(s) logged, consistent with THREAD01.md")


def check_capacity(path):
    doc, prob = _json(path)
    if prob:
        return _res("capacity", False, prob)
    ss = doc.get("sessions") or []
    nums = all(isinstance(s.get("lane_hours"), (int, float)) for s in ss)
    p = []
    if doc.get("schema") != "capacity/v1" or len(ss) < MIN_SESSIONS or not nums:
        p.append(f"need capacity/v1 with numeric lane_hours for sessions 1..{MIN_SESSIONS} (have {len(ss)})")
    if not isinstance(doc.get("baseline_lane_hours_per_session"), (int, float)):
        p.append("no baseline_lane_hours_per_session")
    return _res("capacity", not p, "; ".join(p) or f"re-baselined from {len(ss)} sessions")


def evaluate(manifest: Path, repo: Path = ROOT) -> list:
    manifest = Path(manifest)
    m, prob = _json(manifest)
    if prob:
        return [_res("manifest", False, prob)]
    base = manifest.parent
    art = {k: (base / v) for k, v in (m.get("artifacts") or {}).items() if isinstance(v, str)}
    missing = lambda k: art.get(k) or base / f"<no {k} in manifest>"  # noqa: E731
    out = [check_g0p(missing("g0p"), repo, m.get("require_g0p_ancestry", True) is not False), check_trn0(missing("trn0")),
           check_crv0(base / m.get("reviews_dir", "docs/reviews/claude"), sorted(set(crv0_closure.DEFAULT_REQUIRED) | {w for w in (m.get("required_wps") or []) if isinstance(w, str)}))]
    report, p1 = _json(missing("report"))
    replay, p2 = _json(missing("replay"))
    if p1 or p2:
        out += [_res(k, False, p1 or "report unavailable") for k in ("honesty", "scanner_ids", "doubles")]
        if p2 and not p1:
            out[-3] = _res("honesty", False, p2)
    else:
        out += [check_honesty(report, replay), check_scanner_ids(report), check_doubles(report)]
    out += [check_freeze(repo), check_replay(missing("replay"), art.get("report"), repo),
            check_live(missing("live_log"), repo / "e2e-core" / "THREAD01.md"),
            check_capacity(missing("capacity"))]
    return out


def main(argv=None) -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--manifest", required=True)
    ap.add_argument("--repo", default=str(ROOT))
    ap.add_argument("--out")
    a = ap.parse_args(argv)
    res = evaluate(Path(a.manifest), Path(a.repo))
    for r in res:
        print(f"{'PASS' if r['ok'] else 'FAIL'} {r['item']}: {r['detail']}")
    ok = all(r["ok"] for r in res)
    print("gt0-gate:", "pass" if ok else "fail")
    if a.out:
        Path(a.out).write_text(json.dumps({"schema": "gt0-gate-result/v1", "verdict": "pass" if ok else "fail",
                                           "items": res}, indent=1) + "\n", encoding="utf-8")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())

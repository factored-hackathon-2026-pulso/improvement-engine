"""CRV0 review closure: validates docs/reviews/claude/*.review.json logs and exits 0 only when every reviewed WP
(and every required WP) has a closed log whose reviewer differs from its author.

Log format (review-log/v1): see docs/reviews/claude/README.md.
Usage: python crv0_closure.py [--reviews DIR] [--required WP,WP,...]
"""
from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
DEFAULT_REVIEWS = ROOT / "docs" / "reviews" / "claude"
# CRV0 dependencies in wp_table.csv plus the E0 data path named in its title.
DEFAULT_REQUIRED = ["TPS", "DC0", "G1", "M2a", "M0RP", "ED0", "ED0L", "SMAP", "M3", "E0-DATA-PATH"]
STATUSES = {"open", "fixed", "accepted"}
PROVENANCE = {"contemporaneous", "reconstructed"}


def _norm(x) -> str:
    return re.sub(r"[\s_\-]+", "-", x.strip().casefold()) if isinstance(x, str) else ""


def check_log(log: dict) -> list:
    """Format problems of one log (empty list = valid)."""
    p = []
    if not isinstance(log, dict) or log.get("schema") != "review-log/v1":
        return ["schema must be review-log/v1"]
    wps = log.get("wps")
    if not (isinstance(wps, list) and wps and all(isinstance(w, str) and w for w in wps)):
        p.append("wps must be a non-empty list of WP ids")
    a, r = (log.get("author") or {}).get("id"), (log.get("reviewer") or {}).get("id")
    if not _norm(a):
        p.append("author.id missing")
    if not _norm(r):
        p.append("reviewer.id missing")
    if _norm(a) and _norm(a) == _norm(r):
        p.append("reviewer must differ from author")
    prov = log.get("provenance")
    if prov not in PROVENANCE:
        p.append("provenance must be contemporaneous or reconstructed")
    if prov == "reconstructed" and not log.get("sources"):
        p.append("a reconstructed log must cite its sources")
    findings = log.get("findings")
    if not isinstance(findings, list):
        return p + ["findings must be a list"]
    for f in findings:
        fid = f.get("id", "?") if isinstance(f, dict) else "?"
        if not isinstance(f, dict) or not f.get("id") or not f.get("summary") or not isinstance(f.get("loop"), int):
            p.append(f"finding {fid}: needs id, summary and integer loop")
            continue
        st = f.get("status")
        if st not in STATUSES:
            p.append(f"finding {fid}: status must be one of {sorted(STATUSES)}")
        elif st == "fixed" and not f.get("fix_ref"):
            p.append(f"finding {fid}: fixed needs fix_ref")
        elif st == "accepted" and not (f.get("reason") or "").strip():
            p.append(f"finding {fid}: accepted needs a reason")
        elif st == "open" and log.get("verdict") == "closed":
            p.append(f"finding {fid}: open finding in a log with verdict closed")
    if log.get("verdict") not in ("closed", "open"):
        p.append("verdict must be closed or open")
    return p


def is_closed(log: dict) -> bool:
    return (not check_log(log) and log.get("verdict") == "closed"
            and all(f.get("status") != "open" for f in log.get("findings", [])))


def run(reviews: Path, required: list) -> tuple[int, list]:
    lines, ok, covered = [], True, set()
    paths = sorted(reviews.glob("*.review.json")) if reviews.is_dir() else []
    if not paths:
        return 1, [f"no review logs in {reviews}"]
    for path in paths:
        try:
            log = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, ValueError) as e:
            ok = False
            lines.append(f"FAIL {path.name}: unreadable ({type(e).__name__})")
            continue
        problems = check_log(log)
        wps = log.get("wps", []) if isinstance(log, dict) else []
        if problems:
            ok = False
            lines += [f"FAIL {path.name}: {x}" for x in problems]
        elif not is_closed(log):
            ok = False
            n = sum(1 for f in log["findings"] if f["status"] == "open")
            lines.append(f"OPEN {path.name} [{','.join(wps)}]: {n} open finding(s), verdict {log['verdict']}")
        else:
            covered.update(wps)
            lines.append(f"closed {path.name} [{','.join(wps)}] ({log['provenance']})")
    for w in required:
        if w not in covered:
            ok = False
            lines.append(f"MISSING closed log for required WP {w}")
    return (0 if ok else 1), lines


def main(argv=None) -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--reviews", default=str(DEFAULT_REVIEWS))
    ap.add_argument("--required", default=",".join(DEFAULT_REQUIRED))
    a = ap.parse_args(argv)
    code, lines = run(Path(a.reviews), [w for w in a.required.split(",") if w])
    print("\n".join(lines))
    print("crv0-closure:", "pass" if code == 0 else "fail")
    return code


if __name__ == "__main__":
    sys.exit(main())

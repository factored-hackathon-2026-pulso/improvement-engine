"""TRN0 train integrator for W0: lane branches merge in dependency order, one PR per 25-35 lane-hours, a restack check,
a W0 (pre-pr-gate/v1) receipt per train PR, and an exchange/ bundle when a PR reaches the cap.

Manifest (train/v1): {"schema","base","train_branch","pr_cap_hours","receipts_dir","lanes":[{"id","branch","deps":[ids],"lane_hours"}]}
Lanes are listed in merge order. The receipt of train PR n is <receipts_dir>/pr-<n>.json.

Usage: python trn0_train.py check|merge|bundle|receipt --manifest M.json --repo PATH [--exchange DIR]
`merge` writes only to the local train branch; it never pushes.
"""
from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path

DEFAULT_CAP = 35
LEGS = ("ci", "pytest", "ratchet")


def check_order(lanes: list) -> list:
    problems, seen = [], set()
    ids = [l["id"] for l in lanes]
    for i in {x for x in ids if ids.count(x) > 1}:
        problems.append(f"duplicate lane id {i}")
    for l in lanes:
        for d in l.get("deps", []):
            if d not in ids:
                problems.append(f"{l['id']}: unknown dependency {d}")
            elif d not in seen:
                problems.append(f"{l['id']}: out of order, dependency {d} is not merged before it")
        seen.add(l["id"])
    return problems


def plan(lanes: list, cap: float = DEFAULT_CAP) -> list:
    """Group lanes, in order, into train PRs whose lane-hours stay within the cap."""
    groups, cur, hours = [], [], 0.0
    for l in lanes:
        if cur and hours + l["lane_hours"] > cap:
            groups.append(cur)
            cur, hours = [], 0.0
        cur.append(l)
        hours += l["lane_hours"]
    return groups + ([cur] if cur else [])


def check_plan(lanes: list, cap: float = DEFAULT_CAP) -> list:
    return [f"{l['id']}: {l['lane_hours']} lane-hours exceeds the PR cap {cap}; split the lane" for l in lanes
            if l["lane_hours"] > cap]


def check_receipt(path: Path) -> list:
    if not path.is_file():
        return [f"W0 receipt {path.name} missing"]
    try:
        r = json.loads(path.read_text(encoding="utf-8"))
    except ValueError:
        return [f"W0 receipt {path.name} is not JSON"]
    p = []
    if r.get("schema") != "pre-pr-gate/v1":
        p.append(f"{path.name}: schema must be pre-pr-gate/v1")
    if r.get("verdict") != "pass":
        p.append(f"{path.name}: verdict is {r.get('verdict')!r}, not pass")
    for leg in LEGS:
        st = (r.get("legs") or {}).get(leg, {}).get("status")
        if st != "pass":
            p.append(f"{path.name}: leg {leg} is {st or 'absent'}")
    return p


def _git(repo, *args, check=True):
    return subprocess.run(["git", "-C", str(repo), *args], capture_output=True, text=True, check=check)


def _is_ancestor(repo, a, b) -> bool:
    return _git(repo, "merge-base", "--is-ancestor", a, b, check=False).returncode == 0


def check_restack(repo, lanes: list, base: str = "main") -> list:
    """Every lane branch must contain its dependencies' tips (and the base): otherwise it needs a restack."""
    by_id = {l["id"]: l for l in lanes}
    p = []
    for l in lanes:
        if _git(repo, "rev-parse", "--verify", "-q", l["branch"], check=False).returncode != 0:
            p.append(f"{l['id']}: branch {l['branch']} not found")
            continue
        for tip in [base] + [by_id[d]["branch"] for d in l.get("deps", []) if d in by_id]:
            if not _is_ancestor(repo, tip, l["branch"]):
                p.append(f"{l['id']}: restack needed, {l['branch']} does not contain {tip}")
    return p


def merge(repo, base: str, train_branch: str, lanes: list) -> None:
    order = check_order(lanes)
    if order:
        raise ValueError("; ".join(order))
    _git(repo, "checkout", "-q", "-B", train_branch, base)
    for l in lanes:
        r = _git(repo, "-c", "user.name=trn0", "-c", "user.email=trn0@example.invalid",
                 "merge", "--no-ff", "-q", "-m", f"train: merge {l['branch']}", l["branch"], check=False)
        if r.returncode != 0:
            _git(repo, "merge", "--abort", check=False)
            raise RuntimeError(f"merge of {l['branch']} failed: {r.stdout.strip()} {r.stderr.strip()}")


def write_bundles(repo, base: str, lanes: list, cap: float, out_dir: Path) -> list:
    """Write exchange/pr-<n>.bundle for every train PR that reached the cap (all groups except the still-open last one)."""
    groups = plan(lanes, cap)
    full = groups[:-1] + ([groups[-1]] if groups and sum(l["lane_hours"] for l in groups[-1]) >= cap else [])
    out_dir.mkdir(parents=True, exist_ok=True)
    written = []
    for n, g in enumerate(groups, 1):
        if g not in full:
            continue
        f = out_dir / f"pr-{n}.bundle"
        _git(repo, "bundle", "create", str(f), f"^{base}", *[l["branch"] for l in g])
        written.append(f)
    return written


def check_all(repo, m: dict) -> list:
    lanes, cap = m["lanes"], m.get("pr_cap_hours", DEFAULT_CAP)
    problems = check_order(lanes) + check_plan(lanes, cap)
    if not problems:
        problems += check_restack(repo, lanes, m.get("base", "main"))
    for n, _ in enumerate(plan(lanes, cap), 1):
        problems += check_receipt(Path(m["receipts_dir"]) / f"pr-{n}.json")
    return problems


def make_receipt(repo, m: dict, out: Path) -> dict:
    """Write a train-receipt/v1: the check verdict plus, per train PR, its lanes, lane-hours and a copy of its W0 receipt."""
    import datetime
    import shutil
    lanes, cap = m["lanes"], m.get("pr_cap_hours", DEFAULT_CAP)
    problems = check_all(repo, m)
    out.parent.mkdir(parents=True, exist_ok=True)
    prs = []
    for n, g in enumerate(plan(lanes, cap), 1):
        src = Path(m["receipts_dir"]) / f"pr-{n}.json"
        ref = None
        if src.is_file():
            ref = f"w0-pr-{n}.json"
            shutil.copyfile(src, out.parent / ref)
        prs.append({"n": n, "lanes": [l["id"] for l in g], "lane_hours": sum(l["lane_hours"] for l in g), "w0_receipt": ref})
    head = _git(repo, "rev-parse", "HEAD", check=False).stdout.strip()
    doc = {"schema": "train-receipt/v1", "generated_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
           "base": m.get("base", "main"), "train_branch": m.get("train_branch"), "head_sha": head,
           "pr_cap_hours": cap, "prs": prs, "problems": problems, "verdict": "fail" if problems else "pass"}
    out.write_text(json.dumps(doc, indent=2) + "\n", encoding="utf-8", newline="\n")
    return doc


def main(argv=None) -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("cmd", choices=["check", "merge", "bundle", "receipt"])
    ap.add_argument("--manifest", required=True)
    ap.add_argument("--repo", required=True)
    ap.add_argument("--exchange", default="exchange")
    ap.add_argument("--out", help="receipt: where to write train-receipt/v1")
    a = ap.parse_args(argv)
    m = json.loads(Path(a.manifest).read_text(encoding="utf-8"))
    if a.cmd == "check":
        problems = check_all(a.repo, m)
        print("\n".join(problems) or "train check: clean")
        return 1 if problems else 0
    if a.cmd == "receipt":
        doc = make_receipt(a.repo, m, Path(a.out))
        print(f"train receipt {doc['verdict']}: {a.out}")
        return 0 if doc["verdict"] == "pass" else 1
    if a.cmd == "merge":
        problems = check_order(m["lanes"]) + check_restack(a.repo, m["lanes"], m.get("base", "main"))
        if problems:
            print("\n".join(problems))
            return 1
        merge(a.repo, m.get("base", "main"), m["train_branch"], m["lanes"])
        print(f"merged {len(m['lanes'])} lanes into {m['train_branch']} (local only)")
        return 0
    files = write_bundles(a.repo, m.get("base", "main"), m["lanes"], m.get("pr_cap_hours", DEFAULT_CAP), Path(a.exchange))
    print("\n".join(map(str, files)) or "no train PR has reached the cap")
    return 0


if __name__ == "__main__":
    sys.exit(main())

#!/usr/bin/env python3
"""SYNTHETIC post-release cell table for the demo clock (R1): the planted table with the TREATED cell's numerator cut after a pseudo release.

    post_cells.py --in planted.ndjson --out post.ndjson --release 2025-06 --category "Cobro indebido" --cut-pp 8

Rows of the treated category with period > release lose `cut-pp` percentage points of their denominator (numerator floored at 10, never above
the denominator); every other row is copied. The caller labels the output synthetic (PULSO_OUTCOME_DATA_LABEL); it proves the `improved` path only.
Aggregates only, no customer data.
"""
import argparse
import json
import sys
from pathlib import Path


def cut_rows(rows, release, category, cut_pp):
    out = []
    for r in rows:
        r = dict(r)
        if r.get("dims", {}).get("category") == category and r["period"] > release:
            r["numerator"] = max(10, min(r["denominator"], r["numerator"] - round(r["denominator"] * cut_pp / 100)))
        out.append(r)
    return out


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--in", dest="src", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--release", required=True)
    ap.add_argument("--category", required=True)
    ap.add_argument("--cut-pp", type=float, default=8.0)
    a = ap.parse_args(argv)
    rows = [json.loads(x) for x in Path(a.src).read_text(encoding="utf-8").splitlines() if x.strip()]
    res = cut_rows(rows, a.release, a.category, a.cut_pp)
    Path(a.out).write_text("".join(json.dumps(r, sort_keys=True, separators=(",", ":")) + "\n" for r in res), encoding="utf-8", newline="\n")
    print(json.dumps({"rows": len(res), "changed": sum(1 for x, y in zip(rows, res) if x != y), "label": "synthetic-planted-effect"}))
    return 0


if __name__ == "__main__":
    sys.exit(main())

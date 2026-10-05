#!/usr/bin/env python3
"""SYNTHETIC planted-cell table for the demo loop (labelled synthetic everywhere it is shown).

    python scripts/demo-loop/planted_cells.py --out cells.ndjson [--seed 7]

One metric (M4 pqr_open_rate, by `category`) over 35 months and both halves (discovery / holdout). Four categories share a base
open rate of about 33%; ONE category ("Cobro indebido", the star story's dispute front) is planted at about 62% in BOTH halves, so
the cells sensor corroborates exactly one association and the finding maps to the `t/estado_pqr` template patch (the announceable
path proven in W11/W13). The numbers are invented: no customer, no bank row, no identifier. Same schema as
scripts/aggregate/bank_cells.py (every count is 0 or >= 10). Deterministic for a seed.
"""
from __future__ import annotations

import argparse
import json
import random
import sys
from pathlib import Path

CATEGORIES = ("Cobro indebido", "Cargo no reconocido", "Problema con app", "Calidad de servicio", "Atencion en sucursal")
PLANTED = "Cobro indebido"
SECOND_PLANTED = "Problema con app"  # only with --plant 2 (the Langfuse closure wants two stories)
BASE_RATE = 0.33
PLANTED_RATE = 0.62


def periods() -> list[str]:
    out = []
    for y, m in ((2023, range(7, 13)), (2024, range(1, 13)), (2025, range(1, 13)), (2026, range(1, 6))):
        out.extend(f"{y}-{mo:02d}" for mo in m)
    return out


def build(seed: int = 7, plant: int = 1) -> list[dict]:
    planted = (PLANTED, SECOND_PLANTED)[:max(1, min(plant, 2))]
    rng = random.Random(seed)
    rows = []
    for period in periods():
        for half in ("discovery", "holdout"):
            for cat in CATEGORIES:
                den = rng.randint(150, 230)
                rate = PLANTED_RATE if cat in planted else BASE_RATE
                num = max(10, min(den - 10, round(den * rate + rng.randint(-6, 6))))
                rows.append({"metric": "M4", "dims": {"category": cat}, "half": half, "period": period, "numerator": num, "denominator": den})
    return rows


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--out", required=True)
    ap.add_argument("--seed", type=int, default=7)
    ap.add_argument("--plant", type=int, default=1, choices=[1, 2], help="how many categories are planted (default 1)")
    a = ap.parse_args(argv)
    rows = build(a.seed, a.plant)
    Path(a.out).parent.mkdir(parents=True, exist_ok=True)
    with open(a.out, "w", encoding="utf-8", newline="\n") as f:
        for r in rows:
            f.write(json.dumps(r, sort_keys=True, separators=(",", ":")) + "\n")
    print(json.dumps({"rows": len(rows), "metric": "M4", "planted": PLANTED, "label": "synthetic"}))
    return 0


if __name__ == "__main__":
    sys.exit(main())

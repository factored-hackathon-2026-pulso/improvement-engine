#!/usr/bin/env python3
"""OUT1 reference adapter: the `pulso.outcome.v1` JSON CLI over an outcome estimator that exposes
`estimate_outcomes(rows, release_period, window_months=...)` (Codex T1: `scripts.aggregate.outcome.outcome_estimator`).

This file contains NO estimator code. It imports the estimator when it is on PYTHONPATH (today: a copy outside git, because
Codex's T1 is not merged) and narrows its all-cells report to the one treated cell the engine asked about.

    python outcome_cli_adapter.py --cells <ndjson> --treated <json> --release-date YYYY-MM [--window-months N]

stdout: one JSON object {contract, release_period, window_months, multiplicity, cells: [<the treated cell's verdict>]}.
Exit codes: 0 ok; 2 bad arguments or input rows; 3 estimator not importable. Nothing but aggregates is read or written.
"""
import argparse
import json
import sys

CONTRACT = "pulso.outcome.v1"
CELL_FIELDS = {"metric", "dims", "half", "period", "numerator", "denominator"}


def load_rows(path):
    rows = []
    with open(path, encoding="utf-8") as stream:
        for n, line in enumerate(stream, 1):
            if not line.strip():
                continue
            row = json.loads(line)
            if not isinstance(row, dict) or set(row) != CELL_FIELDS:
                raise ValueError(f"line {n} is not a bank_cells aggregate row")
            rows.append(row)
    return rows


def _estimator():
    from scripts.aggregate.outcome.outcome_estimator import estimate_outcomes  # noqa: PLC0415
    return estimate_outcomes


def run(cells_path, treated_path, release_date, window_months, estimate=None):
    rows = load_rows(cells_path)
    with open(treated_path, encoding="utf-8") as stream:
        treated = json.load(stream)
    estimate = estimate or _estimator()
    report = estimate(rows, release_date, window_months=window_months)
    cells = [c for c in report.get("cells", []) if c.get("metric") == treated.get("metric") and c.get("dims") == treated.get("dims")]
    return {
        "contract": CONTRACT,
        "release_period": report.get("release_period", release_date),
        "window_months": report.get("window_months", window_months),
        "multiplicity": report.get("multiplicity"),
        "cells": cells,
    }


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--cells", required=True)
    p.add_argument("--treated", required=True)
    p.add_argument("--release-date", required=True)
    p.add_argument("--window-months", type=int, default=3)
    a = p.parse_args(argv)
    try:
        y, m = a.release_date.split("-")
        if not (len(y) == 4 and y.isdigit() and len(m) == 2 and m.isdigit() and 1 <= int(m) <= 12):
            raise ValueError
    except ValueError:
        print("release-date must be YYYY-MM", file=sys.stderr)
        return 2
    try:
        out = run(a.cells, a.treated, a.release_date, a.window_months)
    except ImportError as exc:
        print(f"estimator not importable: {exc}", file=sys.stderr)
        return 3
    except (ValueError, OSError) as exc:
        print(f"bad input: {exc}", file=sys.stderr)
        return 2
    print(json.dumps(out, sort_keys=True, separators=(",", ":")))
    return 0


if __name__ == "__main__":
    sys.exit(main())

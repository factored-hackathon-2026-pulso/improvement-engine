#!/usr/bin/env python3
"""DEMO-ONLY wrapper of scripts/out1/outcome_cli_adapter.py for the SYNTHETIC planted profile.

The T1 estimator accepts a closed set of PQR categories (transactions, fees, technical, branch, service); the planted table uses Spanish category
names ("Cobro indebido"). This wrapper maps the planted names to that set on the way in (cells and treated file) and back on the way out, then
delegates to the real adapter. It contains no estimator code and exists only because the planted labels are invented. Same CLI contract
(`pulso.outcome.v1`): --cells F --treated F --release-date YYYY-MM --window-months N.
"""
import contextlib
import io
import json
import os
import runpy
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
ADAPTER = HERE.parent / "out1" / "outcome_cli_adapter.py"
FORWARD = {"Cobro indebido": "fees", "Cargo no reconocido": "transactions", "Problema con app": "technical",
           "Atencion en sucursal": "branch", "Calidad de servicio": "service"}
BACK = {v: k for k, v in FORWARD.items()}
DEMO_MIN_WINDOW_N = 300  # R4 demo floor (the estimator default is 1500 contacts per window); only with a synthetic data label


def map_dims(obj, table):
    """Replace a `category` value found in any dims-like dict, recursively."""
    if isinstance(obj, dict):
        return {k: (table.get(v, v) if k == "category" and isinstance(v, str) else map_dims(v, table)) for k, v in obj.items()}
    if isinstance(obj, list):
        return [map_dims(x, table) for x in obj]
    return obj


def lower_floor():
    """R4: lower the estimator's minimum window support, ONLY for a synthetic data label. Returns the floor used."""
    if not os.environ.get("PULSO_OUTCOME_DATA_LABEL", "").startswith("synthetic"):
        raise SystemExit("demo floor refused: PULSO_OUTCOME_DATA_LABEL is not synthetic-*")
    root = str(Path.cwd())
    if root not in sys.path:
        sys.path.insert(0, root)
    from scripts.aggregate.outcome import outcome_estimator as oe  # noqa: PLC0415
    fn = oe.estimate_outcomes
    kw = dict(fn.__kwdefaults__ or {})
    if "min_window_n" in kw:
        kw["min_window_n"] = DEMO_MIN_WINDOW_N
        fn.__kwdefaults__ = kw
    else:  # positional default
        names = fn.__code__.co_varnames[:fn.__code__.co_argcount]
        d = list(fn.__defaults__ or ())
        d[len(d) - (len(names) - names.index("min_window_n"))] = DEMO_MIN_WINDOW_N
        fn.__defaults__ = tuple(d)
    return DEMO_MIN_WINDOW_N


def main(argv):
    args = list(argv)
    cells_i, treated_i = args.index("--cells") + 1, args.index("--treated") + 1
    floor = lower_floor()
    with tempfile.TemporaryDirectory() as tmp:
        cells_out, treated_out = Path(tmp, "cells.ndjson"), Path(tmp, "treated.json")
        lines = [json.dumps(map_dims(json.loads(x), FORWARD), sort_keys=True, separators=(",", ":"))
                 for x in Path(args[cells_i]).read_text(encoding="utf-8").splitlines() if x.strip()]
        cells_out.write_text(chr(10).join(lines) + chr(10), encoding="utf-8")
        treated_out.write_text(json.dumps(map_dims(json.loads(Path(args[treated_i]).read_text(encoding="utf-8")), FORWARD)), encoding="utf-8")
        args[cells_i], args[treated_i] = str(cells_out), str(treated_out)
        buf = io.StringIO()
        old_argv = sys.argv
        sys.argv = [str(ADAPTER), *args]
        try:
            with contextlib.redirect_stdout(buf):
                try:
                    runpy.run_path(str(ADAPTER), run_name="__main__")
                except SystemExit as exc:
                    if exc.code not in (None, 0):
                        sys.stderr.write(buf.getvalue())
                        return int(exc.code) if isinstance(exc.code, int) else 1
        finally:
            sys.argv = old_argv
    out = map_dims(json.loads(buf.getvalue()), BACK)
    out["support_profile"] = f"demo (min_window_n={floor}, synthetic data only)"
    sys.stdout.write(json.dumps(out))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))

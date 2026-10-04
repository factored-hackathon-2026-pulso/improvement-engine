"""CLI: python -m product_stream (run from platform-sim/). See docs/plan-real/r1-product-sim.md."""
from __future__ import annotations

import argparse
import json
import os
import sys
import threading
import time
from pathlib import Path

from . import DATA_ORIGIN
from .generator import ProductStream
from .scenarios import SCENARIO_NAMES
from .sinks import DEFAULT_DSN_ENV, PostgresSink, SqliteSink

DEFAULT_FOLLOW_HORIZON = 50000


def _parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(prog="product_stream", description=f"{DATA_ORIGIN}: synthetic, never the real platform")
    tgt = p.add_mutually_exclusive_group(required=True)
    tgt.add_argument("--sqlite", metavar="FILE", help="write the product tables into this SQLite file")
    tgt.add_argument("--postgres", action="store_true", help="write into schema product (DSN from env var)")
    p.add_argument("--dsn-env", default=DEFAULT_DSN_ENV, help="env var holding the Postgres DSN (never printed)")
    p.add_argument("--seed", type=int, default=0)
    p.add_argument("--scenario", default="null", choices=SCENARIO_NAMES)
    p.add_argument("--onset-sequence", type=int, default=None, help="override where the planted effect starts")
    p.add_argument("--backfill", type=int, metavar="N", help="write exactly N event_log rows, unpaced")
    p.add_argument("--follow", action="store_true", help="keep appending batches at --rate until stopped")
    p.add_argument("--rate", type=float, default=20.0, help="event_log rows per second in --follow")
    p.add_argument("--batch", type=int, default=50, help="rows per batch")
    p.add_argument("--horizon-events", type=int, default=None,
                   help="planned total events (windows and onset derive from it); default N for backfill-only")
    p.add_argument("--customers", type=int, default=400, help="synthetic customer pool size")
    p.add_argument("--start", default="2026-09-01T08:00:00Z", help="simulated clock start (UTC ISO)")
    p.add_argument("--manifest", metavar="FILE", help="write the planted-signal manifest here")
    p.add_argument("--stop-file", metavar="FILE", help="exit cleanly when this file exists")
    p.add_argument("--stop-on-stdin-eof", action="store_true", help="exit cleanly when stdin reaches EOF")
    p.add_argument("--overwrite", action="store_true", help="delete previously simulated rows first")
    return p


def _write_manifest(path: str | None, gen: ProductStream) -> None:
    if not path:
        return
    tmp = f"{path}.tmp"
    Path(tmp).write_text(json.dumps(gen.manifest(), indent=2, sort_keys=True), encoding="utf-8")
    os.replace(tmp, path)


def main(argv=None, sleep=time.sleep, stdin=None, connect=None) -> int:
    a = _parser().parse_args(argv)
    if a.backfill is None and not a.follow:
        _parser().error("give --backfill N and/or --follow")
    run_id = f"{a.scenario}-{a.seed}"
    try:
        sink = SqliteSink(a.sqlite) if a.sqlite else PostgresSink(a.dsn_env, connect=connect, run_id=run_id)
    except RuntimeError as e:
        print(f"error: {e}", file=sys.stderr)
        return 2
    except Exception as e:  # driver errors may echo connection details: print the class only
        print(f"error: could not connect ({type(e).__name__})", file=sys.stderr)
        return 2
    horizon = a.horizon_events or (a.backfill if not a.follow else max(a.backfill or 0, DEFAULT_FOLLOW_HORIZON))
    manifest_path = a.manifest or (f"{a.sqlite}.manifest.json" if a.sqlite else None)
    try:
        sink.ensure_schema()
        if sink.event_log_count() > 0:
            if not a.overwrite:
                print("error: the target already holds an event_log; use --overwrite to replace the simulated rows",
                      file=sys.stderr)
                return 2
            sink.reset()
        gen = ProductStream(seed=a.seed, scenario=a.scenario, horizon_events=horizon, start=a.start, n_customers=a.customers,
                            **({"onset_sequence": a.onset_sequence} if a.onset_sequence is not None else {}))
        if a.backfill:
            left = a.backfill
            while left > 0:
                batch = gen.next_batch(min(a.batch, left))
                sink.write_batch(batch)
                left -= len(batch["event_log"])
        if a.follow:
            eof = threading.Event()
            if a.stop_on_stdin_eof:
                src = stdin if stdin is not None else sys.stdin
                threading.Thread(target=lambda: (src.read(), eof.set()), daemon=True).start()
            n = 0
            try:
                while not eof.is_set() and not (a.stop_file and os.path.exists(a.stop_file)):
                    sink.write_batch(gen.next_batch(a.batch))
                    n += 1
                    if n % 20 == 0:
                        _write_manifest(manifest_path, gen)
                    sleep(a.batch / a.rate)
            except KeyboardInterrupt:
                pass
        _write_manifest(manifest_path, gen)
        print(json.dumps({"data_origin": DATA_ORIGIN, "scenario": a.scenario, "seed": a.seed,
                          "last_sequence": gen.last_sequence, "manifest": manifest_path}))
        return 0
    finally:
        sink.close()

# R1S: product-data simulator (`platform-sim/product_stream`)

**Honesty label.** Everything this tool writes is `data_origin = synthetic product-sim`. It is shaped like what the
support platform emits (event catalog 1.1.0, the exporter allow-list in `db/catalog.py`) so platform mode can be exercised
before the real platform runs on Postgres. It is never the real platform and must be reported as a double in `doubles[]`.
Ids are opaque (`CAS-1a2b3c4d`), every customer has `simulator = true`, there are no names, emails, phones or free text;
payloads carry enums, counters and ids only. Support-platform was used only to shape tables and event kinds.

## What it writes

Exactly the allow-listed `product` tables, nothing else: `event_log`, `cases`, `customers`, `staff`, `turns`,
`assignments`, `customer_case_slots`. `event_log.sequence` is dense and monotonic, `event_time` is non-decreasing,
`ingested_at >= event_time` (0-3 s lag). Event types are all admitted in catalog 1.1.0 (`case.opened|queued|assigned|
status_changed|read|first_responded|closed`, `turn.created`). The SQLite file has the same column names as the allow-list
(no lineage columns); Postgres rows also get `_batch_id` (`product-sim:<scenario>-<seed>:<n>`) and
`_source_file = product_stream`. A schema never gets a denylisted table or column (`sinks.validate_batch` refuses them).

## Run (from `platform-sim/`, no third-party deps for SQLite)

    # one-shot, deterministic
    python -m product_stream --sqlite out/product.sqlite --backfill 20000 --seed 7 --scenario escalation_rise
    # live stream: backfill, then follow at 20 events/s in batches of 50
    python -m product_stream --sqlite out/product.sqlite --backfill 5000 --follow --rate 20 --batch 50 \
        --horizon-events 100000 --stop-file out/STOP
    # Postgres (DSN only from the env var, never printed; needs `pip install psycopg[binary]`)
    $env:PULSO_PRODUCT_SIM_PG_DSN = "<dsn of a writer role>"   # PowerShell
    python -m product_stream --postgres --follow --rate 20 --scenario null --stop-on-stdin-eof

`--follow` ends cleanly (exit 0, manifest flushed) when the `--stop-file` exists, on stdin EOF with
`--stop-on-stdin-eof`, or on Ctrl-C. A target that already has an `event_log` is refused unless `--overwrite`
(SQLite: deletes all rows; Postgres: deletes only rows with `_source_file = product_stream`). The Postgres DDL is
`CREATE ... IF NOT EXISTS`, so it is a no-op over `db/sql/030_product.sql`. The writer needs INSERT/DELETE on `product.*`
(role `pulso_loader`, not the read-only `pulso_product_ro`). Output on stdout is one JSON summary line.

## Scenarios and manifest

Effects are functions of the `event_log` sequence, never of wall time or batch size: output depends only on
`(seed, scenario, horizon-events, onset)`. Default onset is 30% of the horizon; windows are discovery = (0, 60%] and
holdout = (60%, 100%], so a planted effect is present in BOTH windows; the null scenario has it in neither.

| `--scenario` | planted effect (cell = language/channel; target `pt/web_chat`) |
|---|---|
| `null` | nothing planted; reassignment 5% and reopen 6% everywhere |
| `escalation_rise` | manual reassignment (`case.assigned`, `reason=manual`, `previous_staff_id`) per case opened 5% -> 32% in the target cell. Stand-in for escalations: the allow-list has no `escalations`/`calls` tables (gap G2). |
| `recurrence_rise` | reopen rate (`cases.previous_case_id` set) 6% -> 36% in the target cell |
| `volume_drift` | arrival rate ramps 1x -> 4x from the onset (queueing and `waited_seconds` rise) |

`--manifest FILE` (default `<sqlite>.manifest.json`) is written every 20 batches and at exit: scenario, seed, horizon,
`planted` (effect, cell, onset_sequence, baseline/elevated, metric), `windows`, and `realised` counts per window so a
test can assert that a sensor finds the planted cell (and does not on `null`). Decide thresholds from the manifest, not
from the generator code.

## Tests and fixture

`uv run --python 3.12 --with pytest python -m pytest platform-sim/tests/pstream` (offline, ~2 s). The committed sample
`platform-sim/product_stream/fixtures/product_sample.sqlite` (98 KB, 300 events, seed 7, `escalation_rise`, 100 customers)
is checked against the catalog and the exporter allow-list column for column and must equal its regeneration:
`python -m product_stream --sqlite product_stream/fixtures/product_sample.sqlite --backfill 300 --seed 7 --scenario escalation_rise --horizon-events 300 --customers 100`.

## Gaps (honest)

- No live Postgres write test was run (psycopg not a dependency here; Postgres path tested with a fake connection and
  SQL-statement assertions only). First live use should be on a throwaway container.
- `escalations` and `calls` do not exist in the allow-list (G2), so escalation is modelled as supervisor reassignment.
- Staff availability, `case.viewed`, auth events and release events are not generated. No SLA breach status (no `status`
  column is allow-listed); breach is derivable from `sla_due_at` and turn/assignment times.
- Payload shapes are simulator assumptions like `platform_live`. Volumes and timing are plausible, not calibrated.
- Single process, in-memory state; `--follow` cannot resume a previous run's state (use `--overwrite`).

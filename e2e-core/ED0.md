# ED0: E0 ingest and local detection

Detection is done by the existing Rust local-sim sensor (`improvement-engine local-sim`, crate
`crates/runner`, read only here). `claude_standin/ed0_detect.py` invokes it and projects the result to an
aggregate-only record: one admitted family (`e0_recurring_copilot_query_cases`), recorded discards, holdout
status, `data_origin=generated_sample`, `providers=["local"]`. No planted column, no raw rows.

Build (one cargo job, own target dir):
`CARGO_TARGET_DIR=D:/cargo-targets/claude-ed0 cargo build -j 2 --locked --offline -p improvement-engine-runner`

Tests (synthetic fixtures written by the dependency-free `parquet_min.py`):
`ED0_RUNNER_EXE=<target>/debug/improvement-engine.exe`, PYTHONPATH `src;tests`, `unittest discover -s tests/unit -p "test_ed0*.py"`.
The rename and permute mutation test needs the exe; without it those tests skip.

Real E0 is read only at runtime, never copied or committed:
`python -m claude_standin.ed0_detect --exe <exe> --input <E0 dir> --output <new dir outside the repo>`
(or env `ED0_E0_PATH`).

## ED0L: treated lab

`claude_standin/ed0_lab.py` folds cases into per-(group, window) aggregates in memory and stores only
groups with count >= k (default 10) in a sqlite lab (`lab_rows`, `lab_meta`). Group keys are
`g_*` = HMAC-SHA256(salt, field NUL value) truncated to 16 hex. `lab_query` returns scanner-shaped rows
(numerator stays in the lab); `verify_claim` recomputes the rate exactly and checks a per-row digest.
The salt and any real-E0-derived lab stay local and untracked. Reading the real E0 parquet needs a reader
(not in the stdlib); until then the lab is fed by any iterable of (case_id, group, window, outcome).

## ED0b: original bank CSV (`--source original`)

The original dataset is hive-partitioned CSV (`<table>/year=/month=/day=/*.csv`) under the local data directory.
`claude_standin/ed0_original.py` reads one table at runtime (env `ED0_ORIGINAL_PATH`), keeps only the two columns the
caller names, and yields the same `(case_key, group, window, outcome)` shape as ED0F into the same ED0L lab (k and
min_cell as in `--e0`). Run:
`python -m claude_standin.thread01 --replay <queue> --source original --original-map table,case_col,group_col --summary <out outside repo>`.

Labels: steps 1-2 `original`, steps 3-4 `original-treated`, `doubles[]` `data.origin = original-treated-aggregates`; never
`generated_sample` and never mixed with E0. G1 treats `original*` as restricted, and `gw-hosted` rejects it (DC0).
Errors report the exception type only. Raw rows, ids and group labels exist only in memory.

Known gap (not forced): nothing in the original schema is declared as "a recurring case with a catalogue-matching
flow", and the Rust sensor reads the E0 parquet shape. So step 2 is `not_exercised(sensor_not_mapped_to_original_csv)`,
the SMAP ending is `unlinked`, steps 5-10 `not_exercised`. Which table, case and group columns mean "recurrence" is a
human decision passed via `--original-map`; an original-only finding is never reported as supported.

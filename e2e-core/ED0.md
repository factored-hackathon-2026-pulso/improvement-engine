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

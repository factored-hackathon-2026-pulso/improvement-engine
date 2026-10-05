# R3-4 exact-link aggregate run

R3-4 is a preregistered, retrospective descriptive analysis of the E0-enriched
sample linked to bank complaint rows by exact `complaint_id`. E0 query and
close events are generated enrichment; they are not observations of real bank
contact handling, model performance, causal mechanisms, lift, or savings.

## Frozen protocol and input boundary

Read [`PREREGISTRATION.md`](PREREGISTRATION.md) before running. The command
requires explicit roots for the original bank dataset and E0 `datos` directory,
an output directory that does not exist, and full Git commit SHAs for both the
preregistration and code. The CLI projects only its allowlisted parquet columns
and selects only the registered complaint CSV fields. It writes no source rows,
identifiers, free text, query signatures, or input paths. Input digests and row
counts are held only in an external local run manifest.

The runner refuses a pre-existing output directory, output inside the working
repository or either input root, E0 samples other than the preregistered 2,000
cases, and E0 cases with no exact complaint key. Duplicate keys, many-to-one
links, invalid event ordering, schema drift, or unsafe cells fail closed or
produce an explicitly suppressed aggregate table; they are not repaired after
observing results. Minimum support is fixed at `k=10`.

## Local command (PowerShell)

Replace `<PREREG_SHA>`, `<CODE_SHA>`, and the new output-directory suffix with
the full SHA/path for the run. Do not reuse an output directory. Keep generated
results outside Git and do not paste row-level source values into logs or chat.

```powershell
cargo run --offline `
  --manifest-path docs/data/opbench/e0-export/Cargo.toml `
  --bin opbench-r3-4 -- `
  --bank-data-root 'D:\.codex\factored\data' `
  --e0-data-dir 'D:\.codex\factored\pulso_muestra_e0\datos' `
  --output-dir 'D:\.codex\factored\tmp\r3-4-run-01' `
  --repo-root 'D:\.codex\factored\improvement-engine' `
  --prereg-commit '<PREREG_SHA>' `
  --code-revision '<CODE_SHA>'
```

The requested output directory's parent must already exist. A successful run
creates only:

- `run_manifest.json`: preregistration/code revision, fixed configuration,
  allowlist schema-contract identifiers, source-file digests, row counts, and
  the aggregate result digest. It contains no input paths.
- `results.json`: stable, aggregate-only tables; suppression status and
  provenance labels are explicit. It contains no source digests or row data.

The manifest records that no temporal cutoff is applied (the full registered
snapshot is used), deterministic partition/aggregate ordering, and fingerprints
of the observed allowlisted schema projection. It is written before aggregate calculation and finalized only after
the result file is written. For determinism, run the exact same command with a
different new output directory and compare `results.json` byte-for-byte. The
manifest differs only if its recorded code/config/input metadata differs.

## Local validation

```powershell
cargo fmt --manifest-path docs/data/opbench/e0-export/Cargo.toml -- --check
cargo test --offline --manifest-path docs/data/opbench/e0-export/Cargo.toml
cargo clippy --offline --manifest-path docs/data/opbench/e0-export/Cargo.toml --all-targets -- -D warnings
```

CI Actions are not treated as the first test environment. This repository's
Actions spending gate has previously prevented jobs from starting; report the
live gate separately from these local results.


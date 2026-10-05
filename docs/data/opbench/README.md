# OPBENCH-lite

Independent, deterministic aggregate benchmark for the bank contact/PQR
snapshot and operational E0 queries. It is not a causal analysis or a claim
about real-bank prevalence.

## Regenerate locally

From the repository root in PowerShell:

```powershell
python docs/data/opbench/generate.py `
  --data-root D:\.codex\factored\data `
  --e0-data D:\.codex\factored\pulso_muestra_e0\datos
```

The only bank table inputs are `call_center_interactions`, `complaints`, and
`satisfaction_surveys`; the only E0 tables read by the benchmark are `case`
and `copilot_query`. The Rust exporter validates the E0 package with the
production source adapter before it reads those two tables. Its standard
output is aggregate sufficient statistics only. Raw keys, signatures, query
text, answers, labels, timeline, and case-close tables are not serialized.

The output directory defaults to `results/` and contains only the aggregate
catalog and complete 181-cell audit. A second run over unchanged input bytes
must be byte-identical. Exact duplicate primary keys collapse; conflicting
duplicates and schema drift fail closed. Counts below 10 and their associated
rates/effects are suppressed. The catalog separates full-snapshot facts from
discovery-half effects and replication-half evidence. `discovery_v1.md` freezes the metrics and tests;
`CROSSCHECK.md` defines independent matching/scoring.

## Local tests

```powershell
python -m unittest discover -s docs/data/opbench -p 'test_*.py' -v
cargo test --offline --manifest-path docs/data/opbench/e0-export/Cargo.toml
```

No raw or intermediate dataset artifacts belong in Git. The JSON catalog is
aggregate-only; inspect it with `validate_safe_pack` before commit.

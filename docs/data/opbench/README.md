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

## V2 preregistered extension

V2 is a separate immutable version governed by [`discovery_v2.md`](discovery_v2.md)
and the strict [`opbench-lite-v2.schema.json`](opbench-lite-v2.schema.json). It
extends the audited opportunity catalog with channel/reason coverage,
digital-action context, a consent risk, and the E0 complaint-ID linkage. Those
descriptive context entries are not additional inferential tests. The catalog
also keeps registered negative controls and distinguishes corroborated
evidence from actionability; an already-covered capability is not presented
as a new opportunity.

Run V2 from the repository root in PowerShell, writing to a new directory
outside the checkout while validating real data:

```powershell
python docs/data/opbench/generate_v2.py `
  --data-root D:\.codex\factored\data `
  --e0-data D:\.codex\factored\pulso_muestra_e0\datos `
  --output-dir D:\.codex\factored\outcome-temp\opbench-v2-run
```

The command creates an immutable `v2/` directory containing the aggregate
catalog and audit JSON only after every source has been read and validated. It
never overwrites an existing `v2/` result directory. Commit the exact
preregistration revision before generation; then validate both files against
the V2 schema, inspect disclosure-safe counts and hashes, and compare a second
sequential run in a different outside-repository directory for byte-identical
output. Keep real-data outputs outside Git until privacy review approves any
aggregate for publication. Never copy intermediate tables, source identifiers,
free text, or row-level data into the repository.

V2 consumes bank complaints, contacts, digital events, campaign sends, the
customer consent flag, and operational E0 cases required by the frozen
adapter contract. Customer keys are used only as in-memory digests for
deduplication/linkage and are not output. The E0 join is aggregate-only; it
does not authorize reading labels or timeline tables. The V1 command and
`results/` artifacts remain unchanged.

The V2 catalog is a separately implemented comparison protocol, but scoring
the bank-cell sensor against it does not provide an independent dataset or
ground truth: both methods use the same underlying snapshot with different
customer-hash splits and statistical/support gates. Report the comparison as
cross-protocol agreement, not predictive accuracy, causal effect, or
production performance. Do not tune thresholds after reviewing the score
without a new preregistered version.

## Round-2 sensor comparison (exploratory, not preregistered)

The current real-data sensor was invoked twice, sequentially, from
`seams/target/debug/steps_cli.exe` against the same 3,657-row aggregate
bank-cell input (input SHA-256 `739cde15ff173c3e2b316cef81971cbf29898138bcbeba727b957acb05fb2974`).
Both signal files were byte-identical (SHA-256
`51cd9be86e1b94272b8b27528b83e53bbb873cfa26e0276ed1e9832a560712f9`). The
binary labels its implementation `claude-standin`; it explored 146 cells and
emitted 29 signals (19 `corroborated`, 9 `refuted`, 1 `uncertain`). Reported
signals comprise M1=11, M2=1, M3=1, M4=1, M5=1, M6=7, M6R=1, M6U=1, M7=1,
M8=1, M9=1, and M10=2.

The versioned scorer, using the frozen OPBENCH v2 vocabulary and keeping
M6R/M6U distinct, matched 10 of 22 catalog-positive cells and 10 of 13 reported
signals; one report matched a refuted M6 non-finding, six descriptive M6 cells
were ignored, and 12 positive cells were missed. This yields **cross-protocol
agreement** of recall 0.455, precision 0.769, and Spearman effect-rank
agreement 0.879. Two additional unmatched engine findings are M10 signals
(Phone/Queja and Phone/Retencion); the 12 missed catalog cells span complaints
on Web, technical across Email/App/WhatsApp/Web Chat/Web, commercial on
WhatsApp/Web Chat, and retention on Email/App/WhatsApp/Web Chat.

These values are not accuracy, lift, or independent validation: the sensor
and catalog use the same source snapshot with different customer splits and
statistical gates. The metric/cell protocol and `Web Chat` alias were committed
before successful v2 catalog generation (`952092a9`, `78f9fe08`, `a994ba24`).
The current protocol copy adds a post-result cohort-identity caveat, so the E0
initial-sample comparison remains exploratory until the audited member set is
verified; this caveat does not alter the registered metrics or catalog. The
sensor executable is a local stand-in binary (SHA-256
`dabbf84215f9210918edfa6f07e6c957ace976f9a2ca8a478622562f59f2b401`), not the
fully integrated production engine. Also, the catalog output had already been
generated and inspected while that cohort-identity caveat was not yet recorded;
the sensor score remains exploratory cross-protocol agreement, not independent
validation. All real aggregate and signal files remain outside Git.

## Local tests

```powershell
python -m unittest discover -s docs/data/opbench -p 'test_*.py' -v
cargo test --offline --manifest-path docs/data/opbench/e0-export/Cargo.toml
```

No raw or intermediate dataset artifacts belong in Git. The JSON catalog is
aggregate-only; inspect it with `validate_safe_pack` before commit.

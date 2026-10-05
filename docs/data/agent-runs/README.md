# Agent Core run-export aggregates (T3)

T3 now derives privacy-gated, aggregate-only rows from a fully traversed
Agent Core run export. Outcome rows use bounded `agent × locale × topic ×
outcome` dimensions; handoff and tool rows use the first three dimensions.
Topic is always the explicit `not_observed` sentinel, not an inferred label.
The metric set is one terminal-outcome rate with five outcome values, an
overall handoff rate, distinct-run tool-error and retry rates, and one latency
family marked non-computable. No raw
identifiers or free text are emitted. See [the aggregation contract](AGGREGATION_CONTRACT.md)
for exact denominators, cursor/event deduplication, and interpretation limits.

Cursor exhaustion is structural, producer-asserted evidence only. The
aggregator cannot authenticate complete source traversal or snapshot
provenance. Run/event rows must contain schema-valid agent and locale fields;
valid unknown agent IDs and bounded BCP 47-like locale tags are coarsened to
`other`, while missing or malformed values reject the export. Recognized
`es-*` and `pt-*` tags coarsen to their language labels. Terminal close reasons
are required; missing or unknown `closed_by` values reject the export. Missing
tool-attempt values default to attempt 1, as in the pinned source schema.

Outcome and overall handoff marginals are jointly suppressed unless every
internal outcome×handoff and outcome×non-handoff intersection has at least ten
runs. This prevents differencing either marginal to reveal a small joint cell;
the cross-tab is never emitted.

Agent-run metrics use the `AG_*` namespace. They are structurally compatible
with bank cells but are not bank metrics and are not registered by the Rust
M1-M10 sensor. The NDJSON mode writes exactly the shared six fields. A shared
structural reader accepts both sources without merging their populations or
metric registries.

Latency is not computable from the binary numerator/denominator row schema.
Topic is not present in the source and is never inferred from text. The
checked-in recorded fixture is incomplete (nonterminal cursors) and remains
rejected; current tests validate synthetic contracts, not a live Agent Core
1.4 export or bank behavior.

Run the focused tests:

```powershell
python -m unittest discover -s scripts/aggregate/tests/agent_runs -p 'test_*.py' -v
```

For a local fully paginated export:

```powershell
python scripts/aggregate/agent_runs/cli.py `
  --input <local-complete-export.json> `
  --output <outside-repository>/agent-run-aggregates.json

python scripts/aggregate/agent_runs/cli.py `
  --input <local-complete-export.json> `
  --output <outside-repository>/agent-run-cells.ndjson `
  --format ndjson
```

The JSON envelope carries bounded availability and evidence class. NDJSON
contains only exact six-field cell rows; an empty file means no cells survived
suppression, not zero incidence. Both commands refuse overwrite and outputs
inside the checkout. Never commit a real export or generated report.

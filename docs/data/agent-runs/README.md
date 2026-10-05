# Agent Core run-outcome aggregates (T3, partial)

This module creates privacy-gated, aggregate-only rows from a fully paginated
Agent Core export. Its checked-in recorded fixture is synthetic-input evidence,
not production or bank behavior. The current implementation measures only
terminal run outcomes. It does **not** localize an agent-quality problem by
agent, locale, topic, channel, handoff reason, tool, or latency.

## Current metric registry

For each month of `run_closed.ts` and each stable half assigned from the
private `run_id`, the eligible population is the set of unique, closed runs
whose run record and exactly one `run_closed` event agree on outcome and close
month. `open` runs are excluded. The five metrics below are mutually exclusive
and partition that eligible population:

| Metric | Numerator | Denominator | Export evidence used |
| --- | --- | --- | --- |
| `resolved` | Eligible runs with outcome `resolved` | All eligible runs in the month and hash half | Run outcome plus matching terminal event |
| `escalation_or_transfer` | Outcomes `escalated` or `transferred` | Same eligible-run denominator | Run outcome plus matching terminal event |
| `abstention_or_clarification_exhausted` | Outcomes `abstained` or `clarify_exhausted` | Same eligible-run denominator | Run outcome plus matching terminal event |
| `failed` | Outcome `failed` | Same eligible-run denominator | Run outcome plus matching terminal event |
| `other_terminal` | Outcomes `cancelled`, `completed`, or `abandoned` | Same eligible-run denominator | Run outcome plus matching terminal event |

The private split is `SHA-256(domain || run_id)`; the identifier is never
serialized. A cell vector is emitted only if every bucket has at least ten
observations, preventing subtraction of rare buckets from the total. The
each cell has the common six-field structure (`metric`, `dims`, `half`,
`period`, `numerator`, `denominator`) and can be serialized as canonical
NDJSON with `render_cell_ndjson`. The structural reader
`parse_cell_ndjson` accepts both bank and Agent Core cell rows and applies the
shared shape/count-floor checks. It does not unify their metric registries:
these outcome names and empty `dims` remain unregistered in the T1 bank
estimator. Do not merge these rows into a bank report or claim Agent Core
findings were scored by the bank sensor.

## What is not computable yet

Claude reports that Agent Core export contract 1.4.0 carries run-level
`agent{id,version}` and one locale per run, and defines event families such as
`tool_called`, `run_transferred`, `escalated`, `decision_made`, and
`turn_completed` (shared journal CL-0073). The exact 1.4.0 revision is not
present in the available local references, which both pin 1.3.0; therefore
this team-provided inventory is not independently verified against the wire
schema. It also does not mean those fields have been validated against a
complete export in this repo: the checked-in recorded fixture has only two synthetic-input
`pulso-builder` / `es` summaries, nonterminal cursors, and `run_closed` events.
The adapter therefore still does not publish agent or locale dimensions; a
privacy-reviewed finite agent-label mapping and a separate metric registry are
not finalized. It does not infer topic from text or IDs. Event-family names
alone do not establish denominators, duplicate/retry behavior, or when absence
means zero, so tool-error, fallback, confidence, step-count, and latency rates
remain unimplemented. The missing full export is not replaced with synthetic
event payloads or claims of live validation.

The T3 target—agent × locale × topic × outcome cells that the sensor can
consume—therefore remains **partial**. Before adding such metrics, define and
test: the supported export version and fields; safe closed vocabularies for
agent/locale/topic; a topic source that does not use free text; per-metric
denominators and missing-event behavior; k=10 plus complementary suppression
after all grouping; and an adapter from T3 metric IDs to a versioned sensor
registry. Do not silently map unknown or missing values into a real category.

## Evidence labels and reproducibility

Input `_label` is restricted to `RECORDED`, `SYNTHETIC`, or
`LIVE PLATFORM ...`; output keeps only the evidence class. `LIVE PLATFORM`
does not establish bank-customer representativeness. A recorded export must
include every run page and per-run event page through an observed terminal
empty page. A recorded partial page is rejected, not treated as the full
population. The checked-in recorded fixture has two synthetic-input run rows,
but its cursors are nonterminal, so the adapter rejects it before aggregation.
Separately, a complete two-run synthetic fixture verifies that the k=10 gate
publishes no metric cells.

The report is stable JSON and must be written outside the repository. No raw
run/event payload, identifier, free text, or output file from a real export is
committed.

```powershell
python -m unittest discover -s scripts/aggregate/tests/agent_runs -p 'test_*.py' -v
python scripts/aggregate/agent_runs/cli.py `
  --input <local-fully-paginated-export.json> `
  --output <outside-repository>/agent-run-outcomes.json
```

The first command validates the synthetic export contract and aggregate
privacy behavior. The second is only meaningful with a complete export whose
evidence class and provenance are known; its success is not evidence of
production representativeness.

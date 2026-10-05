# Agent-run outcome aggregation contract v2

**Status:** implemented initial contract; synthetic tests only. Claude's
shared-journal entry CL-0073 reports fields in Agent Core run-export contract
1.4.0, but the exact cited source revision is absent from available local
references (both declare contract 1.3.0); the v1.4 wire shape is not
independently verified here. The checked-in Codex fixture does not exercise
most reported fields or event types.
This is a separate Agent Core telemetry stream. It does not alter the bank
dataset, the six-field `bank_cells.py` schema, or the bank outcome estimator.
Matching the bank-cell row shape is structural compatibility only; bank and
agent-run metrics must never be pooled or compared as if they shared a
population.

## Source and evidence boundary

Input is a recorded Agent Core run-export envelope with `runs.pages` and
per-run `events[run_id]` page arrays. Only closed or escalated runs with a known terminal outcome
are eligible; open runs are excluded. Each eligible run must have one
`run_closed` event whose outcome agrees with the summary row and whose UTC
close timestamp is in the same calendar month. Agent Core may persist
`closed_at` and emit `run_closed.ts` a few milliseconds apart, so exact-instant
equality is not a valid integrity condition. Registry events are not read.
Missing, malformed, contradictory, or unknown terminal data fails closed
instead of becoming a measured zero. The saved envelope must contain the
actual cursor traversal for the run listing and for every run's event listing.
Each trace is an ordered `pages` array; every page records
`requested_after`, `items`, and `next_after`. The first request uses null; each
subsequent `requested_after` must exactly equal the prior page's `next_after`.
The traversal is complete only when an empty `items` page is observed, and
that empty page must be last. The cursor is opaque: its value is never decoded,
and even the terminal empty page may repeat a non-null cursor. A caller-supplied
`terminal_page_observed` boolean is not evidence of pagination completeness.

CL-0073 reports that the Agent Core 1.4.0 export contract includes run-level
`agent{id,version}` and one `locale` per run; event families include
`tool_called`, `run_transferred`, `escalated`, `run_closed`, `decision_made`,
and `turn_completed`. This is collaborator-reported contract evidence, not
independent verification of the v1.4 wire schema, proof that our recorded
sample exercises those events, or proof that their cardinality and outcome
semantics support a metric. The available local v1.3 producer source suggests
that tool records are per attempt (including retries), `decision_made` may
recur per run, and `turn_completed` excludes turns without a completion event;
do not treat those v1.3 observations as v1.4 guarantees until the cited source
revision is pinned and audited.
The checked-in `scripts/triggers/fixtures/export_recorded.json` is explicitly
recorded from local Agent Core using synthetic input; it contains two returned
`pulso-builder` / `es` completed-run summaries and trimmed `run_closed` events,
but it does not attest that the run and event pages are terminal. The
aggregator rejects it as incomplete; its two returned rows cannot establish the
total population or support any publishable cell or claim about bank/customer
behavior.
Input labels are mapped to a fixed evidence enum (`recorded`, `synthetic`, or
`live_platform`) and raw labels are never copied to output. The `live_platform`
label records source class only; it does not certify production representativeness.

## Frozen terminal metric registry

Metrics are mutually exclusive buckets over the pinned nine-value Agent Core
terminal outcome enum. Every valid outcome maps exactly once:

| Metric | Included terminal outcomes | Interpretation boundary |
| --- | --- | --- |
| `resolved` | `resolved` | Only the explicit terminal outcome; never infer from `completed` |
| `escalation_or_transfer` | `escalated`, `transferred` | Handoff category; not necessarily a failure |
| `abstention_or_clarification_exhausted` | `abstained`, `clarify_exhausted` | Agent did not complete resolution; the grouping is a reporting bucket |
| `failed` | `failed` | Explicit failed terminal outcome |
| `other_terminal` | `cancelled`, `completed`, `abandoned` | Mixed/ambiguous remainder; do not call success or failure |

The mapping is versioned and closed. Unknown values reject the export; new
Agent Core outcomes require a contract revision and tests before inclusion.

## Population, split, and row contract

- Grain: one unique `run_id`. Exact duplicates over the output-relevant
  terminal fields collapse to one run; conflicting outcome or close-instant
  duplicates reject the export. IDs are held only in process.
- Outcome time: `closed_at` and the sole `run_closed.ts` are parsed as
  timezone-aware UTC instants and must fall in the same UTC calendar month;
  period is the summary time's UTC month (`YYYY-MM`). Naive timestamps are
  invalid.
- Population: all eligible closed runs with one recognized terminal outcome in
  that month and split. The numerator is the count for one frozen metric; the
  denominator is all eligible runs in the same month and split.
- Split: deterministic SHA-256 assignment of the private run ID using a
  protocol-specific domain separator. `discovery` and `holdout` are independent
  reporting halves only—not treatment/control groups and not causal evidence.
- Dimensions are intentionally empty in v1. Agent, release, principal, locale,
  session, run, event, and registry identifiers or attributes are not emitted.
  Although contract 1.4 exposes agent/version and locale, output needs a
  reviewed finite agent-label mapping and a separate Agent Core metric registry
  before these dimensions can be published. Do not emit raw artifact IDs or
  silently collapse unknown values.
- Each disclosed cell has exactly `metric`, `dims`, `half`, `period`,
  `numerator`, and `denominator`. A separate report envelope identifies the
  `agent-run-outcomes.v2` protocol and evidence class. `cell_table.py` can
  render these rows as canonical NDJSON and parses bank-producer NDJSON through
  the same six-field structural reader. Source-specific metric registries stay
  separate; this does not make the Agent Core rows acceptable to the current
  T1 estimator.

## Disclosure rule

The privacy floor is fixed at `k=10` in v1 and cannot be lowered by a caller.
For each month/split, the five metrics partition the eligible population.
Because disclosing four categories could reveal the fifth, v1 releases either
the complete five-cell vector or none of it. The vector is publishable only if
every outcome bucket has at least 10 runs; this also makes each metric's
complement at least 10. Otherwise the entire vector is suppressed without
including small counts, run IDs, or row-level examples. Future dimensions or
more granular metrics require complementary-suppression analysis before they
can be added.

The incomplete recorded fixture therefore fails closed without producing an
output file. A separate complete synthetic export verifies that a fully read
but sub-k population produces no cells. Reports expose only `none`, `partial`,
or `all` as a bounded suppression status, never suppressed period/split keys
or counts. Neither behavior is a no-outcome or zero-rate finding.

## Output, persistence, and tests

The CLI is offline/read-only, writes only to an explicitly supplied destination
outside the Git checkout, and refuses to overwrite an existing versioned
output. Serialized output contains aggregate rows and bounded status codes
only; it excludes source labels, raw IDs, free text, event payloads, and registry
metadata. Tests use synthetic exports plus the recorded two-run fixture. They
cover every outcome mapping, UTC/month attribution, deterministic split,
deduplication/conflicting duplicates, missing or mismatched terminal events,
cursor-chain validation and an observed empty terminal page, fixed privacy
floor, partial suppression status,
privacy-floor boundaries and vector suppression, output schema, ID redaction,
and deterministic no-overwrite file output.

The aggregator verifies only the traversal evidence presented in the envelope;
it cannot authenticate that an arbitrary JSON file was produced by a trusted
collector or that the collector actually exhausted the source endpoint. Until
a trusted export collector is integrated and tested, page completeness is a
producer responsibility and recorded/synthetic input must not be described as
verified live extraction. The current five terminal buckets with empty
dimensions are a **partial T3 implementation**: they do not localize results
by agent, locale, or topic, nor measure handoff, fallback, or tool-error rates.
CL-0073 reports run-level agent/version and locale fields, but that v1.4
contract source is not available locally and no
privacy-reviewed finite category mapping/registry is agreed; the recorded
fixture lacks complete run pagination and contains only `run_closed` events.
Event names in a contract are not enough to define event-derived denominators,
deduplication, retry handling, or absence semantics. Topic is not in this export
contract and must not be inferred from free text. Keep future Agent Core
metrics in a separate versioned registry and source population; do not expand
the bank M1-M10 registry or reuse bank customer splits as a run-source
assumption. The output is an aggregate JSON envelope (`cells` array), not
bank-cell NDJSON, and a shared reader/integration path remains unimplemented;
identical row shape alone is not reader compatibility.

No claim is made that the aggregate measures response quality, customer
satisfaction, productivity, causal lift, or production behavior. In particular,
the current export fixture lacks turn/tool/decision events and has one observed
outcome category; it cannot support latency, tool-success, or agent-comparison
metrics.

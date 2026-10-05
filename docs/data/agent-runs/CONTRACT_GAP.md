# T3 remaining evidence and gaps

**Implementation status:** the synthetic aggregator now implements the
CL-0075 run/event semantics supported by its contract note: finite
registry-label mapping, locale coarsening, explicit not-observed topic,
highest-cursor run selection, `(run_id,seq)` event deduplication, complete
event-page denominators, distinct-run tool errors, retries, and six-field
NDJSON output. It also computes a distinct-terminal-run handoff metric as the
run-level OR of `run_transferred`, `status == escalated`, or
`run_closed.closed_by in {escalation, transfer}`. The numerator is deduplicated
by run; its denominator is all distinct terminal runs in the same safe group,
not the handoff-only cohort or an outcome-conditioned bucket. This closes a
structural implementation gap; it does not prove live platform integration.

## Verified locally

- Outcome and handoff marginals are released only after joint internal
  outcome×handoff complementary suppression at `k=10`; the cross-tab is not
  serialized. Schema-invalid/missing agent or locale rejects the export;
  valid unknown values are safely coarsened to `other`.
- A terminal empty cursor page proves structural exhaustion in the supplied
  envelope only. Completeness and snapshot provenance remain producer-asserted
  and are not authenticated by the aggregator.
- `run_closed.closed_by` is required and restricted to the pinned producer
  enum; locale accepts bounded language-tag forms such as `es-MX` and `pt-BR`
  while coarsening them to `es`/`pt`; absent `tool_called.attempt` defaults to
  one. Tests cover malformed and missing values plus the optional attempt.

- The pinned local Agent Core 1.3 schema for `ToolCalledPayload` enumerates
  `ok`, `error`, `timeout`, `denied`, `uncertain`, and `step_up_required`;
  CL-0075 confirms the 1.4 semantics used for metric classification. Count
  `error|timeout|denied` as error-bearing runs; `uncertain` and
  `step_up_required` are not errors.
- The recorded fixture `scripts/triggers/fixtures/export_recorded.json` has
  only two synthetic-input run rows, no `seq` fields in its trimmed events,
  and nonterminal run/event cursors. It is rejected rather than silently
  treated as a full population.
- The exact six-field NDJSON parser is shared by bank and Agent Core rows.
  Metric names remain source-specific (`M*` versus `AG_*`).

## Still not established

1. No complete live export from the 1.4 runtime has been exercised against
   this adapter. The synthetic fixtures cannot establish the exact endpoint
   pagination behavior or producer consistency guarantees.
2. Agent labels are currently a small code-owned allow-list
   (`pulso-builder` -> `builder`); all other IDs become `other`. Expand only
   after review of the actual registry population and stable non-sensitive
   aliases.
3. There is no topic field or alias in the source. `topic=not_observed` is a
   protocol-owned constant for explicit absence and must not be presented as
   topic analysis.
4. Per-event latency values are present in reported contract fields, but the
   shared consumer schema cannot represent their distribution without
   inventing a statistic or threshold; latency is explicitly non-computable.
5. Tool metrics for open runs are snapshot observations, because a later run
   update may append events. They are not final outcome metrics.

The output must not claim production behavior, customer outcomes, causal lift,
or performance improvement. Keep agent-run aggregates out of M1-M10 and never
use a missing tool event as a successful result.

# Agent Core run aggregates (T3)

**Status:** v3 implementation for finite agent/locale dimensions, terminal
outcomes, tool-error runs, and retry runs. Synthetic contract tests pass; the
checked-in recorded fixture remains incomplete and cannot support a live
result. This is a separate Agent Core metric namespace (`AG_*`), never pooled
with or registered as bank metrics M1-M10.

## Source traversal and snapshot rules

The input is a saved Agent Core export envelope with a structurally complete
cursor chain for `runs.pages` and one complete `events[run_id]` cursor chain for
every selected run. The chain ending at an observed empty page is structural,
producer-asserted evidence only: this aggregator cannot authenticate that the
producer traversed every page, that the snapshot is globally consistent, or
that the envelope is genuine. The cursor is opaque. Missing, malformed, or
structurally incomplete event pages fail closed, because the denominator for
tool metrics must be known before a run can count as a non-error.

Run listing entries can reappear as the last-change cursor advances. They are
deduplicated by `run_id`, retaining the row with the numerically highest
`cursor`; conflicting rows at that same highest cursor reject the export. Event
rows are unique by `(run_id, seq)`. Exact duplicates collapse; conflicting
duplicates reject the export. IDs remain in process only and are never written
to the report.

The checked-in `export_recorded.json` is a two-run synthetic-input capture
whose run and event cursors are not terminal. It is expected to be rejected.
Collaborator contract evidence CL-0075 reports run-level `agent.id`, `locale`,
and event fields used below for contract 1.4.0. The pinned local generated
schemas are older; therefore the implementation's synthetic fixtures verify
the stated semantics, not a live v1.4 wire integration. An actual live export
with complete pagination is still required to establish runtime compatibility.

## Safe dimensions

Every row carries bounded `agent`, `locale`, and `topic` dimensions. Outcome
distribution rows add one bounded `outcome` dimension, giving the requested
agent × locale × topic × outcome shape. Handoff and tool-rate rows use the
three base dimensions; shared six-field rows do not require identical dims
across distinct metrics.

| Dimension | Rule | Meaning |
| --- | --- | --- |
| `agent` | Required schema-valid ID/version; fixed registry-ID allow-list: `pulso-builder` -> `builder`; valid unknown IDs -> `other` | Safe category only; raw registry ID/version never leaves the process. Missing or schema-invalid references reject the export. |
| `locale` | Required bounded BCP 47-like tag: lowercase 2–3 letter language plus optional hyphen-separated 2–8 ASCII alphanumeric subtags, max 35 chars. `es`/`es-*` -> `es`; `pt`/`pt-*` -> `pt`; other valid tags -> `other`. | One locale per run; missing, wrong-type, unsafe or malformed values reject the export. |
| `topic` | Constant protocol-owned `not_observed` | The source contract has no topic field. This sentinel is not source-derived and must never be presented as an observed topic. |

The finite agent allow-list is code-versioned. Adding an ID requires review and
a test. No text, release, principal, session, turn, tool, or other identifier
is used as a dimension. The topic sentinel exists only to preserve the requested
dimension slot while making absence explicit; it does not create topic-level
measurement.

## Metric registry and denominators

The common output row is `{metric,dims,half,period,numerator,denominator}`.
`half` is a deterministic SHA-256 assignment of the private run ID to
`discovery` or `holdout`; this is a reporting partition, not an experiment or
causal comparison. All rows are binary run-level proportions (one run
contributes at most once to a numerator).

| Metric IDs | Numerator | Denominator | Period |
| --- | --- | --- | --- |
| `AG_RUN_OUTCOME_RATE` | Unique terminal runs matching the row's `dims.outcome` value (`resolved`, `escalation_or_transfer`, `abstention_or_clarification_exhausted`, `failed`, or `other_terminal`) | All unique non-open runs with exactly one matching `run_closed` event in the same safe-dimension group | UTC month of `closed_at` |
| `AG_RUN_HANDOFF_RATE` | Distinct terminal runs where `run_transferred` is present **or** `run.status == escalated` **or** `run_closed.payload.closed_by` is `escalation|transfer`; the branches form a run-level OR, not an event sum | All distinct non-open terminal runs with exactly one matching `run_closed` event in the same agent/locale/topic group | UTC month of `closed_at` |
| `AG_TOOL_ERROR_RUN_RATE` | Distinct runs with at least one `tool_called.status` in `{error, timeout, denied}` | Distinct runs with at least one `tool_called` on a complete event page | UTC month of `created_at` |
| `AG_TOOL_RETRY_RUN_RATE` | Distinct runs with at least one `tool_called.attempt > 1` | Same tool-using-run denominator | UTC month of `created_at` |
| Latency family (`tool_called.latency_ms`, `decision_made.latency_ms`, `turn_completed.duration_ms`) | **Non-computable:** these values use attempt/decision/turn grains, but the row contract has no defensible latency statistic, distribution, or unit field | — | — |

The closed outcome values map through the frozen five-bucket partition:
`resolved`; `escalated|transferred`;
`abstained|clarify_exhausted`; `failed`; and
`cancelled|completed|abandoned`. `completed` is not inferred to mean resolved.
Terminal run rows must agree with exactly one `run_closed` event on outcome and
UTC month. Open runs do not enter outcome or handoff metrics, but may enter tool metrics
when a complete event page contains a tool call; tool metrics are therefore
snapshot observations, not final-run outcomes.

`run_closed.payload.closed_by` is required and must be one of the pinned
producer values `flow`, `abandonment`, `escalation`, `revocation`, or `transfer`.
Missing, empty, or unknown close reasons reject the export.

Tool error semantics follow CL-0075: `error`, `timeout`, and `denied` are
counted as an error-bearing run; `uncertain` and `step_up_required` are not.
Other statuses reject the export. `tool_called.attempt` is optional in the
pinned source schema and defaults to 1 when absent; retries are detected from
attempt numbers, not event count; duplicate `(run_id,seq)` records cannot
inflate the numerator.
Absence of `tool_called` is never interpreted as success and never enters the
tool denominator.

Although the export exposes `tool_called.latency_ms`,
`decision_made.latency_ms`, and `turn_completed.duration_ms`, these are
measurements over different event grains. The six-field consumer contract only
represents binary counts and has no defensible unit/summary-statistic field;
therefore all latency metrics are explicitly **non-computable** here. No
latency is averaged, summed, or encoded as a made-up binary threshold. Topic is
also unavailable, not computable from these exports.

## Suppression and output

The privacy floor is fixed at `k=10`. Each published binary cell must have at
least 10 positive and 10 negative contributing runs. Outcome and handoff are
suppressed jointly for one period/split/base-dimension group: an internal
five-outcome × handoff/non-handoff table is checked, and every cell (including
each binary complement) must meet k before either the outcome marginals or
overall handoff marginal is released. The cross-tab itself is never emitted.
This prevents differencing one separately published marginal against the
other to infer a sub-k intersection. Handoff is an overall terminal-run rate
by agent/locale/topic; its three source signals are unioned per run before
counting. Tool metrics are individually gated by
both sides of their tool-user denominator, and are additionally withheld when
the complementary no-tool-run population in the same period/split/base-dimension
group has fewer than k runs. This prevents subtraction of the published tool
denominator from the all-run population from revealing a rare no-tool group.
Suppressed keys and counts are not emitted.

The JSON envelope includes protocol, evidence class, bounded availability and
suppression status, and aggregate `cells`; it includes no source labels, IDs,
free text, event payloads, or registry data. `--format ndjson` serializes the
same cells using the exact six-field bank-cell row shape. Both bank producer
NDJSON and Agent Core NDJSON are parsed by the same structural
`parse_cell_ndjson` reader, but their `M*` and `AG_*` metric registries remain
separate; structural compatibility does not register `AG_*` with Rust M1-M10.
Writes are immutable and must target a location outside the repository.

## Evidence boundary

Synthetic tests cover mappings, safe dimensions, cursor-highest run
deduplication, `(run_id,seq)` event deduplication, complete-page requirements,
denominator construction, retry/error classification, deterministic output,
k=10 and complementary suppression, exact NDJSON row shape, and use of one
reader for both sources. They do not prove a live 1.4 export run, bank/customer
behavior, quality, satisfaction, causal lift, or production representativeness.

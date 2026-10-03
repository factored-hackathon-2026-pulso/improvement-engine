# E0 proposal assembly boundary

Status: implemented in the core API and wired into E0 `local-sim` result
persistence. It is not an Agent Core artifact or a complete end-to-end
proposal/evaluation flow.

## Purpose

Convert an existing E0 `LocalRunResult` and its `LocalSimulationPortfolio`
into a deterministic set of descriptive proposal seeds. Every signal admitted
by the portfolio becomes one candidate. The assembler does not rerun
detection, choose a new primary signal, execute a scenario, or estimate
business lift.

## Input and validation

`assemble_e0_proposals` consumes only the typed local simulation result. It
fails closed when the portfolio's three-metric coverage is incomplete or
internally inconsistent, when measured digests do not match, when dispositions
are duplicated or unknown, when reason text is outside the finite code set,
or when policy identifiers/versions are invalid. E0's recurring-query metric
must be either measured or explicitly unavailable, and an unavailable
disposition has no signal digest and must agree with the run's
`recurrence_measurement_status`.

Each `SignalSummary.summary_commitment` hashes the projection fields and is
recomputed before use, detecting accidental/stale edits such as changed counts
with an old commitment. It is an unkeyed hash, not a signature or authenticity
boundary: a caller able to construct the public structs can fabricate a full
summary and recompute it. The result is trusted only as an in-process local
simulator projection and must never be treated as authenticated U08/U13
evidence or a native Agent Core candidate. Count validation also rejects
overflow and recomputes coverage as `floor(denominator * 10,000 /
(denominator + missing))` (zero when total support is zero).

Before assembly, the run envelope must be a complete deterministic local E0
simulation (`complete_simulated` or `complete_no_opportunity`) with the
runner-generated `run_<pid>_<nanos>` ID, source-adapter UUIDv7 snapshot ID,
SHA-256 digests and a strict UTC whole-second cutoff. Tenant ID must match
between run and snapshot and obey the source adapter's non-empty/128-byte
contract, but is intentionally omitted from the serialized proposal seed.
Candidate states are independently
recomputed from numerator, denominator, missing count and recurring support
floor; matching a stored digest alone cannot turn a zero or unsupported
measurement into a candidate.

An empty portfolio remains `insufficient_evidence`; it is never filled with
synthetic zero observations. A measured portfolio with no qualifying
candidate is `no_qualifying_signals` only when there is no unavailable or
insufficient disposition. Inconsistent portfolio-level status is rejected.

## Output semantics

The result includes source run/snapshot/cutoff provenance, the primary signal
reference, sorted per-metric dispositions, and sorted candidate seeds. Each
candidate refers to its source signal digest and detector policy. It contains
no duplicate metric values, no customer-level content, no inferred route, and
no lift. Its fixed claims are `descriptive_only`, `unlinked`,
`not_evaluated`, `business_lift=null`, and `native_agent_core_status=not_connected`.
This output is persisted under `proposal_assembly` in E0 `result.json` as a
proposal input for later composition, not proof that an improvement works or a
serialized Agent Core artifact. Persistence is E0-only; `original_bank` keeps
its source-specific result and does not receive a local E0 portfolio or
proposal assembly.

`original_bank` and any non-simulator authority return an `unsupported`
envelope with no candidates and redacted placeholder provenance; unsupported
inputs are not echoed into serialized output. This module does not alter the original source
database. The runner adds the E0-only `proposal_assembly` field to the persisted
result without changing the typed `LocalRunResult` schema. It also appends one
sanitized `RunEvent` with stage `proposal_assembly` to both persisted timeline
views. Its status mirrors the assembly disposition; detail contains only
aggregate candidate and disposition counts, and its cutoff is the run cutoff.
It follows any post-selection holdout event using a checked contiguous
sequence. OriginalBank runs do not receive this E0 event. This is internal
local-run observability, not an Agent Core `EngineEvent`, customer-attention
event, execution receipt, or claim of source-level activity.

For a qualifying recurring-query candidate, the local runner also creates an
`e0_mechanism_resolution` envelope from the P3 candidate-bound evidence packet
and exact route resolver. The packet's origin is recorded as `e0_local_run`;
the catalog is separately labeled
`team_generated_empty_local_catalog_fixture` with `catalog_durability=ephemeral`.
This distinction matters: the run evidence comes from the selected E0 local
run, while the empty catalog is a team-generated fixture, not a persisted
Agent Core registry snapshot. It always resolves `unlinked` with
`no_exact_supported_flow_mapping`; it does not invent or imply a Core Flow.
The serialized tagged `resolution` keeps its `unlinked`/`mapped` shape and
includes the exact packet `metric_id` and opaque `pattern_ref` used for lookup.
In memory, `RouteResolution` is a resolver-minted, non-constructible,
non-deserializable receipt bound to the full evidence packet. The
investigation-plan composer rejects a receipt whose private packet binding or
visible lookup key differs from its candidate-bound packet, so a declared
mapping for one pattern cannot be rebound to another candidate. The
runner-only origin and durability labels sit outside that union. No eligible
recurring-query candidate means no mechanism packet or event. OriginalBank
receives neither. A route resolution/event is local descriptive observability
only and proves no Core artifact existence, execution, evaluation, or business
lift.

For a qualifying recurring-query candidate with the exact mechanism packet and
route resolution, the runner may also persist an
`e0_investigation_proposal_plan`. This typed envelope carries the candidate's
proposal reference, source run/snapshot/cutoff, signal and summary commitments,
the exact candidate-bound evidence packet, and exact catalog resolution. Its
`artifact_kind` is
`e0_read_only_investigation_plan_not_agent_core_proposal`; it is not a Core
Proposal, Agent, Flow, Jev, or executable artifact. An unlinked candidate
offers `investigate_mapping` and `do_nothing`; an exact mapped fixture offers
`investigate_mapped_flow` and `do_nothing`, with the mapped option explicitly
read-only. Both plans are `pending_review`, have no authority, are not
executable, and keep `claim_level=descriptive_only` and `business_lift=null`.
The deterministic SHA-256 plan digest covers schema, candidate/evidence
lineage, catalog resolution, and decision options; integrity validation rejects
content drift or inconsistent duplicate bindings. Tenant scope and source
query values remain excluded. No recurring candidate means no plan or plan
event; OriginalBank never receives either. The plan does not compile, invoke,
evaluate, publish, or release a mapped artifact and is not evidence of lift.

When present, one `e0_investigation_proposal_plan` RunEvent follows the
mechanism-resolution event in both result timeline views. It records only the
pending-review state, allowlisted recommendation and non-executable boundary.

When present, one `e0_mechanism_resolution` RunEvent follows the optional
holdout and `proposal_assembly` events, with contiguous checked sequence and
identical JSON/NDJSON serialization. Its detail names the catalog's generated
and ephemeral provenance plus aggregate resolution counts/reason only; it
contains no tenant, candidate or signal identifiers, digest, query signature,
or source-row data.
The investigation-plan event then follows mechanism resolution with the same
checked sequence and timeline-parity guarantees.

## Validation boundary

Core unit tests cover candidate fan-out, no-op/insufficient/empty/unsupported
states, stable permutation ordering, duplicate and mismatched provenance,
unavailable recurrence, disposition coverage, finite reason codes, policy
code validation, threshold/state recomputation and invalid run envelopes.
Public CLI integration tests verify E0 fan-out and source isolation in
persisted results. Full local CI and real-data E2E are not yet claimed.

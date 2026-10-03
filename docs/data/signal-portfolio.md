# Signal portfolio

## Purpose

The signal portfolio preserves each independently measured signal that meets
the initial descriptive eligibility rule. It is a pure Rust boundary that can
be composed by discovery flows; this slice does not connect it to the local
runner or Agent Core.

## Input and evidence boundary

Measured inputs are opaque E0DiagnosticSignal values returned by
E0DiagnosticSensor after U04/U08 replay and U12 query evidence have been
verified. The portfolio accepts no caller-supplied counts, raw rows, or
deserialized signal payloads.

Unsupported and Unknown are explicit non-measurement outcomes. They carry
metric and policy identity, a reason, and scope derived from an authenticated
E0 query receipt—not from a successful metric measurement. The scope includes
tenant, run, grant, authority, source snapshot reference/binding, and cutoff.
This lets a run represent the case where every configured metric is
unavailable while preventing an outcome from another run or tenant being
mixed into a portfolio. They deliberately have no count or rate fields and
cannot create candidates. A measured signal that has no
known denominator or no positive known observation remains visible as
NotQualified with its observed counts and a reason; it is not silently
discarded or re-labelled as missing.

## Eligibility and provenance

Qualification policy positive_observed_cases version 1 is intentionally
descriptive: numerator must be positive and denominator must be nonzero. This
does not establish causality, customer harm, commercial value, feasibility, or
that an Agent Core artifact can change the outcome.

Each candidate carries its metric/policy version and commitment, measured
counts, missingness, coverage, observation window, cutoff, run/tenant/grant/
authority scope, source snapshot reference and binding, source/table/transform
digests, replay and evidence digests, and query receipt digests. Its stable ID
is derived from the signal digest. Candidates are ordered by metric, window,
then signal digest.

Signals in one portfolio must share tenant, run, grant, authority, source
snapshot reference and binding, and cutoff. Mixed scope fails closed. Repeated
signal digests are rejected rather than counted twice.

Digest inputs must use the canonical `sha256:` prefix plus 64 lowercase
hexadecimal characters. The sensor emits digests in this canonical form; the
portfolio rejects alternate uppercase spellings instead of treating them as a
second textual identity for the same digest.

## Output and limitations

The result is either `candidates_ready`, `no_qualifying_signals`, or
`insufficient_evidence`, each with a disposition reason. The latter means
there was no successful measurement to assess, or missing metric coverage
remains alongside measured non-qualifying results, including a measured signal
with a zero known denominator; it must not be interpreted as a negative
finding or evidence that no issue exists. `no_qualifying_signals` is reserved
for a non-empty portfolio where every outcome was measured with a nonzero
known denominator and no signal met the descriptive eligibility rule. This
status is still bounded to this metric policy, not proof that the bank has no
issue. Empty input has top-level status `insufficient_evidence` and the
distinct reason `no_signals_provided`; an absent source is not a measured
negative. If any
candidate qualifies, status remains `candidates_ready` while the observation
list still exposes other missing metrics for downstream coverage review. The
full observation list retains candidate, non-qualifying, unsupported, and
unknown states. It performs no source access, model call, Agent Core
operation, persistence, or mutation.

Serialization is currently an internal projection for tests and inspection,
not a durable API or persistence contract.

The authenticated `SignalPortfolio` consumes the E0 diagnostic signal
contract, currently backed by the reviewed technical-error metric policy; it
does not consume the local simulator's retry or recurring-query summaries.
Separately, E0 local-simulation results now expose a `LocalSimulationPortfolio`
over their own retry, technical-error, and recurring-query `SignalSummary`
values. That simulator-only projection is not authenticated evidence and does
not change this contract. Neither portfolio includes original-bank snapshots
or platform telemetry; those families require their own measured,
provenance-bound adapters.

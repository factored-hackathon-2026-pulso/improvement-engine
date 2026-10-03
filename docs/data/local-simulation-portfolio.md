# E0 local-simulation portfolio

## Purpose and authority

`LocalSimulationPortfolio` is a simulator-only summary of the E0 local runner's
existing `SignalSummary` values. It preserves each measured signal's disposition
and links back to its existing digest. It is a separate type from the
authenticated `SignalPortfolio`: it does not contain U08-authenticated evidence
and cannot be used for U13/U14 eligibility, durable evidence, production
authority, or release decisions.

The portfolio is attached only to E0 local-simulation results. OriginalBank
snapshot results retain their existing behavior and serialize no E0 portfolio.
The runner's existing primary-signal selector, recurrence support floor,
proposal construction, and post-selection holdout behavior are unchanged. The
portfolio does not select or elevate a signal; at most, its candidate digest
references identify measured signals that may be investigated by the simulator.
The current proposal remains built from the existing primary signal only.

## Data shape and interpretation

The output contains `source_family=e0`, fixed `authority=simulator_only`, an
aggregate status, an ordered `dispositions` list, candidate digest references,
and the primary signal digest. Each disposition contains only a metric ID,
optional reference to that signal's digest, a state, and a reason. Numerators,
denominators, rates, and missing counts remain solely in the existing `signals[]`
field; the portfolio does not copy them.

| Disposition | Meaning |
| --- | --- |
| `candidate_for_simulated_investigation` | Existing local threshold passed with a known, non-empty denominator. This is descriptive eligibility only. |
| `not_qualified` | A measured signal did not meet the local threshold; it is not a business conclusion. |
| `insufficient_evidence` | The metric has no known denominator or has missing observations that prevent a negative conclusion. |
| `unavailable` | The source table needed for a metric was unavailable; no digest or zero is fabricated. |

Portfolio status is `candidates_ready` when at least one measured signal meets
its local threshold, `no_qualifying_signals` only when the available
measurements are complete and none qualify, and `insufficient_evidence` when
there are no measured signals or no candidate and any signal is unknown,
unavailable, or has an empty denominator. A qualifying signal can coexist with
other unavailable metrics; those remain explicit in the per-signal list.

An E0 query-table absence adds an `unavailable` recurrence disposition with no
signal digest. Existing measured signals retain their runner-defined order; the
recurrence-unavailable entry follows them. Candidate digest references follow
that same order. The primary digest points to the existing selected primary and
is not itself an eligibility claim.

## Timeline and persistence boundary

The `signal_portfolio` timeline event uses the fixed `simulator_only` status and
contains aggregate disposition counts only. It does not repeat metric values,
digests, query signatures, tenant/customer identifiers, or raw source rows.
Run-result serialization is for this local simulator's inspection/output path;
it does not establish a durable API or persistence contract. The source adapters
and source databases are unchanged; signal measurement continues through the
existing disposable local investigation path.

## Validation

Integration tests cover multi-signal preservation, candidate digest references,
primary-selector parity, known-zero versus unknown denominator, missing query
table, deterministic disposition ordering, repeatable ordered metric IDs,
sanitized timeline detail, and absence of an E0 portfolio on OriginalBank
results. The existing proposal and holdout paths remain unchanged and continue
to make no causal or business-lift claim.

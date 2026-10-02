# Original-source snapshot-descriptive projection

## Outcome

Added a typed, local-core projection for monthly descriptive aggregates from
the original `call_center_interactions` and `complaints` partitions. It is
separate from the existing UTC as-of projector and from the runner's existing
snapshot contact-volume count. It groups by the literal month in exact naive
source wall-clock text; it never converts timezone, compares event dates with
the snapshot's UTC `observed_cutoff`, or exposes that cutoff in its output.

The descriptive result has its own temporal/value-semantics tags and a
domain-separated manifest digest. Complaint output includes only the
source-provided final SLA flag, resolution duration, and satisfaction. It omits
derived elapsed first-response time because no shared-clock contract is
available. No identities, free text, or source values leave the streamed
aggregation.

## Verification

- Test-first: confirmed the new contract test failed because the typed
  descriptive API did not exist; implemented the contact and complaint paths.
- Adversarial digest check: confirmed UTC and descriptive outputs initially
  shared one manifest digest; domain-separated the descriptive digest and
  verified it differs.
- `cargo +1.98.1 test --locked --offline -p improvement-engine-core --test original_contact_projection`:
  22 passed, 1 ignored.
- Bounded local source smoke (first 25 stable-sorted partitions per table,
  `coverage=partial`): both descriptive projections supported valid rows,
  rejected no naive timestamps, applied k=5 suppression, and emitted aggregate
  counts only. The parallel legacy UTC projectors remained unsupported and
  emitted no cells. No source rows, identifiers, values, or paths were printed.
- `cargo +1.98.1 fmt --all -- --check`: passed.
- `cargo +1.98.1 clippy --locked --offline --workspace --all-targets -- -D warnings`:
  passed.
- `git diff --check`: passed.

## Limits and follow-up

## Adversarial privacy/status correction

An independent review found that public `ProjectionManifest` construction
accepted k=1 even though the documented control was k=5. The constructor now
fails closed outside 5..=10,000, and a regression test exercises k=0, k=1,
and k=4; test-only small-k fixture arguments are raised to the enforced floor.
The same review found that valid timestamps with every channel blank could
produce `Supported` with zero cells. Descriptive outputs now require at least
one row with a valid literal month and usable channel; otherwise they return
`Unsupported` with `usable grouping rows`. Rejected rows may coexist with
supported usable rows.

- RED: `public_projection_manifest_rejects_k_below_privacy_floor` failed on
  public k=1 before the constructor change.
- GREEN: `cargo +1.98.1 test --locked --offline -p improvement-engine-core --test original_contact_projection` — 24 passed, 0 failed, 1 ignored.
- GREEN: bounded local smoke over 25 partitions per source table — contacts:
  16,277 descriptive rows, 48 visible cells, 2 suppressed; complaints: 1,452
  descriptive rows, 19 visible cells, 1 suppressed. UTC projectors rejected
  naive timestamps as before and emitted no cells.
- GREEN: `cargo +1.98.1 fmt --all -- --check`,
  `cargo +1.98.1 clippy --locked --offline --workspace --all-targets -- -D warnings`,
  and `git diff --check`.

The 25-partition local sample is not a full-history estimate; complete coverage
requires the caller to enumerate the entire sealed inventory. A source month
means only the month text present in the source, not a comparable UTC period,
bank-observed time, or causal cohort. Final-extract outcomes remain unsuitable
for point-in-time or online decisions.

The runner consumes the existing snapshot contact-volume projection today; it
does not yet consume these monthly descriptive outputs. The smallest next seam
is a discovery-input adapter that preserves the snapshot binding, coverage,
and descriptive-only tag while mapping safe normalized contact cells into the
existing volume signal path. Complaint aggregates need a separate evidence
contract if they are to support a candidate. This slice intentionally does
not modify runner/P2 discovery, proposal, or evaluation behavior.

# 0054 — Original snapshot descriptive opportunity envelope

Date: 2026-10-02

## Decision

Keep the original-bank contact projection separate from the UTC as-of
projection. Source timestamps are naive and must not be assigned a timezone,
compared with the run cutoff, or used as eligibility evidence. The source's
literal calendar month is descriptive grouping only; source values represent
final-extract facts and coverage remains `partial`.

When a complaint month × reason × channel cell reaches the configured `k`
floor, the local runner may emit a snapshot descriptive finding and an
unverified proposal envelope. Both remain bound to the snapshot digest,
source-manifest digest, and projection digest. This envelope is not a U13 or
Agent Core candidate: it has
`agent_core_candidate=dependency_blocked_snapshot_semantics`, proposal status
`simulated_unverified`, execution `not_executed`, publication disabled, and
formal route `do_nothing`. It does not fabricate query receipts, candidate
admission, compiled artifacts, release eligibility, causality, or ROI.

## Agent Core boundary

Read-only contract review against Agent Core commit
`53e729d624c8284e906249df84c1a1df84cc8d40` found no offline/snapshot discovery
candidate contract. `SnapshotRegistry` is a release-time in-memory registry,
not the bank's dataset snapshot. The `auto_detect` proposal origin does not
remove the governed U13 candidate/query-receipt boundary. A future output
adapter therefore requires an Agent Core contract extension; the local
descriptive envelope must not be called an Agent Core artifact.

## Validation record

- Adapter RED: new test showed missing-channel rows were excluded from the
  descriptive rejection count (`0`, expected `1`). GREEN after accounting for
  unusable grouping rows. Such rows do not enter any k cell or visible-count
  denominator.
- Core RED: test failed to compile because the typed projection, builder, and
  output envelope did not exist. GREEN after adding the separate
  `LocalSnapshotContactProjection` boundary and non-publishable envelope.
- CLI RED: removing only the runner adapter mapping caused the real CLI test to
  return `snapshot_projection_complete` rather than
  `snapshot_descriptive_finding_ready`. Mapping restored; the CLI E2E passed
  with 10 synthetic contacts in two literal 2027 months and a 2025 invocation
  cutoff. Persisted output contains no fixture IDs and the snapshot envelope
  itself contains no cutoff.
- Independent review round: privacy and product/semantics reviewers both
  returned NO-GO for exposing exact `rejected_rows` and `suppressed_cells`.
  Their evidence was that suppressed-cell totals and rejection prevalences add
  unnecessary small-cell metadata and permit cross-run differencing, while
  neither count changes the proposal decision. The implementation now omits
  these totals from PreparedSource/AgentInput, the core projection, findings,
  and `result.json`; the focused fixtures retain nonzero rejected/suppressed
  rows to exercise that disclosure boundary. The discovery route now fixes
  `k=5` under policy v1 in public configuration and rejects other k values at
  the core boundary; changing k requires a new policy version/release.
- Follow-up public-API audit found a second path outside `result.json`:
  `Projection<T>` and `SnapshotDescriptiveProjection<T>` exposed exact counts
  as public Rust fields, while callers could construct a variable
  `ProjectionPolicy`. The output structs no longer expose exact counters;
  `ProjectionPolicy` fields are private and can only be obtained as fixed v1
  (k=5), and `SnapshotDescriptiveManifest::new` no longer accepts a policy
  argument. Focused API regressions check safe debug/output shape and the
  five-row threshold behavior. This is distinct from the CLI-only boundary.
- The semantic review noted that `snapshot_descriptive_opportunity_ready` could
  overstate what a visible complaint cell establishes. The terminal state is
  now `snapshot_descriptive_finding_ready`; the envelope remains an unverified,
  non-executed hypothesis and makes no causal, ROI, or publication claim.
- Full original-source aggregate-only smoke completed successfully. It reads
  `D:\.codex\factored\data` locally and writes only below the worktree's
  isolated `target-original-snapshot-red\original-smoke-output-current`; no raw
  data is uploaded or printed. Invocation:
  `improvement-engine.exe local-sim --mode local-simulation --source original --input D:\.codex\factored\data --output <isolated-worktree-output> --tenant-id pulso_local --observed-cutoff 2026-10-02T00:00:00Z --arranque-cases 1`.
  It completed with `snapshot_descriptive_finding_ready`, partial coverage,
  37 literal months, 686,290 visible contacts (117,021 complaint contacts), and
  25 visible reason×channel cells. Exact rejection/suppression counters are not
  part of the emitted contract. The scan covered 7,671 source CSV partitions,
  including 1,097 contact-table partitions. This is an aggregate-only local
  smoke; its visible-contact totals are sums of cells satisfying policy v1
  (k=5), not raw-row values or a completeness/business-prevalence guarantee.

## Denominator and privacy

The k policy is applied per valid literal month × normalized reason ×
normalized channel cell before suppression. A cell with fewer than k rows is
suppressed. `supported_contact_count` is the sum of the visible qualifying
cells only. Invalid timestamps and missing/unusable grouping labels do not
enter a visible cell; suppressed cells are likewise omitted. Exact counts for
either condition are not serialized to agent-facing inputs, findings, or
`result.json`, avoiding small-cell metadata disclosure and cross-run
differencing. No row identifiers, customer values, free text, source paths, or
contact outcomes appear in the envelope. Independent privacy and
product/semantics reviews rejected publishing those exact counters; the CLI
also rejects a per-run `k` override.

## Remaining limits

This is historical descriptive E2E, not production discovery. It does not
prove contact causality, repeat-contact status, business lift, reduced PQRs,
ROI, or operational suitability. Native Agent Core artifact authoring and
admission remain dependency-blocked until Agent Core defines a snapshot-only
contract that preserves this authority boundary.

## CI correction (2026-10-03)

The first PR verification failed at `cargo clippy --workspace --all-targets --
-D warnings` on both Ubuntu and Windows. The diagnostic was
`clippy::type_complexity` for the private contact projection helper's
three-part tuple return. Reproducing the pinned lint locally produced the same
warning. The helper now names that tuple with a private
`ContactSnapshotProjection` alias; this changes no runtime behavior. The first
full workspace test run also exposed a stale CLI test that still expected the
removed `--min-contact-cell-count` flag to parse a variable numeric threshold.
The policy is fixed at k=5, so the test now asserts the flag is rejected for
both a sub-floor (4) and high (10,000) request rather than reopening a per-run
override. The focused regression passed. Post-fix `cargo test --workspace
--features test-support` passed, including 112 core unit tests, all workspace
integration tests, 50 core doctests, and the original-snapshot CLI tests; the
original-data scan test remains ignored. `cargo +1.98.1 fmt --all --check` and
`cargo +1.98.1 clippy --workspace --all-targets -- -D warnings` passed before
the test-only adjustment; Python contract tests passed (11 tests, 1 opt-in
Podman test skipped), fixture validation passed, and Windows Pester passed
10/10. This branch does not contain a `local-ci-preflight.ps1` script, so the
workflow's local gates were run directly. The separate PostgreSQL service tests
were not rerun locally; GitHub's PostgreSQL job passed on the failing head.

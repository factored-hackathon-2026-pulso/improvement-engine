# P2 deterministic signal portfolio

Timestamp: 2026-10-03 13:39 UTC (CODEX)

## Scope

Added an isolated, deterministic portfolio boundary that retains every
independently qualifying authenticated E0 diagnostic signal as a candidate
with provenance. This slice does not wire into the local runner, source
adapters, original-bank discovery, or Agent Core. It uses the existing E0
technical-error diagnostic contract only; it does not claim coverage for
other metric families.

## Behavior and safety

- Candidate eligibility is descriptive only: positive observed count and a
  nonzero known denominator. It is not a causal/business-value conclusion.
- Unsupported and Unknown outcomes carry no counts or rate fields. Their
  tenant/run/grant/authority/source-snapshot/cutoff scope is derived from an
  opaque authenticated E0 query receipt, independent of any successful metric
  measurement; mixed scope fails closed. The test fixture obtains this query
  receipt without invoking the diagnostic sensor, demonstrating that an
  all-metrics-unavailable run can still be represented without caller-supplied
  tenant/run IDs.
- A portfolio containing only Unsupported/Unknown signals is
  `insufficient_evidence`, not `no_qualifying_signals`; this avoids turning
  absent measurement into evidence that no issue exists.
- When no candidate qualifies, any Unsupported/Unknown metric also makes the
  overall result `insufficient_evidence`, even if another measured metric has
  zero positive observations. `no_qualifying_signals` is reserved for a
  complete set of measured outcomes with no qualifying candidate.
- A measured signal with zero known denominator also makes a no-candidate
  portfolio `insufficient_evidence`; empty/uncovered population is not
  evidence of a negative.
- `insufficient_evidence` is now described alongside the other output
  dispositions in the data contract and implementation status.
- Digests use the canonical lowercase `sha256:` representation; repeated
  signal digests are rejected. Candidate and observation ordering is stable
  under input permutation.
- Tests use synthetic fixtures only; no raw dataset rows or PII were added.
- Serialization is an internal projection for current inspection/tests, not a
  durable API or persistence contract.

## TDD and validation

- RED for scope binding: after temporarily removing only the unavailable
  outcome scope comparisons, the tenant/run/snapshot mismatch regression
  failed because the portfolio returned `Ok` for a tenant-B Unknown mixed
  with a tenant-A measured candidate.
- RED for insufficient evidence: the all-unavailable regression observed
  `no_qualifying_signals` instead of the expected `insufficient_evidence`.
- RED for denominator coverage: a measured diagnostic with zero known
  denominator serialized as `no_qualifying_signals`; the regression now
  requires `insufficient_evidence` and passes after the terminal-state fix.
- RED for empty-input semantics: `SignalPortfolio::build(&[])` returned
  top-level `no_qualifying_signals`; the regression now requires
  `insufficient_evidence` while preserving the explicit reason
  `no_signals_provided`, and passes after the status fix.
- GREEN focused command:
  `cargo +1.98.1 test --locked --offline -p improvement-engine-core --lib signal_portfolio`
  — 9 passed, 0 failed (including mixed measured-zero/unavailable,
  uncovered measured-population, and empty-input regressions).
- Before the final insufficient-evidence change, the full core package command
  `cargo +1.98.1 test --locked --offline -p improvement-engine-core` passed
  105 unit tests and 52 doctests; one existing destructive PostgreSQL test was
  ignored.
- Previous full local CI after the zero-denominator correction:
  `pwsh -NoProfile -File scripts/verify-local-ci.ps1` — exit 0. All eight
  selected gates passed: pinned toolchain, formatting, workspace Clippy, Rust
  unit tests (core 145 passed/1 ignored; source-adapters 2 passed), Rust
  integration tests, Python contracts (11 passed/1 container test skipped),
  artifact-envelope fixture validation, and Windows Pester tests (15 passed).
  The ignored PostgreSQL/destructive checks require explicit opt-in; the
  container test was skipped because `PULSO_RUN_CONTAINER_TESTS=1` and a
  working Podman backend are required. The script reported
  `Local CI preflight passed for the selected gates.` This earlier run predates
  the empty-input disposition correction; final post-correction result is
  recorded in the addendum below.

## Review and integration state

Independent adversarial review identified and prompted correction of four
issues: unavailable outcomes lacked run scope, requiring a measured signal
prevented representing a run with no successful metrics, and an all-
unavailable portfolio could imply a measured negative. Tests now use an
authenticated query receipt, prove rejection of mixed tenant/run/snapshot
scope, and classify all-unavailable or mixed measured-zero/unavailable runs
as `insufficient_evidence`; they also cover a zero-known-denominator measured
signal. See the final review addendum below. No push or PR has been created.

## Final gate and review addendum

Timestamp: 2026-10-03 16:20 UTC (CODEX)

- After the empty-input correction, the first full CI attempt stopped at
  formatting. Ran `cargo +1.98.1 fmt --all`, then reran the exact full gate.
- Final command `pwsh -NoProfile -File scripts/verify-local-ci.ps1` exited 0
  and printed `Local CI preflight passed for the selected gates.` All eight
  gates passed: pinned toolchain, formatting, Clippy, Rust unit tests (core
  145 passed/1 ignored; adapters 2 passed), Rust integration/doctests, Python
  contracts (11 passed/1 Podman test skipped), artifact-envelope fixtures,
  and Pester (15 passed). PostgreSQL destructive tests remain opt-in/ignored.
- Final focused `signal_portfolio` unit tests: 9 passed, 0 failed. The empty
  input disposition regression was observed RED before the status change and
  GREEN after it. `git diff --check` passed.
- Parent reports independent final P2 review passed after the empty-input
  correction. P2 remains uncommitted until this consolidation step; no push or
  PR has been created.

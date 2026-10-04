# 0496 — DREPLAY source-agnostic replay campaign coordinator

## Scope

Compose the existing deterministic `ReplayPlan` and Frozen/Prequential
`ReplayProtocolMachine` into a one-cohort-at-a-time in-memory campaign. The
campaign binds the source snapshot digest, run-config digest, temporal schedule
digest, protocol, and initial revision. Each detector work item carries the
campaign digest; its work hash therefore binds source/config/schedule/protocol/
revision commitments. Detector work receives one indivisible cohort and its
planner-approved visible-event IDs; advancement currently accepts a canonical
digest-shaped token from a trusted caller. It does not verify that the token
commits to the work or authenticate an evaluator. The future evaluator adapter
must authenticate the producer and bind the result receipt to this
campaign/work item before this coordinator can be used as a trusted evaluation
gate. The coordinator does
not invoke a detector or evaluator, inspect labels/timeline, alter source
relationships, or claim E0 evaluation acceptance.

The checkpoint supports typed clone/restore inside the process only. It is not
serializable or durable and makes no crash/restart recovery claim. Durable
recovery still requires a versioned persistence contract and an atomic
cursor/receipt transaction. The work DTO contains identifiers for the in-process
consumer; the serializable public summary contains only digests, protocol,
counts, revision, and completion status.

The lower-level state machine accepts/stores an arbitrary opaque seal string;
the campaign boundary validates canonical lowercase SHA-256 digest syntax,
but neither API proves an evaluator ran, authenticates the caller, or binds
the token to work. Frozen campaigns cannot queue updates, and a completed Prequential
campaign rejects a revision that would have no later cohort on which to
activate. Debug representations of identifier-bearing work/checkpoint objects
are explicitly redacted; callers must still avoid logging raw DTO fields.

## TDD and validation

- RED: campaign integration tests initially failed to compile because the
  coordinator API did not exist. An adversarial regression for revision after
  the final cohort then failed because it was accepted but could never activate.
- GREEN: focused replay protocol integration target passed 18/18. Coverage
  includes tied cases, event visibility supplied by the planner, frozen and
  prequential revision timing, empty-cohort rejection, exact retry and
  conflicting seals (including retry while later work is pending), exact digest
  binding on restore, pending-work and queued-update resume, a 1,800-cohort
  run, duplicate IDs in a large cohort, canonical inputs, and an identifier-free
  summary.
- RED/GREEN adversarial regression: the initial cross-campaign substitution
  test showed source A's work item could seal source B's identical-plan
  campaign. Adding campaign identity to the work DTO and digest rejects the
  foreign item without consuming B's pending work.
- A second adversarial pass found derived `Debug` output could print case and
  event IDs; public error Debug was a remaining leak. Identifier-bearing DTOs
  and errors now provide redacted/count-only Debug representations; regressions
  cover work, campaign/protocol checkpoints, and wrapped errors. The API docs
  explicitly state that the trusted caller supplies an unbound digest token;
  evaluator authentication and receipt-to-work binding are not implemented.
- Final reviewer identified a module-doc ambiguity because the lower-level
  protocol API intentionally accepts any opaque string while the campaign API
  validates SHA-256 syntax. Clarified both API boundaries; neither authenticates
  or work-binds the evaluator receipt.
- `cargo +1.98.1 fmt --all -- --check` and `git diff --check` passed after the
  final source/docs edits. The consolidated Windows `scripts/verify-local-ci.ps1`
  passed on this tree with one Cargo build slot, including Clippy, workspace
  unit/integration/doc tests, CLI/source-adapter tests, Python contract tests,
  fixture validation, and Pester. Two explicitly destructive PostgreSQL tests
  were skipped because no isolated database/consent was supplied.
- First adversarial review found a terminal-update state error, quadratic
  duplicate-ID lookup, a need to test pending and queued-update resume, and the
  distinction between typed in-memory checkpoint and durable restart. The first
  three were corrected/tested; the durability limit is documented as an
  explicit remaining integration requirement. A second adversarial review
  found the cross-campaign substitution gap; campaign binding and its regression
  test are now in place. The same follow-up identified identifier leakage via
  derived Debug implementations; those are now redacted and regression-tested.

## Remaining boundary

This is a bounded replay coordinator, not the spec's complete replay/evaluation
flow. No durable checkpoints, E0 contract-compliant run, holdout labels, lift,
causality, business impact, production memory update, or Agent Core execution
is demonstrated. In particular, the known E0 `turn.evidence_ids` relationship
discrepancy remains fail-closed pending the source-contract owner decision.

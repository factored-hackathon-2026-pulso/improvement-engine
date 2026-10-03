# E0 read-only investigation proposal plan

**Date:** 2026-10-03
**Owner:** CODEX
**Branch:** `feat/p2-e0-investigation-plan`
**Base:** `3f58a4c2b436446fae74bd565dbf58d3df4e0ebe`

## Purpose and boundary

This slice adds a typed decision envelope for the existing E0 recurrence
candidate path. It composes the descriptive proposal seed, the P3
candidate-bound evidence packet and the exact P3 route resolution (including
its immutable catalog reference). The envelope is named and serialized as
`e0_read_only_investigation_plan_not_agent_core_proposal`: it is not an Agent
Core Proposal, Agent, Flow or Jev artifact. It grants no authority and cannot
compile, execute, evaluate, publish or release anything. It makes no causal or
business-lift claim.

The unlinked resolution offers `investigate_mapping` or `do_nothing`. An exact
mapped contract fixture may offer `investigate_mapped_flow` or `do_nothing`,
where the mapped option is a read-only inspection prompt. Both envelopes are
`pending_review` and `not_executable`. Tenant scope and source query content
remain excluded from the serialized envelope.

The plan's deterministic SHA-256 commitment covers its schema, artifact kind,
candidate/evidence lineage, exact route resolution/catalog reference, decision
options, claim level and absent lift. Integrity validation checks both digest
and repeated field bindings. The runner persists the plan only when a
qualifying E0 recurring-query candidate is accompanied by a valid mechanism
packet/resolution. It appends one privacy-safe plan RunEvent after the
mechanism event to both JSON and NDJSON timelines. Other E0 candidates without
the supported recurrence route, no-candidate/insufficient runs and
OriginalBank receive no plan or plan event.

The resolver receipt now carries the exact `metric_id` and opaque
`pattern_ref` key used for lookup. Plan construction and integrity validation
require those values to match the candidate-bound evidence packet. This closes
a cross-pattern rebinding defect: a valid mapping receipt produced for pattern
B must not be paired with pattern A's candidate and packet.
The adversarial follow-up showed that public enum variants still allowed a
caller to reconstruct the same visible key for A while transplanting B's Flow
reference. The resolver output is therefore now a serialize-only public struct
with private tagged route kind and a private full packet binding. It has no
public constructor and no `Deserialize`; plan validation calls the
resolver-owned packet-binding check. The in-memory binding is excluded from
serialization and custom `Debug` output, and it conveys no execution or
cryptographic authority. A compile-fail rustdoc test covers attempted variant
construction; the A/B regression covers mismatched resolver-issued receipts.

## TDD and validation record

- Core public-API RED: `e0_investigation_plan` was not exported, producing an
  unresolved import in the focused test. The API-export correction made the
  intended test path available.
- Runner public-behavior RED: the recurrence CLI scenario failed because
  `e0_investigation_proposal_plan` was absent. Runner composition made the
  field and event visible.
- First adversarial review found a cross-pattern rebinding gap: route
  resolution did not echo the exact metric/pattern key, so a mapped receipt for
  pattern B could be paired with candidate+packet A. A dedicated A/B regression
  reproduced this as `Ok(plan)` (RED). The resolver receipt now carries the
  exact metric and opaque pattern digest, and the plan rejects mismatches. The
  A/B regression is GREEN.
- Focused tests passed: core plan/resolver tests 9/9; runner `cli_e2e` 14/14
  and `e0_proposal_output` 2/2. Coverage includes mapped and unlinked behavior,
  authority/execution boundary, candidate/packet/catalog/schema/snapshot
  mismatch, deterministic digest, privacy, no-candidate, OriginalBank and
  JSON/NDJSON event parity.
- Core documentation tests passed 53/53, including the compile-fail test that
  prevents public construction of `RouteResolution`.
- `cargo +1.98.1 fmt --all -- --check` passed. All-target Clippy passed for
  core with `local-simulation` and runner using `-D warnings`.
- A second independent adversarial review found the public route enum still
  allowed a caller to reconstruct a cross-packet Flow binding. The resolver
  output was sealed as resolver-minted/private and serialize-only; the
  independent follow-up cleared the fix. See shared log CX-0104/CX-0106.
- Full local CI passed on the final consolidated HEAD: all eight gates green;
  Core 168 passed, 0 failed, 1 destructive PostgreSQL test intentionally
  ignored. Windows actual-data E0 smoke passed on the final HEAD with 200
  cases, a 154/200 recurring-query measure, a descriptive-only holdout, and a
  16-event timeline ending in this plan. The unlinked route recommends mapping
  investigation but formal disposition remains `do_nothing`; it is
  `pending_review`, `not_executable`, and grants no authority.
- A final-head OriginalBank scan was started but stopped before completion
  because the 5.35 GB inventory scan advanced only 3,696/7,671 files after
  several minutes. The prior complete OriginalBank smoke on the consolidated
  predecessor remains recorded in `docs/IMPLEMENTATION_STATUS.md`; the final
  full local suite confirms OriginalBank does not receive this E0 plan. Do not
  represent the new-head live scan as complete.

## Open checks

1. Verify digest catches schema, snapshot, decision, evidence, and catalog
   reference drift; ensure structurally inconsistent plans cannot validate.
2. Verify E0 serialized payload and timeline disclose neither tenant ID nor
   source query/case content.
3. If a fast/reusable OriginalBank scan path is added or identified, rerun its
   full live smoke on the final HEAD; otherwise rely on the prior full smoke
   plus final-head regression coverage and report this limitation.

## Owned files

`crates/core/src/e0_investigation_plan.rs`,
`crates/core/tests/e0_investigation_plan.rs`,
`crates/core/src/lib.rs`, `crates/runner/src/main.rs`,
`crates/runner/tests/cli_e2e.rs`,
`crates/runner/tests/e0_proposal_output.rs`,
`docs/data/e0-proposal-assembly.md`, `docs/local-e0-e2e-runner.md`,
`docs/IMPLEMENTATION_STATUS.md`, and this journal.

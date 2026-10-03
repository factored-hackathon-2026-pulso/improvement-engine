# Restore the verified main baseline

## Scope and provenance

Base: `dcb5bd750485ca1fda7d6fff1df02dea50b8dae7` (main, PR #68).
PR #69 merged into `feat/p2-e0-retry-error-overlap`, and PR #71 into
`feat/e0-frozen-memory-cycle`; neither merge carried its new code to main.
The consolidation selectively cherry-picks #69's `c189c47`, `7bdf5d5`,
`b5850c8` and #71's `04a5166` onto a fresh main-based branch.
It adds no new product feature or external deployment.

## Reproduction and repair

`cargo +1.98.1 fmt --all --check` initially failed with an unclosed
delimiter in the source-adapter privacy test. The assertion loop now closes
and asserts every long unique sentinel. The same fixture contained two
copies of the seven-row sample; retaining one copy restores the intended
five admitted rows, one rejected row and one suppressed cell without
weakening the identifier/field privacy assertions.

## Conflict decisions

- Retain main's `--progress-jsonl` option and its live-progress implementation.
- Retain #69's final policy: discovery uses versioned fixed `k=5`; the
  override is rejected, as in #69's regression tests. Remove the dangling
  help/default/field left by the conflict, rather than advertise an unusable flag.
- Combine main's structured forbidden-field checks with #69's omission of
  exact suppression/rejection metadata and its original-snapshot tests.
- Keep #71's local two-run memory composition and revocation/scope controls.
  It remains in-memory and does not claim durable learning or release.
- Preserve main's V2 run timeline, platform discovery, migrations, local
  preflight and unrelated documentation.

## Validation

- Full Rust workspace suite with `test-support`: passed after isolating the
  runner tests' temporary directories. Windows can return the same timestamp
  to concurrent tests, so snapshot and persistence tests now use distinct prefixes.
- Formatting and `git diff --check`: passed.
- Python contract harness: 10 passed, one opt-in container test skipped; fixtures valid.
- Windows Pester 3.4.0 wrappers: 15 passed.
- Independent adversarial review: no remaining blockers; timestamp privacy
  assertions cover all seven fixture rows.
- Clippy with and without `test-support`, all targets, warnings denied: passed.
- All six isolated real-PostgreSQL CI gates: passed (artifact immutability/CAS,
  model-attempt recovery, observation RLS, run-event atomicity/concurrency,
  authenticated V2 timeline and temporal memory receipts).

Main becomes the verified baseline only after the PR is merged and its
main-branch workflow is green. Final gate results will be recorded before merge.

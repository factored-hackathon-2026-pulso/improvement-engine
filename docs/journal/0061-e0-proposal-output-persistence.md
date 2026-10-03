# 0061 — E0 proposal output persistence

Status: focused public-CLI integration, both actual-data source smokes, and the
full eight-gate local CI are green. No push or PR has been created.

- Stacked exact P0 progress commit `8ca4b5acb5f0569cc6089697825d61eb1073f109`
  onto the reviewed P2 core commit `c150622a7a4dd9677b5b4d83ea44cda71ba8a365`
  as local commit `0f65c6f`.
- Added `crates/runner/tests/e0_proposal_output.rs` before implementation. The
  first public CLI run produced the expected RED: E0 output lacked
  `proposal_assembly`; the OriginalBank negative case passed.
- Wired `assemble_e0_proposals` into the E0-only persistence branch. An invalid
  assembly now aborts persistence through the existing staging cleanup path;
  OriginalBank is not passed to the E0 assembler and receives no E0 portfolio
  or proposal-assembly field.
- The persisted E0 envelope contains all independently qualifying candidate
  seeds exactly once with source run/snapshot and signal provenance. Claims
  remain `descriptive_only`, `unlinked`, `not_evaluated`, null business lift,
  and `not_connected` to native Agent Core. It is proposal input only, not a
  native artifact, evaluation, or business-impact claim.
- Tests also verify fixture case/call sentinels do not appear in the persisted
  E0 result, and OriginalBank does not contain synthetic fixture identifiers.
- GREEN: `cargo +1.98.1 test --locked --offline -p
  improvement-engine-runner --test e0_proposal_output` passed 2/2 (exit 0),
  with `CARGO_TARGET_DIR=target-local-p2-proposal-assembly` and
  `CARGO_BUILD_JOBS=2`.
- P4 read-only adversarial review found no blocker in E0 gating, persistence
  atomicity, output privacy, provenance, or truthful status semantics.
- Full CI follow-up: the first run stopped at formatting; after workspace
  rustfmt, the next run exposed two Clippy style findings and the stacked P0
  source-adapter helper's argument-count lint. These were fixed without lint
  suppression. A regression covers failure while projecting the second
  contact CSV after the first file completed.
- Final acceptance: `pwsh -NoProfile -File
  scripts/verify-local-ci.ps1`, with `CARGO_BUILD_JOBS=2` and isolated
  `CARGO_TARGET_DIR=target-local-p2-proposal-assembly`, exited 0. All eight
  gates passed: pinned toolchain, formatting, workspace Clippy, Rust unit
  tests, Rust integration/doc tests, Python contracts (11 passed, 1 skipped),
  contract fixtures, and Pester (E0 wrapper 20/20; verify-local-ci 5/5).
- Actual-data E0 and OriginalBank smoke runs both exited 0; E0 emitted one
  descriptive proposal candidate, while OriginalBank remained source-specific
  and emitted no E0 assembly. This does not demonstrate business lift or
  native Agent Core execution. Live GitHub base/state verification remains
  required before publishing.

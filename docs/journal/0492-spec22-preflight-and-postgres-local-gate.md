# Spec-22 preflight inventory and complete local PostgreSQL gate listing

## Scope

This journal records two additive changes accumulated into existing draft PR
#95. It does not claim that the full Spec-22 fixture acceptance or destructive
PostgreSQL integration checks are complete.

## Spec-22 family inventory

- Add a versioned inventory of the eleven payload families named by V3 §22.
  It is explicitly an evidence/gap inventory, not a set of the required golden
  payload fixtures.
- A family is `published` only when its `contract_refs` include a valid,
  repository-contained JSON Schema. Internal Rust types and validators are
  not wire-contract references; `read_model` remains partial.
- Validate repository-relative contract/fixture refs, reject absolute and
  parent-traversal paths, and report malformed manifests instead of treating
  external files or internal types as published wire contracts.
- The opt-in `--strict-spec22` gate intentionally fails for six families that
  lack a complete published wire contract: signal, workflow_bridge, tree,
  release_ack, memory_wiki, and read_model. Normal fixture validation remains
  green while reporting those gaps.
- No family payload goldens, Rust serde round-trips, or N/N−1 tests were
  fabricated. FX22a remains incomplete against the plan's acceptance criteria.

## Local PostgreSQL gate inventory

- Add the U24 timeline-v2 test to `scripts/verify-local-ci.ps1 -IncludePostgres`
  and assert the command in `tests/run-local-ci.Tests.ps1`. It remains behind
  existing explicit destructive-test consent and local `pulso_test` safeguards.
- This only ensures the local opt-in runner lists all workflow DB checks; it
  does not make PostgreSQL available or count as a database test pass.

## Verification

- TDD RED/GREEN for the local gate: the focused Pester assertion first failed
  because U24 was missing, then passed after the exact isolated command was
  added. Adversarial review found no P0–P3 findings for this change.
- TDD RED/GREEN for the inventory: 13 focused Python tests pass. Normal
  `python contracts/validate_fixtures.py` passes and reports six pending
  families; `--strict-spec22` fails intentionally with those six names.
  Rust formatting and `git diff --check` pass.
- Final `scripts/verify-local-ci.ps1` exited 0 on Windows with
  `CARGO_BUILD_JOBS=1`: Rust core 200 passed and 2 opt-in PostgreSQL tests
  ignored; Python 24 passed and 1 container test skipped; fixture validation
  passed; Pester 20 + 5 passed. No database service or container was started.
- Independent adversarial review closed the initial status-label, path
  containment, and weak-test findings. No P1/P2 remains. One malformed-manifest
  edge was additionally hardened and regression-tested.

## Remaining acceptance gaps

- FX22a still needs actual valid/invalid wire goldens for all eleven families,
  JSON Schema validation, Rust serde round-trips, and N/N−1 compatibility
  checks. The present inventory is a preflight aid only.
- Destructive PostgreSQL integration tests remain unrun; the local environment
  has no verified disposable `pulso_test` instance. Do not claim database
  integration success from the non-destructive local CI pass.

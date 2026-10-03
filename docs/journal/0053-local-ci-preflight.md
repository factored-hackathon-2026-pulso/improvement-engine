# Local CI preflight

## Behavior

`scripts/verify-local-ci.ps1` mirrors the repository's Rust formatting,
Clippy, Rust test, Python contract, fixture-validation, and Windows Pester
gates so a contributor can run those checks before pushing. `-PlanOnly` makes
the selected checks inspectable without executing them. The optional
PostgreSQL migration suite is excluded by default because it destroys test
data; it requires three explicit flags/values, and only accepts loopback plus
the dedicated `pulso_test` database. The database URL is not printed, and
destructive-test environment consent is scoped to those database commands.

The GitHub workflow runs the plan-contract Pester tests as well as the existing
E0 wrapper tests. CI remains authoritative and includes a disposable
PostgreSQL 17 service that the local script does not start automatically.

## Verification

Commands run:

- `Invoke-Pester -Path tests/run-local-ci.Tests.ps1 -EnableExit` — 5 passed,
  including IPv6 loopback validation, failure-fast exit handling, and restoring
  caller PostgreSQL environment values after an opted-in DB gate fails. It was
  rerun after the adversarial cleanup correction and passed 5/5.
- `scripts/verify-local-ci.ps1 -PlanOnly` — expected safe gates printed; DB
  tests absent by default and included only after explicit opt-in.
- `pwsh -NoProfile -File scripts/verify-local-ci.ps1` — passed on Windows:
  pinned Rust toolchain, fmt, workspace Clippy, workspace unit tests, complete
  workspace integration/doctests, Python contract tests, fixture validation,
  existing E0 PowerShell tests (10 passed), and preflight-contract Pester tests
  (5 passed). Python contract tests reported 11 passed / 1 skipped (the
  optional live Podman contract). The original-data smoke remained ignored.
- All five destructive PostgreSQL CI gates passed against a fresh local
  `pulso_test` on PostgreSQL 18.4: immutable artifacts, model-attempt restart,
  platform-observation RLS/retention, durable run events, and temporal memory
  receipts. The disposable cluster was stopped and its exact scratch directory
  removed. CI uses PostgreSQL 17, so version parity still needs confirmation
  from CI.
- Independent review found and the author corrected a test-harness cleanup
  hazard: capture `PSModulePath` before setup can fail, create temp paths inside
  `try`, and conditionally remove the scratch path in `finally`. The reviewer
  is rechecking this final diff; full-suite results above predate this
  test-harness-only correction, while the focused 5-test suite is current.

## Trade-offs and limits

- This is a focused preflight, not a local Actions emulator: it does not
  install toolchains, launch Podman/PostgreSQL, or claim CI parity for operating
  systems the developer did not run.
- PostgreSQL version parity is the caller's responsibility; the URL guard
  prevents accidental remote/production DB use but cannot prove a loopback DB
  is disposable.

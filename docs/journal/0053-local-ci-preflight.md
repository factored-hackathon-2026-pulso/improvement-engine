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

- `Invoke-Pester -Path tests/run-local-ci.Tests.ps1 -EnableExit` — 4 passed.
- `scripts/verify-local-ci.ps1 -PlanOnly` — expected safe gates printed; DB
  tests absent by default and included only after explicit opt-in.
- `pwsh -NoProfile -File scripts/verify-local-ci.ps1` — passed on Windows:
  pinned Rust toolchain, fmt, workspace Clippy, workspace unit tests, complete
  workspace integration/doctests, Python contract tests, fixture validation,
  existing E0 PowerShell tests (10 passed), and preflight-contract Pester tests
  (4 passed). Python contract tests reported 10 passed / 1 skipped (the
  optional live Podman contract). The original-data smoke remained ignored.
- PostgreSQL destructive integration gates and independent adversarial review
  — pending.

## Trade-offs and limits

- This is a focused preflight, not a local Actions emulator: it does not
  install toolchains, launch Podman/PostgreSQL, or claim CI parity for operating
  systems the developer did not run.
- PostgreSQL version parity is the caller's responsibility; the URL guard
  prevents accidental remote/production DB use but cannot prove a loopback DB
  is disposable.

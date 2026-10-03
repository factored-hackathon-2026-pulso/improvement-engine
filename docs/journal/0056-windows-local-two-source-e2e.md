# Windows local E2E for E0 and original snapshots

## Scope

Add a Windows-first entry point for repeatable local-simulation runs over the
E0 enriched history and the original bank CSV snapshot. Keep each source run
separate and preserve the source-specific meaning of its result. This does not
change runner or detection semantics.

## Decision and behavior

- `scripts/run-local-e0-e2e.ps1` accepts `-Source e0|original`; the existing
  E0 default and its aggregate/holdout output remain intact.
- `scripts/run-local-snapshots-e2e.ps1` validates both input directories, one
  fresh non-overlapping output root, direct filesystem paths, and an explicit
  valid UTC whole-second cutoff before starting either run.
- The combined wrapper invokes the existing runner sequentially with
  `local-sim --mode local-simulation`. Each run gets a distinct `e0/` or
  `original/` directory and writes its own immutable result and event files.
  It never invokes a model provider, Agent Core, or a network service.
- E0 summaries retain the descriptive recurrence/holdout caveats. Original
  summaries validate the snapshot envelope's partial coverage, wall-clock
  month basis, final-extract semantics, unverified/non-executed status,
  non-publishability and dependency-blocked Agent Core marker. Original runs
  reject E0 signals/holdout or an executable proposal payload.
- Existing outputs are never overwritten. If the second source fails after E0
  completed, preserve the first source's derived output; retries use a new root.

## TDD evidence

- RED: the new public-wrapper test first failed because `-Source` was not a
  recognized parameter.
- GREEN: original-source dispatch and source-specific summary validation pass
  using a synthetic Cargo shim; no source rows or private result text are
  printed.
- Regression coverage exercises both-source dispatch, distinct result paths,
  existing-root refusal, junction-root refusal, cutoff validation, safe
  failure handling, original proposal non-executability, and fail-closed
  rejection of explicitly supplied E0-only case-count options for original
  source. Default parameters remain valid for original source.
- `Invoke-Pester -Path tests/run-local-e0-e2e.Tests.ps1`: 15 passed, 0 failed.
- The local safe-CI preflight passed before the final explicit-argument guard:
  Rust fmt, Clippy, workspace tests and doc-tests; Python contract tests (one
  Podman-only test skipped by its documented opt-in); fixture validation; and
  pinned Pester (14 tests at that point, plus five verifier tests). The final
  guard was then covered by the targeted Pester suite (15 passed).
- Independent review identified that the shared PowerShell parameter block
  could accept and silently ignore explicit E0-only counts on original runs.
  The wrapper now rejects either explicitly bound option before Cargo; both
  cases have regression coverage proving Cargo is not invoked.

## Real local two-source execution — 2026-10-03

- Command: `pwsh -NoProfile -File .\scripts\run-local-snapshots-e2e.ps1 -E0InputPath D:\.codex\factored\pulso_muestra_e0 -OriginalInputPath D:\.codex\factored\data -OutputRoot output\snapshot-runs-20261003-cx-full-local-retry -ObservedCutoff 2026-10-03T04:40:00Z`.
- Exit: 0. Both sources wrote one `result.json` and one `events.ndjson` beneath separate fresh `e0/` and `original/` directories. No raw rows were printed.
- E0: `complete_simulated`, 200 discovery cases; technical-error metric 0/187 with 13 missing; tool-retry metric 0/187 with 13 missing; recurring-query count 154/200; holdout `replicated`, 1433/1539 matches and descriptive only; proposal `simulated_unverified`/`not_executed`; formal route `do_nothing`.
- Original: `snapshot_descriptive_finding_ready`; partial final-extract descriptive snapshot only; `simulated_unverified`, `not_executed`, non-publishable; Agent Core dependency `dependency_blocked_snapshot_semantics`; formal route `do_nothing`.
- This proves the local wrapper can execute the two current source paths on the supplied snapshots. It does not prove Agent Core execution, causal explanation, holdout efficacy, business lift, or production behavior. The prior incomplete root `output/snapshot-runs-20261003-cx-full-local` is retained and was not reused.
- Final PowerShell contract suite: `Invoke-Pester -Path .\tests\run-local-e0-e2e.Tests.ps1` passed 15/15; `git diff --check` passed. The post-change `pwsh -NoProfile -File .\scripts\verify-local-ci.ps1` exited 0 with all eight selected gates passing: toolchain, fmt, Clippy, workspace tests/doc-tests, Python contracts, fixture validation, wrapper Pester (15/15), and verifier Pester (5/5). One container-only Python test was skipped by its opt-in guard; no database migration gate was selected.

## Limitations

The PowerShell contract tests use a local Cargo shim; they do not build Rust or
validate the provided datasets. The real local execution above is a separate
evidence item and should not be generalized beyond its recorded inputs/cutoff.
The original-bank output remains a descriptive, partial-coverage snapshot
finding; it is not an Agent Core candidate or executable proposal. E0 holdout
is descriptive recurrence only. Neither output establishes causality, business
lift, or production behavior.

# Windows one-command local E0 E2E runner

## Behavior

Added scripts/run-local-e0-e2e.ps1 as a Windows-first wrapper around the
existing opt-in local simulation. It requires explicit input, output and
whole-second UTC cutoff values; defaults Arranque to 200 and recurrence
support to 20. It invokes Cargo with --locked --offline and a branch-local
target-local-e2e directory. The output directory must be new and must not
overlap the input tree; the wrapper does not delete or overwrite either.

The console is a strict summary projection: run status, discovery/replay
counts, allowlisted metric IDs with numerator/denominator/missing counts,
proposal status/execution status, and formal route. Source paths, raw Cargo
diagnostics, arbitrary result fields, query signatures, hypotheses and
identifiers are not printed. Unknown summary codes fail closed. Persisted
result/timeline files remain sensitive derived data; hashes do not anonymize
their inputs.

## Verification

- RED: before the wrapper existed, Pester's happy-path and Cargo-failure cases
  failed because the public script path was unavailable; safety-precondition
  cases also failed under the host's legacy Pester assertion syntax. Updated
  assertions to framework-independent throwing checks.
- GREEN: Invoke-Pester -Path tests/run-local-e0-e2e.Tests.ps1; 5 passed,
  0 failed. The temporary fake Cargo verifies locked/offline arguments,
  local-simulation E0 arguments, default settings, summary allowlisting,
  explicit cutoff, path overlap, no-overwrite and failure-output suppression.
- Actual local smoke:

      pwsh -NoProfile -File .\scripts\run-local-e0-e2e.ps1 -InputPath 'D:\.codex\factored\pulso_muestra_e0' -OutputPath '.\output\local-e0-run-script-20261002-02' -ObservedCutoff '2026-10-02T18:00:00Z'

  Completed with status complete_simulated; 200 Arranque discovery cases and
  1,800 Reproduccion cases excluded; recurring Copilot-query pattern in
  154/200 discovery cases; technical errors 0/187 supported with 13 missing;
  proposal simulated_unverified/not_executed; formal route do_nothing.
  The result is descriptive behavior on the augmented sample only. It does not
  demonstrate customer friction, causal lift, holdout efficacy, native Agent
  Core, or production performance.
- git diff --check passed. The local E0 package was read by the local runner;
  no source rows, identifiers or labels were printed, and no provider calls or
  dataset uploads were made.

## Trade-offs and limits

Pester is used as a process-level harness with a local Cargo shim because the
script's purpose is orchestration and output safety; it does not replace the
real CLI smoke. The wrapper intentionally rejects unrecognized statuses and
metric identifiers rather than echoing new values. It does not delete failed
partial output; operators must inspect or choose a fresh destination without
the wrapper removing existing data.

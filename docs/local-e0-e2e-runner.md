# Windows local E0 E2E runner

The PowerShell wrapper runs the engine's explicit local-simulation mode against
an E0 package on disk. It is intended for repeatable local checks, not business
evaluation or release.

## Prerequisites

- Windows with PowerShell 7 or newer.
- Rust/Cargo matching rust-toolchain.toml.
- The workspace dependencies already available locally; Cargo is always invoked
  with --locked --offline, so the command does not fetch crates.
- A local E0 package containing the expected datos/ and contratos/ layout.

## Run

From the repository root:

    pwsh -NoProfile -File .\scripts\run-local-e0-e2e.ps1 -InputPath 'D:\.codex\factored\pulso_muestra_e0' -OutputPath '.\output\e0-run-2026-10-02-a' -ObservedCutoff '2026-10-02T18:00:00Z'

InputPath, OutputPath, and ObservedCutoff are required. The cutoff must
be UTC with whole-second precision. Defaults are 200 Arranque cases and a
20-case recurrence-support floor. Override them explicitly when needed:

    pwsh -NoProfile -File .\scripts\run-local-e0-e2e.ps1 -InputPath 'D:\data\e0-package' -OutputPath '.\output\e0-run-custom' -ObservedCutoff '2026-10-02T18:00:00Z' -ArranqueCases 200 -MinimumRecurringQueryCases 20

The output path must not exist, must not overlap the input tree, and neither
path may traverse an existing junction, symlink, or other reparse point in the
path components inspected by the wrapper. Choose a new output path for each
run. The wrapper never deletes or overwrites data. The engine writes its
immutable run result and event timeline below that output directory.

This is not a defense against every filesystem alias or race: mapped-drive and
UNC aliases are not canonicalized against one another, and another process
must not mutate/repoint path components concurrently with validation or the
run. The guarantee is limited to rejecting reparse-point components observed
during the wrapper's preflight checks.

## Output and safety

The wrapper prints only allowlisted run status, discovery counts, and a
suppressed replay total for E0,
allowlisted metric IDs with numerator/denominator/missing counts, proposal
status/execution status, formal route, and (when present) a holdout status.
Holdout matching/queried distinct-case counts are shown only when support meets
the configured floor; `insufficient_support` prints `counts=suppressed`, and
the engine always serializes the E0 top-level excluded-replay total as null,
even when holdout evaluation is absent or unavailable. It must serialize all
holdout count/rate fields as null below the floor. A missing
holdout field is summarized as `none`; it is not treated as zero. It does not
print source paths, customer/case/query identifiers, query signatures,
hypotheses, arbitrary interpretation strings, or raw Cargo/JSON diagnostics.
Unexpected result codes, metric IDs, malformed JSON or invalid aggregates fail
closed without printing the result. Treat generated artifacts as sensitive
derived data: hashing is not anonymization.

The wrapper selects local-simulation mode; it makes no provider or Agent
Core request. The observed query recurrence is descriptive only. It is not
evidence of customer friction, causality, holdout efficacy, business lift, or
production behavior.

## Test

Run with Pester 3.4.0 installed:

    Invoke-Pester -Path .\tests\run-local-e0-e2e.Tests.ps1

The Windows CI job requires the exact Pester 3.4.0 module to already be
available and does not install or upgrade it. The tests use a temporary local
cargo shim to verify argument construction, safe summary filtering, mandatory
cutoff, output/input isolation (including input and output junction paths),
no-overwrite, and sanitized failure behavior. They do not build Rust or prove
the E0 package is valid. For a real run, use the command above against the
local package.

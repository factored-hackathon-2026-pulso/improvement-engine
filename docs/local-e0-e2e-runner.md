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

The output path must not exist and must not overlap the input tree. Choose a
new output path for each run. The wrapper never deletes or overwrites data. The
engine writes its immutable run result and event timeline below that output
directory.

## Output and safety

The wrapper prints only allowlisted run status, discovery/replay counts,
allowlisted metric IDs with numerator/denominator/missing counts, proposal
status/execution status, and formal route. It does not print source paths,
customer/case/query identifiers, query signatures, hypotheses, or raw Cargo
diagnostics. Unexpected result codes or metric IDs fail closed without
printing the result. Treat generated artifacts as sensitive derived data:
hashing is not anonymization.

The wrapper selects local-simulation mode; it makes no provider or Agent
Core request. The observed query recurrence is descriptive only. It is not
evidence of customer friction, causality, holdout efficacy, business lift, or
production behavior.

## Test

With Pester installed:

    Invoke-Pester -Path .\tests\run-local-e0-e2e.Tests.ps1

The tests use a temporary local cargo shim to verify argument construction,
safe summary filtering, mandatory cutoff, output/input isolation, no-overwrite,
and sanitized failure behavior. They do not build Rust or prove the E0 package
is valid. For a real run, use the command above against the local package.

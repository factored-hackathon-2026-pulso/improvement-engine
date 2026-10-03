# E0 portfolio summary in the local wrapper

Timestamp: 2026-10-03 15:52 UTC (CODEX)

## Scope

The E0 PowerShell wrapper now prints the simulator-only portfolio status and
four aggregate disposition counts. The summary intentionally omits metric IDs,
digests, source family/authority fields, reasons, and per-signal values. The
original-bank wrapper path has no portfolio line. JSON artifacts and the
runner's output contract are unchanged.

The wrapper validates the E0 source-family/authority boundary, portfolio status,
allowlisted metric IDs, disposition states, status/count coherence, one-to-one
signal/disposition correspondence, exact digest references, and candidate/primary
digest references before printing. OriginalBank now fails closed if it contains
any non-null E0 portfolio. Invalid/missing portfolio data fails closed using
generic messages.

## TDD and validation

- RED: added a Pester assertion for the sanitized portfolio summary line; the
  initial Pester run failed that assertion because the wrapper emitted no
  portfolio summary (14 passed, 1 failed).
- GREEN: `Invoke-Pester -Path .\tests\run-local-e0-e2e.Tests.ps1 -PassThru` —
 19 passed, 0 failed, 0 skipped. RED regressions first demonstrated acceptance
  of a forged OriginalBank portfolio and malformed E0 disposition mappings;
  the final tests confirm these are rejected without leaking digest/metric fields.
  The primary E0 fixture now matches Rust dispositions (two candidates, one
  insufficient-evidence signal, no not-qualified signals). Additional regressions
  reject paired signal/disposition omission and bind the portfolio primary digest
  to the selected `result.signal.digest`. The test confirms portfolio status/counts
  only and checks the portfolio line for digests, metric identifiers/values,
  source identifiers, and raw content. Original-bank output is asserted not to
  contain a portfolio line. A separate RED/GREEN regression rejects a
  contradictory status/disposition combination without echoing raw fields.
- Initial wrapper-only validation used Pester and `git diff --check`; full local
  CI was deferred to the orchestrator's serialized gate.
- No actual dataset E2E was run. No source rows, PII, or output JSON contract
  were changed. No push or PR has been created.

## Runner persistence regression follow-up

The full-CI compile exposed a `LocalRunResult` literal in the runner persistence
test that did not initialize the newly added optional E0 portfolio field. The
fixture now sets `local_simulation_portfolio: None` and asserts persisted JSON
omits that field under the existing skip-None serialization contract.

- `cargo +1.98.1 fmt --all -- --check` — passed.
- `cargo +1.98.1 test --locked --offline -p improvement-engine-runner persistence_publishes_result_and_timeline_together_and_never_overwrites` — passed (1 passed; 6 filtered out; integration target 11 filtered out).
- `git diff --check` — passed.
- No full local CI was started; the orchestrator owns the single full-suite gate.

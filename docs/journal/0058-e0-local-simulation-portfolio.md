# E0 local-simulation portfolio

Timestamp: 2026-10-03 15:05 UTC (CODEX)

## Scope and decisions

- Added a separate `LocalSimulationPortfolio` to E0 local-simulation results;
  the authenticated P2 `SignalPortfolio` and its U08 provenance contract are
  unchanged.
- Portfolio authority is fixed to `simulator_only`; source family is E0. Each
  existing measured signal keeps its digest reference and an ordered
  disposition. If the query table is unavailable, recurrence is represented as
  unavailable without a digest or fabricated zero.
- Existing signal records remain the only place with numerator, denominator,
  coverage, and missing counts. The portfolio lists candidate digest references
  but does not alter primary selection, recurrence minimum, proposal input, or
  holdout behavior. OriginalBank results carry no E0 portfolio.
- A no-candidate portfolio is `insufficient_evidence` if any metric is
  unavailable, has a zero known denominator, or has partial missing evidence;
  only complete measured non-qualifying observations produce
  `no_qualifying_signals`.
- Timeline detail contains aggregate disposition counts only. No source
  adapter, source database, or write path changed. Outputs remain simulator
  inspection artifacts, not durable evidence or release authority.

## TDD and tests

- RED: the existing multi-signal integration test failed to compile at the
  missing `LocalRunResult.local_simulation_portfolio` field, confirming the
  expected feature absence.
- GREEN: `cargo +1.98.1 test --locked --offline -p improvement-engine-core
  --features local-simulation --test local_simulation` — 23 passed, 0 failed.
- Coverage includes candidate references for all qualifying signals, primary
  selector parity, retry's unknown denominator, source-table unavailable vs
  zero, a complete known-zero no-op, stable ordered metric IDs across repeated
  runs, sanitized timeline aggregate detail, and OriginalBank separation.
- `cargo fmt`, package-wide tests, and `scripts/verify-local-ci.ps1` have not
  yet been run for this slice. No raw dataset or PII was added.

## Review state

Implementation is waiting for independent adversarial review. The feature is
uncommitted and has not been pushed or opened as a PR.

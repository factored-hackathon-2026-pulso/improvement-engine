# Post-merge selective consolidation

Date: 2026-10-04
Owner: CODEX
Base: verified GitHub `main` `41492cb7f2b20fc2d033643798659436dffb861a`

## Scope

This consolidation carries only deltas that remain additive to current `main`:

- Local U12-E source evidence composition into the U13 E0 runner path. The
  adapter constructs evidence from the locally supplied E0 package and the
  configured local simulation source. This is not a bank-issued attestation or
  a production source connector.
- A fail-closed platform-source privacy policy that only admits reviewed,
  allowlisted event fields and excludes simulator-only rows and unsafe text
  from live model context.
- Paired scenario comparison for fixture data, marked
  `FixtureOnlyUnverified`; it does not assert causal impact, production lift,
  or customer outcomes.

The local U12-E composition is intentionally tagged
`positive_count_plumbing_v1`: it proves the evidence→composition→admission
path, but the “any positive numerator” trigger is not a calibrated or
configurable business detection threshold. This is a known scope limit, not a
recommendation to use the trigger in production.

## Explicit non-claims

No live LLM/provider or native Agent Core execution occurs in this slice. No
new bank policy authority, deployable agent artifact, production signature,
business outcome, or measured lift is created. Original-source input does not
receive synthetic U12 evidence. Unsupported evidence remains blocked rather
than being fabricated.

## Validation and review

Two independent adversarial reviews found no immediate PII-egress bypass or
production-authority confusion. Their findings were addressed as follows:

- Persist only candidate content digests in the composition summary so the
  debug view can identify exactly which drafts crossed admission without
  exposing raw draft context or IDs.
- Name the trigger `positive_count_plumbing_v1` and document that it is a
  plumbing demonstration, not a calibrated/configurable detection threshold.
- Make oracle-match results tri-state (`true`, `false`, or `not_evaluated`) so
  sandbox failures cannot be misread as behavioral failures.
- Keep the platform source component explicitly scoped to policy primitives;
  `PlatformSourceReader` is not a sandbox and a contract-backed live exporter,
  schema enforcement tests, and source-treatment authority remain prerequisites
  before live platform reads or hosted-model egress.

The Windows `scripts/verify-local-ci.ps1` preflight passed after these changes:
Rust 1.98.1 format, Clippy with warnings denied, workspace unit/integration/
doc tests, Python contract tests and fixture validation, plus Pester suites
(20/20 and 5/5). PostgreSQL destructive integration tests were not run because
they require explicit local database opt-in and an available isolated backend.
No hosted Actions result is inferred from local validation.

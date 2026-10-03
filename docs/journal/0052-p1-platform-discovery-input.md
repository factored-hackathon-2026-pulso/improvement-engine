# P1 — prepared discovery input from governed platform signals

## Behavior and boundary

`PlatformDiscoveryInput::from_measured_signal` copies a measured U30 signal
into a typed, read-only discovery payload. It preserves opaque tenant scope,
metric/version/layer/population, measured counts, observation window and as-of clock, U29 projection,
source contract, batch and coverage receipts, U30 trusted mapping resolution,
and the U30 signal digest. A canonical SHA-256 commitment binds all of those
fields. Public consumers can inspect and serialize the input but cannot build
or deserialize one by supplying their own evidence fields.

The constructor requires the active `CoreTaskScope` and rejects when its
tenant differs from the U29 projection tenant. An A/B integration regression
creates equal measurements in two separately authorized tenant repositories:
their U30 signal and discovery commitments differ, and replaying tenant A's
signal under tenant B's scope returns `TenantScopeMismatch`.

Insufficient, ambiguous and inconsistent U30 outcomes are rejected. No rate is
converted into a causal claim, Opportunity artifact or executable proposal.

## Deliberate limitation / next dependency

This is an enabling discovery seam, not complete U13 Scout integration. U13's
current authenticated routes require different source, tenant/grant/authority,
Core-task and model-provider receipts (U08/U09/U10, or E0-specific U04/U08/U12
bindings). U30's platform metric contract does not provide those receipts, and
there is no trusted platform-specific invocation authority/receipt contract.
Therefore this slice does not mint `ScoutCandidateDraft`, `VerifiedScoutCandidate`,
or `Opportunity`; adding those would fabricate authority. The next slice must
define and test a platform-specific invocation boundary that can take this
sealed input through the approved hypothesis/Scout process without weakening
the U13-E0 route or attributing arbitrary LLM output as evidence.

## TDD and verification

- RED: the measured-input regression first failed because the module did not exist. The tenant-binding regression later failed to compile because the scope argument and mismatch error did not exist.
- GREEN: `cargo +1.98.1 test --locked --offline -p improvement-engine-core --test platform_sensor` — 12 passed, including measured integration, partial-coverage rejection and cross-tenant replay denial.
- GREEN before the tenant-binding addition: `cargo +1.98.1 test --locked --offline -p improvement-engine-core --doc` — 51 passed, including external construction compile-fail. Re-run after final rebase is pending.
- Fixture path uses the real U29 in-memory authorized repository and U30 sensor. PostgreSQL is not involved; this slice adds no persistence.
- Full core suite, clippy and final independent review/CI remain pending.

# Platform-live source privacy guard

**Status:** internal PL-C4 policy primitives; the platform exporter/schema contract is still owned by the platform integration lane. This document does not claim the exporter is implemented or that any platform database has been read.

## Boundary

`PlatformSourceReadPlan::platform_live()` is a closed Rust capability, not a SQL query builder. A source reader receives only the typed relations `cases`, `turns`, `assignments`, `customer_case_slots`, `customers`, `event_log`, and `staff`, with semantic field allow-lists. There is no relation variant for `login_accounts`, `mfa_challenges`, or `staff_sessions`; there is no arbitrary table-name input. Staff/customer names, email, hashes, and authentication/session material have no projected-field variant. The adapter must fail closed if it cannot map the plan to the exact versioned source contract.

The `cases` projection omits mutable status and close labels. Those are reconstructed as-of from `event_log`, `assignments`, and `turns`; the current `cases` row must not leak future state into discovery. Event payload is explicitly local-only. `auth.*` events are denied by default; an exact, syntactically validated allow-list is required, and unknown event types are quarantined rather than passed through a wildcard.

Rows whose `customers.simulator` flag is true are excluded from platform populations. Customer/staff/case identifiers are join keys only and must not be interpolated into model context.

## Text and model egress

Platform turn text has no serialization or raw-value accessor and formats as redacted in `Debug`. No live source-treatment authority or independently verifiable treatment receipt exists yet. Therefore the internal treatment constructor always returns `AuthorityUnavailable`, and the egress boundary returns `TreatmentAuthorityUnavailable` before `ProjectionBrokerPort`, both when no receipt is supplied and when a caller presents a self-asserted/forged receipt. Platform text egress is unavailable by design until an authority verifier is implemented. Local reads do not imply permission for hosted-model egress.

Arbitrary event JSON uses the distinct `PlatformEventPayloadLocalOnly` type. It has no conversion to `PlatformTurnBody` or `PlatformSourceText`, and its explicit egress method always returns `LocalOnlyEventPayload` without invoking the broker. A compile-fail doctest protects the type boundary.

`PlatformEventCatalog` requires a positive pinned version and computes a domain-separated SHA-256 digest over the version, the built-in business-event set, and the sorted/deduplicated exact auth allow-list. `PlatformSourceReadPlan::provenance()` returns a serializable `PlatformSourcePolicyProvenance` containing this catalog reference so a source/run manifest can bind and replay the exact classifier catalog.

This module does not infer names from natural-language text, implement a generic DLP engine, or decide the bank's retention/egress authority. The platform contract/exporter must provide the exact field map and a trusted source-treatment implementation plus independently verifiable receipt before wiring this policy into ingestion. Until then, all turn-text egress intentionally remains blocked; the module is an independently testable guard, not a live-source integration claim.

## Verification

`cargo test -p improvement-engine-core --test platform_source_policy --no-default-features` covers the static read-plan boundary, exact auth-event allow-list behavior, stable manifest provenance, simulator exclusion, raw-text fail-closed behavior, and the local-only event-payload boundary. The source plan test records every relation and column; no credential relation can be passed to the typed reader. Contract-backed SQLite/Postgres exporter tests belong to the platform exporter/schema owner and remain pending.

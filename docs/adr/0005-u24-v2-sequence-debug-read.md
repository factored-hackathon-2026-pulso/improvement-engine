# ADR 0005: U24 reads the durable V2 run sequence

## Status

Accepted for the U24 read adapter. This supersedes ADR 0004's statement that
U24 must wait for a versioned sequence-native read contract; the legacy U07
`RunActivityReadModel` remains unchanged and does not read `pulso_run_events`.

## Context

The durable event identity is `(tenant_id, run_ref, sequence)`. The legacy U07
cursor is job/timestamp based and cannot faithfully represent this ordering.
U24 already has an authenticated, read-only composition boundary that derives
tenant from the same per-call `DebugViewerIssuer` capability. Reusing the old
timeline types would conflate two different contracts; exposing stored text
directly would also allow arbitrary code-like strings to cross into debugging
responses.

## Decision

- Add a distinct U24 V2 request with only `run_ref`, `after_sequence`, and
  `page_size`. Validate UUIDv7 run identity, nonnegative sequence, and a page
  size from 1 through 100. The request contains no tenant selector.
- Authenticate through the existing trusted `DebugIdentityPort` and one-call
  issuer. Pass only the resulting `AuthenticatedTenant` to the read port.
- Before reading events, require a root row scoped by both `tenant_id` and
  `id=run_ref`; missing and cross-tenant runs map to the same safe `NotFound`
  result. Every event query independently predicates `tenant_id`, `run_ref`,
  and `sequence > after_sequence`.
- Order exclusively by `sequence ASC`; fetch one extra row to determine whether
  a continuation exists. When present, `next_after_sequence` is the last
  returned sequence. The cursor is a position, not authorization; each page
  repeats tenant/run checks.
- Project only internal event/run/job refs, sequence, timestamp, and closed
  event vocabulary. Stage/event/status values use code allowlist version 1,
  reported with the page; an unknown stored value becomes the constant `other`.
  Omit
  `reason_code`, `artifact_ref`, `trace_id`, `details_ref`, payloads and raw
  errors entirely. Do not change the schema or durable event timeline.
- The composer has no mutation port. Storage or malformed stored-sequence
  failures map to generic `Unavailable`; raw PostgreSQL error details are not
  returned by the composer.

## Consequences

U24 can page the durable run ledger without changing U07 legacy behavior. A run
with no events remains a valid empty page; a missing or foreign run is
indistinguishable. New event names must be deliberately added to the safe code
allowlist before their exact values are visible in the debug contract. This is
an internal Rust composition boundary, not an HTTP endpoint, browser UI, SSO
implementation, stream, or operator command. The reader module stays
`pub(crate)` because `AuthenticatedTenant::new` is not proof of identity. A
future cross-crate endpoint must expose a facade that accepts only an
authentication-issued viewer/context, never a tenant string.

## Validation

Unit tests cover request bounds, authenticated tenant derivation, denial before
read, storage-error redaction, and unknown-code coarsening. The ignored
PostgreSQL integration test uses the existing migration and real PG client to
cover tenant/run isolation, sequence pagination, and omission/coarsening of
reason/artifact/trace/detail and arbitrary code strings. It requires an
isolated `PULSO_TEST_POSTGRES_URL` plus `PULSO_ALLOW_DESTRUCTIVE_TEST_DB=1`;
it additionally verifies the connected database is exactly `pulso_test` before
applying migration/truncation. Windows local PostgreSQL could not bind a port
in this environment, so CI's ephemeral PostgreSQL service is the required
real-database gate. No persistence mock substitutes for that gate.

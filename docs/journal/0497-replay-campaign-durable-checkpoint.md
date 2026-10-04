# CX-0497 — Durable replay campaign checkpoints

**Status:** implemented; final focused local tests pass; isolated PostgreSQL execution remains environment-gated.  
**Owner:** X-LEARN  
**Recorded:** 2026-10-04 13:00 UTC

## Goal

Make replay campaign progress recoverable across process restarts without
copying case, event, or customer identifiers out of the immutable source
snapshot. This is persistence for the replay coordinator only; it does not
authenticate an evaluator or establish outcome validity.

## Implementation

- Added a canonical JSON wire document at version 1. It contains the campaign,
  source, configuration, and schedule commitments; cursor/revision metadata;
  digests of work and update identifiers; and evaluator receipt digests. Raw
  replay identifiers and update IDs are omitted.
- Restore checks canonical bytes and exact plan/source/config bindings, then
  reconstructs state by replaying each cohort seal and queued revision update.
  It compares the reconstructed wire state to the supplied document and rejects
  tampered cursors, journal entries, or protocol state.
- Added PostgreSQL append-only revision history plus a head row. Each commit is
  a single transaction guarded by revision and cursor CAS; exact payload retries
  are idempotent. Existing seal history cannot be rewritten, and conflicting
  receipt digests are rejected.
- Revision update IDs are normalized with a domain-separated SHA-256 commitment
  inside the replay protocol. The public API always hashes caller strings as raw
  IDs; only a crate-private typed path can restore a commitment. This preserves
  same-ID retry/conflicting-reuse semantics without allowing caller strings to
  opt into the persisted identity namespace. The commitments are pseudonymous,
  not anonymization.
- `load` cross-checks head and revision revision/cursor/version/digest metadata
  against the canonical payload before replay restoration.
- The process-local `ReplayCampaign::checkpoint` / `restore` API remains
  available. Durable persistence is an additional adapter, not a replacement.

## Validation and boundaries

- RED: before the storage module and migration existed, the focused target failed
  to compile because `replay_protocol::store` and migration `0006` were absent.
- Final GREEN after adversarial fixes: `cargo +1.98.1 test --locked -p
  improvement-engine-core --features local-simulation --test
  replay_campaign_store` — 4 passed, 2 ignored. The ignored PostgreSQL tests
  remain unexecuted because no isolated test database and explicit consent were
  supplied.
- The ignored PostgreSQL tests require `PULSO_TEST_POSTGRES_URL` and explicit
  `PULSO_ALLOW_DESTRUCTIVE_TEST_DB=1`; they were not run, and no container was
  started. They cover two-writer CAS contention, exact retries, seal conflicts,
  corrupted head metadata, and a forced failure after revision insert to check
  transaction rollback. Those database behaviors remain unverified at runtime
  in this environment.
- Receipt digest format validation is not evaluator authentication. A trusted
  evaluator adapter must verify/bind the receipt to the exact work before
  calling `seal`; this slice makes no such claim.
- The checkpoint is versioned and immutable by revision; retention, tenant
  partitioning, and external backup policy remain deployment concerns.

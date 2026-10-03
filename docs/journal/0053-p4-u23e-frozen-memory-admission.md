# P4 U23-E Frozen memory admission

## Slice

Added a crate-private U23-E admission seam over the existing U33-E Frozen
publication state. It does not create a second memory registry or ledger and
does not route through generic U22/U33 publication. Admission re-attests the
exact immutable publication reference and current head, tenant/world/scope,
Frozen replay cutoff, current grant revision, and revocation state before
recording a payload-free use receipt in the same U33-E state.

The receipt key is the semantic tuple
`(publication_commitment, scope, run_id)`. Exact retries return the canonical
receipt. A different valid grant or access timestamp for the same tuple is a
conflict and cannot create a second receipt. A changed access snapshot,
publication revision, head, revoked grant/snapshot, scope drift, or use after
cutoff fails closed.

The durable U33-E adapter deliberately returns `DependencyUnavailable`. It
must remain unavailable until one PostgreSQL transaction can check the exact
publication/head, U05 grant revision/liveness and revocation, then write the
canonical U23-E receipt. This slice is not yet selected by the local CLI or a
production runtime; it establishes the sealed Rust contract and in-memory
semantic adapter only.

## TDD and verification

The changed-grant regression was run against the earlier identity that
included grant/time and failed as expected: a second valid grant created a new
receipt instead of returning a semantic conflict. The implementation changed
the identity to the publication/scope/run tuple while retaining grant/time in
the receipt's temporal commitment.

Focused U23-E tests:

```text
cargo +1.98.1 test --locked --offline --target-dir target-u23e-cycle -p improvement-engine-core --lib --features test-support e0_frozen_memory_publication::tests::u23e_
```

Result after the fix: `11 passed; 0 failed`.

Core unit suite:

```text
cargo +1.98.1 test --locked --offline --target-dir target-u23e-cycle -p improvement-engine-core --lib --features test-support
```

Result: `122 passed; 0 failed`.

Clippy:

```text
cargo +1.98.1 clippy --locked --offline --target-dir target-u23e-cycle -p improvement-engine-core --all-targets -- -D warnings
```

Result: passed. `rustfmt --edition 2024` and `git diff --check` also pass.

## Review disposition

An adversarial P4 review found that the original receipt ID included the grant,
revision and timestamp, permitting multiple records for one logical run. The
fix and three regressions cover changed valid grant, changed valid timestamp,
and wrong access snapshot. Independent review of the corrected semantic slice
returned GO. The focused U23-E suite and Clippy were rerun before the local
commit; this is not a merge or durable PostgreSQL approval. Durable admission
remains dependency-blocked pending a real single-transaction adapter test.

## Durable contract audit and blocker (2026-10-02)

Read-only inspection found usable but incomplete PostgreSQL primitives:

- `migrations/0001_pulso_artifact_revisions.sql` owns immutable U02 revisions.
- `migrations/0002_pulso_memory_control.sql` owns scoped
  `pulso_memory_heads`, immutable `pulso_memory_tombstones`, and
  `pulso_memory_use_receipts`.
- `migrations/0004_pulso_memory_temporal_receipts.sql` adds the temporal
  commitment/event key and `pulso_record_memory_use_temporal`; that function
  locks and validates the current head, checks the snapshot and revocation
  lineage, and writes a use receipt. Its `p_grant_ref` is only a string: the
  SQL does not resolve grant revision, status, expiry or revocation.
- `crates/core/src/e0_frozen_memory_publication.rs` has the in-memory U33-E
  publication sidecar/head and `DurableFrozenE0SummaryPublicationUnavailable`.
  There is no durable U33-E publication table/adapter to re-attest the exact
  `FrozenE0PublicationRecord` before U23-E use.
- `crates/core/src/wiki_scratch.rs` defines
  `MemoryUseCommitAuthority::memory_use_grant_is_live`, but only the in-memory
  authority implements it; it is not transaction-aware and is not U05's
  durable grant lifecycle contract.

These pieces cannot safely be composed as-is: validating grant state in Rust
before the SQL call would permit a revoke/reissue race; calling the existing
temporal writer with a caller-supplied `grant_ref` would assert authority the
database never checked; and checking a Rust-only U33-E publication would not
prove that the durable current head has the exact published commitment. The
durable adapter therefore remains `DependencyUnavailable`. No parallel grant
authority table, mock durable-success adapter or redundant receipt table was
added. Existing `pulso_memory_use_receipts` is the candidate durable ledger;
U23-E may bind its semantic `(publication_commitment, scope, run_id)` key to
the existing event/idempotency key only after the U33-E and U05 transaction
contracts agree.

Minimum U05/U33-E adapter contract to unblock implementation:

1. U05 supplies a transaction-bound `lock_and_resolve(tx, request)` operation,
   not a pre-read boolean. The request is exact `grant_id`, requested revision,
   run, tenant, purpose/action, scope and snapshot reference. Its ephemeral
   `GrantSnapshot`/`AuthorityDecision` returns authority reference, tenant,
   grant ID and revision, active/revoked state, validity interval, revocation
   epoch/commitment, allowed action, exact scope/snapshot binding and a
   canonical authorization digest. The result is usable only for the same
   transaction that holds the U05 liveness/revision fence through commit.
2. U33-E accepts that decision only inside the same PostgreSQL transaction.
   It rechecks the exact publication sidecar and current head/version, scope,
   immutable U02 memory revision/digest, ancestry revocation overlay and
   `allowed_at <= U04-B cutoff`; then it inserts/retrieves one payload-free
   receipt keyed semantically by `(publication_commitment, scope, run_id)`.
   Exact retry returns the canonical receipt; any changed grant revision,
   access time, publication or scope conflicts. Head/revocation/grant races
   must serialize against this transaction.
3. Persist the missing U33-E publication sidecar as a new append-only Pulso
   table/row only when its exact record schema is finalized. Reuse U02 for
   artifact bytes/revisions and the existing U33 head/tombstone/use-receipt
   structures; do not add another wiki store, authority ledger or memory-use
   receipt table.

Required real-PostgreSQL acceptance tests once U05 exposes that contract:

- apply forward migrations to an isolated disposable PostgreSQL database;
- admission for exact tenant/world/scope/publication/head/grant revision and
  cutoff commits exactly one receipt, with no payload in the receipt;
- exact retry returns the same receipt; changed grant revision, timestamp,
  publication, scope or snapshot for the same `(publication, scope, run)` is
  rejected without a second row;
- missing, expired, replaced or revoked U05 grants and revoked snapshot
  lineage fail with no receipt; advance-head, revoke-vs-use and two concurrent
  same-key calls are exercised with independent connections/barriers;
- force an error after validation and prove the transaction leaves neither a
  receipt nor partial admission; verify runtime role cannot bypass the
  privileged transaction function.

Local DB feasibility check on 2026-10-02: `pg_isready -h localhost -p 5432`
reported no response; Podman was unavailable because its machine identity file
returned Access Denied. No PostgreSQL test was attempted, so this lane makes
no durable-persistence claim. The existing semantic U23-E tests remain the
only green evidence for this branch.

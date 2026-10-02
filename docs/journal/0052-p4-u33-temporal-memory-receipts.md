# P4 U33 temporal memory-use receipt persistence

## Contract added

Migration `0004_pulso_memory_temporal_receipts.sql` adds an owner-only U33
database operation for recording a memory-use receipt with the U23 temporal
commitment and the triggering event reference. It serializes on the exact
scoped memory head, verifies that the immutable artifact is the current live
`memory_wiki`, checks its availability against both use time and replay cutoff,
and inserts the receipt in the same PostgreSQL statement. While the selected
head remains current and live, repeating the exact event/run/scope request
returns successfully without a second receipt; reusing the tenant-scoped event
identity for another run or scope is rejected. A stale or revoked retry fails
closed before idempotency lookup; the stored receipt never bypasses current
authorization fences.
Revocation, head drift, wrong scope, and temporal ineligibility fail closed.

The gated integration test applies the real U02/U33/P4 SQL migrations and
asserts idempotency, cross-run event reuse rejection, scope mismatch, cutoff
rejection, revocation rejection, and no receipt side effects on failed calls.
It follows the existing opt-in destructive-test contract and requires an
isolated PostgreSQL database.

## Explicit limits / next unit

This is the durable U33 persistence seam, not yet a Rust `PostgresMemoryPublisher`
or public event-to-run composition. Function execution is revoked from
`PUBLIC`; a trusted service adapter still needs to call it only after U22/U23
admission and U05 grant validation. The existing U06 event-to-successor-run
transaction is also not connected here. Until those callers exist, this
migration does not claim that a new platform event triggers autonomous memory
recovery or a new run. Frozen remains a protocol value validated by U23; this
SQL operation stores the supplied opaque commitment and never derives learning
from outcome data.

## Review / validation

The PostgreSQL integration test was authored first against the missing
function. Its RED run was not observed because the shared Cargo target lock was
occupied at that time; after adding the migration the isolated target compiled
and the test passed against PostgreSQL 18, including two concurrent exact
retries, cross-scope event contention, null temporal-fence rejection, and a
revoke-vs-use lock-order regression. Command:

```text
cargo test -p improvement-engine-core --target-dir .target-p4-memory --test postgres_memory_temporal_receipts -- --ignored --exact temporal_use_receipt_is_bound_to_event_scope_cutoff_and_current_head --nocapture
```

Result: `1 passed; 0 failed; 0 ignored` (11.54s), with
`PULSO_TEST_POSTGRES_URL` targeting the isolated local PostgreSQL 18 cluster
and `PULSO_ALLOW_DESTRUCTIVE_TEST_DB=1`. The test remains ignored by default
and requires the explicit destructive-test flag plus an isolated database.
The CI job uses pinned PostgreSQL 17; that compatibility run is pending CI.

The independent adversarial review's initial NO-GO findings were addressed:
explicit NULL fences, a real revoke-vs-use lock-wait regression, CI wiring, and
cross-scope concurrent event-identity conflict rejection. A final P2 review
also required proof that PUBLIC cannot execute the `SECURITY DEFINER` writer;
the test now inspects the function ACL for the exact signature and fails if
PUBLIC has EXECUTE. Final local PostgreSQL 18 run after that addition:
`1 passed; 0 failed; 0 ignored` (15.76s). Independent re-review: GO, with
PostgreSQL 17 CI compatibility still pending. No Rust composition/U05 wiring
was claimed or added.

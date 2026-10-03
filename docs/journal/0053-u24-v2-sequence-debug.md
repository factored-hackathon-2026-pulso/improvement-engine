# 0053 — U24 sequence-native debug read

## Scope / acceptance

Implements the U24 V2 read slice over the already-defined `pulso_run_events`
table: tenant comes only from the same authenticated viewer capability used by
the existing U24 composer; request has `run_ref`, `after_sequence`, and
`page_size`; ordering and continuation use durable sequence; output is a
closed safe projection. No migration, writer, mutation, arbitrary SQL, artifact
detail, payload, reason code, trace ID, HTTP route, or UI is added. `RunActivity`
legacy cursor remains untouched.

## RED → GREEN

- RED: `cargo +1.98.1 test -p improvement-engine-core --test
  postgres_run_timeline_v2 --no-run` failed with E0432 because the planned
  `run_timeline_v2` contract/adapter did not exist.
- GREEN: added a sequence-native request, PG reader, closed stage/event/status
  allowlist (`other` for unknown stored text), and authenticated read-only
  composer using `DebugIdentityPort` / `DebugViewerIssuer`. Missing and
  cross-tenant run refs map to the same `NotFound`; each page verifies the root
  by tenant and run before fetching events.
- RED/GREEN contract cases: page sizes 0 and 101, invalid UUID and negative
  sequence rejected; valid page size 100 accepted. Composer derives tenant
  from identity; denied identity never reaches the read port; storage errors
  become generic `Unavailable`.
- RED/GREEN destructive-test guard: the helper initially failed to compile
  because the database-name guard was absent; it now accepts only exact
  `pulso_test`, and rejects `production` / `pulso_test_copy` before any schema
  or truncate statement.
- PostgreSQL test uses the real schema/migration and covers sequence ASC
  continuation, tenant/run separation, and omission/coarsening of arbitrary
  `reason_code`, artifact/trace/detail refs, and unapproved stage/event/status
  text. It is ignored unless both isolated PG URL and explicit destructive-test
  consent are supplied. It checks `SELECT current_database()` and refuses to
  proceed unless the database is exactly `pulso_test`. No persistence mock is
  used as substitute evidence.

## Decisions / constraints

- Kept a V2 contract separate from U07's `(job_id, event_time, event_id)`
  cursor; `after_sequence` is a position and not authorization.
- Only internal event/run/job refs, sequence, event time and enumerated
  debugging categories cross the read projection. The page identifies closed
  vocabulary version 1; unknown stored strings map to `other`; `reason_code`
  stays omitted because current storage validation is only a regex, not a
  closed domain enum.
- No schema changes or source-table writes.
- Local native PostgreSQL was attempted in a disposable Windows environment;
  PostgreSQL initialized but could not bind a port (WSA bind failure, including
  elevated retry). The actual PostgreSQL test therefore remains an explicit
  CI-only gate; the test is not reported as passed locally.

## Verification

Commands run on Windows / PowerShell (the request and composer tests were
initially in an external integration target; following security review, the
reader and request types were restricted to `pub(crate)` and the PG test was
moved into the crate's `#[cfg(test)]` module):

```powershell
cargo +1.98.1 test -p improvement-engine-core --test postgres_run_timeline_v2 --no-run
cargo +1.98.1 test -p improvement-engine-core --lib
cargo +1.98.1 test -p improvement-engine-core --lib run_timeline_v2::tests::destructive_fixture_refuses_any_database_other_than_pulso_test
cargo +1.98.1 fmt --all
cargo +1.98.1 fmt --all -- --check
cargo +1.98.1 clippy -p improvement-engine-core --all-targets -- -D warnings
git diff --check
```

Observed results so far: RED failed only because `run_timeline_v2` was absent;
after restricting the reader to `pub(crate)`, the full core library suite
passed after the DB-name guard (97 passed, 0 failed, 1 ignored PostgreSQL test); this includes both
composer regressions and request-bound/allowlist tests. Formatting check,
Clippy with `-D warnings`, and `git diff --check` passed on the current code.
The focused DB-name test also passed. Independent read-only adversarial review
returned GO. Real PostgreSQL execution remains pending CI.

CI executes the ignored PG test with this exact command after the existing
U07 test, against the workflow's ephemeral PostgreSQL service:

```powershell
cargo +1.98.1 test --locked -p improvement-engine-core --lib run_timeline_v2::tests::authenticated_scope_sequence_pagination_and_sanitization_use_real_postgres -- --ignored
```

The reader is now `pub(crate)`: constructing `AuthenticatedTenant` is not
authentication. Any future cross-crate facade must take an auth-issued
viewer/context, not a tenant string. The DB test remains real PostgreSQL (not a
mock), ignored locally pending CI's ephemeral service; this Windows host's
disposable PostgreSQL could not bind a socket.

### CI follow-up — PostgreSQL fixture UUID parameter typing

The first PR CI run compiled and reached the real PostgreSQL integration test,
then failed at fixture setup with `ToSql(0) ... WrongType { postgres: Uuid,
rust: "&str" }`. The test SQL cast UUID bind parameters directly as `$n::uuid`,
which asks the Rust PostgreSQL client to encode a Rust string as PostgreSQL's
UUID wire type. The fixture uses string UUID constants; changed those casts to
`$n::text::uuid`, matching the existing storage adapter convention and making
the text bind type explicit before PostgreSQL parses the UUID. Applies to root
rows and event fixture inserts only; production read queries already use the
text-to-UUID cast. The integration regression itself is the failing case and
will be rerun by the PR's ephemeral-PostgreSQL CI gate. Local reproduction is
not available because PostgreSQL cannot bind on this Windows host; no mock was
used.

After the correction, local verification passed: `cargo +1.98.1 test --locked
-p improvement-engine-core --lib run_timeline_v2::tests` (3 passed, 1 ignored),
the full core library suite (97 passed, 1 ignored),
`cargo +1.98.1 fmt --all -- --check`, and
`cargo +1.98.1 clippy --locked -p improvement-engine-core --all-targets --
-D warnings`; `git diff --check` passed. The ignored live PostgreSQL test is
not represented as locally passing; the corrected real-database fixture is
queued for the new PR CI run.

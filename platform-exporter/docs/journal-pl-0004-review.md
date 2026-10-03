# Journal PL-0004: independent adversarial review of platform-exporter (a4bb257, 68df107, 262bdc8, 125e0ec)

Real runs: SQLite and Postgres 16-alpine (Podman `pulso-dev`, `--cgroups=disabled`, 127.0.0.1, throwaway container,
removed afterwards). Baseline 39 passed (PG included); after the review 51 passed.

## Defects found and fixed (RED first, tests in tests/test_review_pl0004.py)
1. HIGH PostgresSource was not read-only: `conn.read_only = True` on an autocommit psycopg connection is not applied
   (`SHOW default_transaction_read_only` = off, CREATE TABLE succeeded with a privileged role). The old PG test was vacuous
   (the role had no INSERT grant anyway). Fix: `options=-c default_transaction_read_only=on` at connect.
2. HIGH payload redaction was an exact-key deny-list: `customer_email`, `fullName`, `phoneNumber`, `message_preview`,
   `subject`, `comment`, `username`, `snippet`, `address` and e-mails inside values were forwarded. Fix: key tokens
   (snake/camel/kebab) + e-mail value masking (bounded regex; the first draft was ReDoS-prone on a 700 KB value, found
   by the oversized-event test).
3. MEDIUM a non-object JSON payload (e.g. a bare string) was forwarded untouched. Now quarantined as `payload_not_object`.
4. MEDIUM one oversized event (> batch_max_bytes alone) stopped the partition forever (`batch_too_large`). Now that row
   is quarantined as `oversized_event` (bad_row finding, payload dropped) and the stream continues.

## Mutation check (26 mutants, 8 initial survivors became tests, 2 equivalents)
Killed: as-of ingested_at/event_time visibility, reopen reset, sort; nested redaction, e-mail value mask, denied-type
path, gap hold-back, gap range width, start_sequence, lateness config, late boundary, digest-mismatch ack, cursor commit,
idempotency key, persisted pending (needed a stronger crash test: rebuilt batch was deterministic so it hid the
mutant), simulator marking, authorizer column/write, cases.status allow-list, PG read-only, holes_cleared, oversized,
non-object. Equivalent: dropping the `email` key (now covered by tokens), disabling the authorizer table check
(the column check still denies). Not meaningful: remote-cursor persistence (cursor re-read on every build).

## Findings not fixed (design, report)
- Local state loss forgets open backfill requests (`hole_ranges skipped=1`): the Pulso cursor wins, but holes that were
  skipped before the loss are never re-requested, so a row that commits late below the cursor is lost until a manual
  `backfill(lo, hi)`. Needs a Pulso-side record of open gaps (or a rebuild-time full re-scan as late) to fix.
- A permanently missing sequence (PG rollback) keeps a backfill request open forever and is re-queried every poll.
- `start_sequence=0` cannot export a row with sequence 0 (`sequence > after`); contract says minimum 1, so ignored.

## `exporter.*` as platform_event (spec 24.2)
The exporter wraps its own meta-observations (`exporter.finding`, `exporter.capability_profile`,
`exporter.dimension_snapshot`) as `kind=platform_event`, `source_namespace=platform_live`. Collisions with the 24.2 DTO:
- `source_event` is not the DTO `PlatformEvent{event_id, source_contract_ref, source_sequence?, occurred_at, received_at,
  entity_refs, actor_ref, event_type, payload_ref, digest}`: it carries event_time/ingested_at/available_at, entity +
  entity_id + case_id, inline treated payload and no digest/payload_ref; the digest is the envelope `source_event_digest`.
  Findings have no `event_id` at all (identity is the envelope `native_event_id = finding:...`).
- Findings reuse the `source_sequence` of the row they describe (a late event appears twice at the same sequence; a
  quarantined row appears only as a finding), and gap/turn findings have `source_sequence=null`.
- `observed_at` of findings is wall clock, not event time.
What Codex's adapter must do: branch on `source_event.event_type` starting with `exporter.` BEFORE DTO validation; map
those to the quality/coverage/profile channel (never InteractionObservation, never population or window projections, never
`source_sequence` contiguity or dedup by sequence); keep dedup on `(tenant, source_id, native_event_id)`; for domain rows
map `event_time -> occurred_at`, `ingested_at/available_at -> received_at`, `entity+entity_id -> entity_refs`,
`actor_ref`, and store the inline treated payload as the artifact behind `payload_ref`. An alternative is a distinct
discriminator or catalog entry for exporter meta events; this needs a contract decision (owner: platform-contract).

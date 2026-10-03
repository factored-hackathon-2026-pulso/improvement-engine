# platform-contract

Contract pack for the `platform_live` source profile (TECH_SPEC_PULSO_AUTOMEJORA_V3 section 32,
package PL-L3): JSON Schemas for the allow-listed tables of the real support platform, the event
type catalog, golden examples and a conformance suite with drift checks. Codex's PL-C1 adapter
and the exporter (PL-L1) test against these files.

## Version stamp

| Field | Value |
|---|---|
| Contract version | 1.1.0 (profile `platform_live.phase1`); additive over 1.0.0 |
| Source artifact | Product team, "Modelo de datos · Plataforma CC", id `BWx4saeWfYsLbQEbkNKMPg` |
| Platform commit | `a492bfa` (slices 0-3) |
| Captured | 2026-10-03 |

The artifact is third-party data. Anything it does not state (payload shapes per event type,
sequence contiguity after rollbacks, meaning of `ingested_at`) is an open question for Product
(see PLATFORM_DATA_MODEL_IMPACT_CLAUDE.md section 7) and is NOT invented here: `event_log.payload`
is only constrained to be a JSON object.

## Layout

- `platform_contract/model.py`: single source of truth (tables, enums, event catalog, denylist).
- `schemas/*.schema.json`, `event-catalog.json`: generated from the model
  (`python -m platform_contract` regenerates; a drift test fails if files differ).
- `examples/*.valid.json`, `examples/invalid/*.json`: golden vectors.
- `platform_contract/conformance.py`: row validation, `check_event_stream` (gap_suspected,
  unknown/planned event type, late event), `check_turn_sequences`, `check_database` (SQLite).
- `tests/`: contract tests (run from repo root, see below).

## Allow-list and denylist (spec 32.2.2)

Allowed: `cases`, `turns`, `assignments`, `customer_case_slots`, `customers`, `event_log`,
`staff_dimensions` (projection of `staff`: id, roles, languages, team/team_id, active, version).

Never read: tables `login_accounts`, `mfa_challenges`, `staff_sessions` (no schema exists; loading
raises `TableRefused` before any query); columns `staff.name|email`, `customers.display_name|suggestions`,
`cases.search_text|last_message_preview|last_turn_preview`. Any table not allow-listed (including the
announced `teams`, `admin_roster` and `analyst_availability`) is refused by default. Text columns
(`turns.text`, `cases.close_note`) carry `x-sensitive-text` and need the pre-ModelPort treatment.

## Event catalog

Admitted: `case.*` (opened, queued, assigned, status_changed, read, first_responded, closed, viewed),
`turn.created`, `staff.availability_changed`, and `auth.login_failed|account_locked|session_started|session_ended`.
Denied (known, never ingested): `auth.password_accepted`, `auth.mfa_challenge_issued`, `auth.mfa_failed`,
`customer.session_started`. Planned (announced prefixes `staff.`, `team.`) and unknown types are counted and
quarantined with a quality finding; they never fail the batch.

## Revision 1.1.0: exporter metadata discriminator

`source_event.kind` separates exporter metadata from domain rows:

- `exporter_finding`: `schemas/exporter_finding.schema.json`. Body: `kind`, `source_namespace`, `catalog_version`,
  `tenant_id`, `finding_code` (closed list), `severity` (info|warning|error), `described_native_event_id` and
  `described_source_sequence` (nullable), `details` (inline, <= 32 KiB serialized, <= 64 keys). Observation envelope
  (outside the digest, so re-emission stays idempotent): `source_id`, `native_event_id` (identity, must be
  `finding:|profile:|dimensions:` prefixed), `observed_at` (wall clock), `source_sequence` (null, or equal to
  `described_source_sequence`: a finding about a late row reuses that row's sequence). Never a domain row, never part
  of source-sequence continuity or population/window projections. Dedup key `(tenant_id, source_id, native_event_id)`.
- `domain_event`: `schemas/domain_event.schema.json`; `event_type` must not start with `exporter.`.
- `schemas/source_observation.schema.json`: self-contained observation view (identity + nullable sequence +
  discriminated `source_event`). Wire `kind` stays `platform_event` (pulso-observations-2 is unchanged).
- Golden fixtures: `examples/source_events/valid/*.json` (normal finding, null sequence, late-event sequence reuse,
  dedup, domain row) and `examples/source_events/invalid/*.json` (unsupported code/kind, missing field, extra field,
  prefix misuse, identity/tenant mismatch, legacy shape, domain without sequence).
- Interim rule: for contract 1.0.0 the `exporter.` event_type prefix identifies exporter metadata. It remains valid
  for 1.0.0 and for exporters run with `legacy_prefix=True`; it is rejected by the 1.1.0 schemas.

## Run

From the repository root:

    uv run --python 3.12 --no-project --with pytest --with jsonschema python -m pytest platform-contract/tests
    uvx ruff --isolated check --select E4,E7,E9,F platform-contract

# platform-contract

Contract pack for the `platform_live` source profile (TECH_SPEC_PULSO_AUTOMEJORA_V3 section 32,
package PL-L3): JSON Schemas for the allow-listed tables of the real support platform, the event
type catalog, golden examples and a conformance suite with drift checks. Codex's PL-C1 adapter
and the exporter (PL-L1) test against these files.

## Version stamp

| Field | Value |
|---|---|
| Contract version | 1.0.0 (profile `platform_live.phase1`) |
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

## Run

From the repository root:

    uv run --python 3.12 --no-project --with pytest --with jsonschema python -m pytest platform-contract/tests
    uvx ruff --isolated check --select E4,E7,E9,F platform-contract
